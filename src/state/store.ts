/**
 * 编辑器状态。
 *
 * 一条原则：**图的事实来自后端**。节点上显示的端口类型、连线合不合法、红线画在
 * 哪，全都来自 `resolve_workflow` 的返回，前端不自己推算类型（唯一的例外是拖动
 * 连线时的即时反馈，见 `lib/ports.ts` 的说明）。
 */

import {
  applyEdgeChanges,
  applyNodeChanges,
  type Connection,
  type EdgeChange,
  type NodeChange,
} from '@xyflow/react';
import { create } from 'zustand';

import { api, messageOf, pickDestination, pickFile } from '../lib/api';
import type { Point } from '../lib/geometry';
import {
  edgeChangeEffect,
  edgesFrom,
  emptyMeta,
  metaOf,
  nodeChangeEffect,
  nodesFrom,
  resolvedNode,
  toWorkflow,
  uid,
  type WorkflowMeta,
} from '../lib/graph';
import { accepts } from '../lib/ports';
import type {
  AppInfo,
  ImageInfo,
  NodeKind,
  PortType,
  ResolvedWorkflow,
  RunReport,
  ToolNode,
  WireEdge,
  Workflow,
  WorkflowSummary,
} from '../lib/types';

export type Status = { text: string; kind: 'info' | 'error' };

type Pulse = { order: string[]; step: number };

/** 刀光。坐标是屏幕坐标 —— 它画在画布之上的一个覆盖层里。 */
export type Slash = {
  from: Point;
  to: Point;
  /** 这一刀会切到的连线，拖动过程中实时更新 */
  doomed: string[];
  /** 收刀跟出的进度，0 → 1 */
  overtake: number;
};

/** 一条正在断开的连线。`progress` 0 → 1，跑完才真的从图里删掉。 */
export type Severing = {
  edgeId: string;
  /** 断口（flow 坐标），用来在切口处放一下光 */
  point: Point;
  /** 从连线起点量到断口的距离 */
  distance: number;
  /** 连线总长 */
  length: number;
  progress: number;
};

type StoreState = {
  ready: boolean;
  kinds: NodeKind[];
  kindById: Record<string, NodeKind>;
  appInfo: AppInfo | null;

  meta: WorkflowMeta;
  nodes: ToolNode[];
  edges: WireEdge[];
  resolved: ResolvedWorkflow | null;
  dirty: boolean;
  selectedId: string | null;
  /** 刚落到画布上的节点，用它放一段入场动画。 */
  justAdded: string | null;

  report: RunReport | null;
  running: boolean;
  previews: Record<string, string>;
  fileInfos: Record<string, ImageInfo>;
  summaries: WorkflowSummary[];

  pulse: Pulse | null;
  status: Status | null;
  slash: Slash | null;
  severed: Severing[];

  init: () => Promise<void>;
  refreshList: () => Promise<void>;

  addNode: (kindId: string, position: { x: number; y: number }) => void;
  removeNode: (nodeId: string) => void;
  /** 节点已经被 React Flow 删掉了，这里只负责清掉附属状态。 */
  forgetNodes: (nodeIds: string[]) => void;
  updateParam: (nodeId: string, paramId: string, value: unknown) => void;
  chooseFile: (nodeId: string) => Promise<void>;

  onNodesChange: (changes: NodeChange<ToolNode>[]) => void;
  onEdgesChange: (changes: EdgeChange<WireEdge>[]) => void;
  onConnect: (connection: Connection) => void;
  canConnect: (connection: Connection | WireEdge) => boolean;
  select: (nodeId: string | null) => void;
  setMeta: (patch: Partial<WorkflowMeta>) => void;

  resolveNow: () => Promise<void>;
  run: (onlyNode?: string) => Promise<void>;
  save: () => Promise<void>;
  newWorkflow: () => Promise<void>;
  openWorkflow: (id: string) => Promise<void>;
  removeWorkflow: (id: string) => Promise<void>;

  clearReport: () => void;
  exportOutput: (source: string, defaultName: string) => Promise<void>;
  reveal: (path: string) => Promise<void>;
  notify: (text: string, kind?: 'info' | 'error') => void;

  beginSlash: (x: number, y: number) => void;
  updateSlash: (x: number, y: number, doomed: string[]) => void;
  /** 收刀：被切中的连线开始断裂动画，刀光跟出后消散。 */
  cutEdges: (cuts: Omit<Severing, 'progress'>[]) => void;
  abandonSlash: () => void;
};

let resolveTimer: ReturnType<typeof setTimeout> | undefined;
let pulseTimer: ReturnType<typeof setInterval> | undefined;
let addFlashTimer: ReturnType<typeof setTimeout> | undefined;
let slashTimer: number | undefined;
let severTimer: number | undefined;

const SEVER_DURATION = 460;

/**
 * 断裂动画：两截各自往两边缩回去，断口先闪一下。
 *
 * 用 rAF 推而不是 CSS keyframes，是因为「断在路径上的哪个位置」每条线都不一样，
 * keyframes 写不了这种每条边都不同的数值。
 * 动画跑完才真的把连线从图里拿掉 —— 提前删了就看不见断裂了。
 */
function runSeverAnimation(
  apply: (update: (state: StoreState) => Partial<StoreState>) => void,
  read: () => StoreState,
) {
  if (severTimer) cancelAnimationFrame(severTimer);

  const ids = read().severed.map((entry) => entry.edgeId);
  const started = performance.now();

  const tick = (now: number) => {
    const progress = Math.min(1, (now - started) / SEVER_DURATION);
    apply((state) => ({
      severed: state.severed.map((entry) => ({ ...entry, progress })),
    }));

    if (progress < 1) {
      severTimer = requestAnimationFrame(tick);
      return;
    }

    severTimer = undefined;
    apply((state) => ({
      edges: state.edges.filter((edge) => !ids.includes(edge.id)),
      severed: [],
      dirty: true,
    }));
    void read().resolveNow();
  };

  severTimer = requestAnimationFrame(tick);
}
/** 防止慢的检查结果盖掉新的。 */
let resolveToken = 0;

export const useStore = create<StoreState>((set, get) => {
  function scheduleResolve() {
    if (resolveTimer) clearTimeout(resolveTimer);
    resolveTimer = setTimeout(() => {
      void get().resolveNow();
    }, 120);
  }

  /** 只标一下「有未保存的改动」。同一趟里已经脏了就别再通知一遍。 */
  function touch() {
    if (!get().dirty) set({ dirty: true });
  }

  function markDirty() {
    touch();
    scheduleResolve();
  }

  function playPulse(order: string[]) {
    if (pulseTimer) clearInterval(pulseTimer);
    if (order.length === 0) {
      set({ pulse: null });
      return;
    }
    let step = 0;
    set({ pulse: { order, step } });
    pulseTimer = setInterval(() => {
      step += 1;
      if (step > order.length) {
        clearInterval(pulseTimer);
        pulseTimer = undefined;
        set({ pulse: null });
        return;
      }
      set({ pulse: { order, step } });
    }, 210);
  }

  function adoptWorkflow(workflow: Workflow, statusText: string) {
    set({
      meta: metaOf(workflow),
      nodes: nodesFrom(workflow),
      edges: edgesFrom(workflow),
      report: null,
      previews: {},
      fileInfos: {},
      dirty: false,
      selectedId: null,
      pulse: null,
      status: { text: statusText, kind: 'info' },
    });
    void hydrateFilePreviews();
  }

  /** 打开工作流后把输入节点的缩略图先读出来，省得看着一片空。 */
  async function hydrateFilePreviews() {
    const { nodes, kindById } = get();
    const targets = nodes.flatMap((node) => {
      const fileParam = kindById[node.data.kind]?.params.find(
        (param) => param.control === 'file',
      );
      // 目录参数没有缩略图可读，跳过。
      if (!fileParam || fileParam.directory) return [];
      const path = node.data.params[fileParam.id];
      return typeof path === 'string' && path ? [{ nodeId: node.id, path }] : [];
    });

    await Promise.all(
      targets.map(async ({ nodeId, path }) => {
        try {
          const info = await api.inspectImage(path);
          set((state) => ({
            fileInfos: { ...state.fileInfos, [nodeId]: info },
            previews: info.preview
              ? { ...state.previews, [nodeId]: info.preview }
              : state.previews,
          }));
        } catch {
          // 文件不在了就先不管，真跑的时候会报出来。
        }
      }),
    );
  }

  return {
    ready: false,
    kinds: [],
    kindById: {},
    appInfo: null,

    meta: emptyMeta(),
    nodes: [],
    edges: [],
    resolved: null,
    dirty: false,
    selectedId: null,
    justAdded: null,

    report: null,
    running: false,
    previews: {},
    fileInfos: {},
    summaries: [],

    pulse: null,
    status: null,
    slash: null,
    severed: [],

    async init() {
      try {
        const [kinds, appInfo] = await Promise.all([api.nodeKinds(), api.appInfo()]);
        set({
          kinds,
          kindById: Object.fromEntries(kinds.map((kind) => [kind.id, kind])),
          appInfo,
          ready: true,
        });
        await get().refreshList();
        const [first] = get().summaries;
        if (first) await get().openWorkflow(first.id);
        else await get().newWorkflow();
      } catch (error) {
        set({ ready: true, status: { text: messageOf(error), kind: 'error' } });
      }
    },

    async refreshList() {
      try {
        set({ summaries: await api.listWorkflows() });
      } catch (error) {
        set({ status: { text: messageOf(error), kind: 'error' } });
      }
    },

    addNode(kindId, position) {
      const kind = get().kindById[kindId];
      if (!kind) return;
      const node: ToolNode = {
        id: uid('n'),
        type: 'tool',
        position,
        data: { kind: kindId, params: { ...kind.defaults } },
      };
      set((state) => ({
        nodes: [...state.nodes, node],
        selectedId: node.id,
        // 卡片「变成画布节点」的那段动画靠这个标记。
        justAdded: node.id,
        status: { text: `已添加「${kind.name}」`, kind: 'info' },
      }));
      markDirty();

      if (addFlashTimer) clearTimeout(addFlashTimer);
      addFlashTimer = setTimeout(() => set({ justAdded: null }), 460);
    },

    removeNode(nodeId) {
      set((state) => ({
        nodes: state.nodes.filter((node) => node.id !== nodeId),
        edges: state.edges.filter((edge) => edge.source !== nodeId && edge.target !== nodeId),
        selectedId: state.selectedId === nodeId ? null : state.selectedId,
      }));
      get().forgetNodes([nodeId]);
    },

    forgetNodes(nodeIds) {
      set((state) => {
        const previews = { ...state.previews };
        const fileInfos = { ...state.fileInfos };
        for (const id of nodeIds) {
          delete previews[id];
          delete fileInfos[id];
        }
        return {
          previews,
          fileInfos,
          selectedId:
            state.selectedId && nodeIds.includes(state.selectedId) ? null : state.selectedId,
        };
      });
      markDirty();
    },

    updateParam(nodeId, paramId, value) {
      set((state) => ({
        nodes: state.nodes.map((node) =>
          node.id === nodeId
            ? { ...node, data: { ...node.data, params: { ...node.data.params, [paramId]: value } } }
            : node,
        ),
      }));
      markDirty();
    },

    async chooseFile(nodeId) {
      const node = get().nodes.find((candidate) => candidate.id === nodeId);
      const kind = node ? get().kindById[node.data.kind] : undefined;
      const fileParam = kind?.params.find((param) => param.control === 'file');
      if (!fileParam) return;

      try {
        const picked = await pickFile(
          fileParam.extensions ?? [],
          fileParam.dialogTitle ?? '选择文件',
          fileParam.directory === true,
        );
        if (!picked) return;
        get().updateParam(nodeId, fileParam.id, picked);

        // 选的是目录：没有图像可看，记一笔就够了。
        if (fileParam.directory) {
          set({ status: { text: `已选择目录 ${picked}`, kind: 'info' } });
          return;
        }

        const info = await api.inspectImage(picked);
        set((state) => ({
          fileInfos: { ...state.fileInfos, [nodeId]: info },
          previews: info.preview
            ? { ...state.previews, [nodeId]: info.preview }
            : state.previews,
          status: {
            text: `${info.fileName} · ${info.width} × ${info.height} · ${info.format.toUpperCase()}`,
            kind: 'info',
          },
        }));
      } catch (error) {
        set((state) => {
          const previews = { ...state.previews };
          const fileInfos = { ...state.fileInfos };
          delete previews[nodeId];
          delete fileInfos[nodeId];
          return { previews, fileInfos, status: { text: messageOf(error), kind: 'error' } };
        });
      }
    },

    onNodesChange(changes) {
      set((state) => ({ nodes: applyNodeChanges(changes, state.nodes) }));
      // 拖动只是挪位置，不影响类型检查 —— 只标脏，不请后端重解析。
      switch (nodeChangeEffect(changes)) {
        case 'recheck':
          markDirty();
          break;
        case 'touch':
          touch();
          break;
        default:
          break;
      }
    },

    onEdgesChange(changes) {
      set((state) => ({ edges: applyEdgeChanges(changes, state.edges) }));
      if (edgeChangeEffect(changes) === 'recheck') markDirty();
    },

    onConnect(connection) {
      const { source, sourceHandle, target, targetHandle } = connection;
      if (!source || !target || !sourceHandle || !targetHandle) return;
      if (source === target) return;

      set((state) => ({
        // 一个输入端口只接一条线，接新的就把旧的挤掉。
        edges: [
          ...state.edges.filter(
            (edge) => !(edge.target === target && edge.targetHandle === targetHandle),
          ),
          {
            id: uid('e'),
            type: 'wire',
            source,
            sourceHandle,
            target,
            targetHandle,
          },
        ],
      }));
      markDirty();
    },

    canConnect(connection) {
      const { source, sourceHandle, target, targetHandle } = connection;
      if (!source || !target || !sourceHandle || !targetHandle) return false;
      if (source === target) return false;

      const resolved = get().resolved;
      const sourceNode = resolvedNode(resolved, source);
      const targetNode = resolvedNode(resolved, target);
      const from: PortType | undefined = sourceNode?.outputs.find(
        (port) => port.id === sourceHandle,
      )?.ty;
      const to: PortType | undefined = targetNode?.inputs.find(
        (port) => port.id === targetHandle,
      )?.ty;
      if (!from || !to) return false;
      return accepts(to, from);
    },

    select(nodeId) {
      set({ selectedId: nodeId });
    },

    setMeta(patch) {
      set((state) => ({ meta: { ...state.meta, ...patch } }));
      markDirty();
    },

    async resolveNow() {
      const token = ++resolveToken;
      const { meta, nodes, edges } = get();
      try {
        const resolved = await api.resolveWorkflow(toWorkflow(meta, nodes, edges));
        if (token === resolveToken) set({ resolved });
      } catch (error) {
        if (token === resolveToken) {
          set({ status: { text: messageOf(error), kind: 'error' } });
        }
      }
    },

    async run(onlyNode) {
      if (get().running) return;
      set({
        running: true,
        status: { text: onlyNode ? '正在运行至此…' : '正在运行工作流…', kind: 'info' },
      });
      try {
        const { meta, nodes, edges } = get();
        const report = await api.runWorkflow(toWorkflow(meta, nodes, edges), onlyNode);

        const previews = { ...get().previews };
        for (const node of report.nodes) {
          const shown = node.outputs.find((output) => output.preview)?.preview;
          // 这个节点这次没产出图，就把可能残留的旧缩略图撤掉。
          if (shown) previews[node.nodeId] = shown;
          else delete previews[node.nodeId];
        }

        const failed = report.nodes.filter((node) => node.status === 'failed');
        const status: Status = report.ok
          ? {
              text: `运行完成：${report.nodes.length} 个节点，用时 ${report.durationMs} ms`,
              kind: 'info',
            }
          : failed.length > 0
            ? { text: `${failed[0]?.name}：${failed[0]?.error ?? '执行失败'}`, kind: 'error' }
            : { text: `工作流还有 ${report.issues.length} 个问题未处理`, kind: 'error' };

        set({ report, previews, running: false, status });

        if (report.nodes.length === 0) await get().resolveNow();
        else playPulse(report.order);
      } catch (error) {
        set({ running: false, status: { text: messageOf(error), kind: 'error' } });
      }
    },

    async save() {
      try {
        const { meta, nodes, edges } = get();
        const saved = await api.saveWorkflow(toWorkflow(meta, nodes, edges));
        set({
          meta: metaOf(saved),
          dirty: false,
          status: { text: `已保存「${saved.name}」`, kind: 'info' },
        });
        await get().refreshList();
      } catch (error) {
        set({ status: { text: messageOf(error), kind: 'error' } });
      }
    },

    async newWorkflow() {
      try {
        adoptWorkflow(await api.createWorkflow(), '已新建工作流');
        await get().resolveNow();
      } catch (error) {
        set({ status: { text: messageOf(error), kind: 'error' } });
      }
    },

    async openWorkflow(id) {
      try {
        const workflow = await api.loadWorkflow(id);
        adoptWorkflow(workflow, `打开了「${workflow.name}」`);
        await get().resolveNow();
      } catch (error) {
        set({ status: { text: messageOf(error), kind: 'error' } });
      }
    },

    async removeWorkflow(id) {
      try {
        await api.deleteWorkflow(id);
        const { meta } = get();
        await get().refreshList();
        if (meta.id === id) await get().newWorkflow();
        set({ status: { text: '已删除', kind: 'info' } });
      } catch (error) {
        set({ status: { text: messageOf(error), kind: 'error' } });
      }
    },

    clearReport() {
      set({ report: null, pulse: null });
    },

    async exportOutput(source, defaultName) {
      try {
        const destination = await pickDestination(defaultName);
        if (!destination) return;
        await api.exportFile(source, destination);
        set({ status: { text: `已另存到 ${destination}`, kind: 'info' } });
      } catch (error) {
        set({ status: { text: messageOf(error), kind: 'error' } });
      }
    },

    async reveal(path) {
      try {
        // await 确实在这里：不然下面这个 catch 捕不到 Promise 的拒绝。
        await api.revealPath(path);
      } catch (error) {
        set({ status: { text: messageOf(error), kind: 'error' } });
      }
    },

    beginSlash(x, y) {
      if (slashTimer) cancelAnimationFrame(slashTimer);
      slashTimer = undefined;
      set({ slash: { from: { x, y }, to: { x, y }, doomed: [], overtake: 0 } });
    },

    updateSlash(x, y, doomed) {
      const slash = get().slash;
      if (!slash) return;
      set({ slash: { ...slash, to: { x, y }, doomed } });
    },

    cutEdges(cuts) {
      const slash = get().slash;
      if (!slash) return;

      if (cuts.length > 0) {
        set({ severed: cuts.map((cut) => ({ ...cut, progress: 0 })) });
        runSeverAnimation(set, get);
        set({
          status: {
            text: `断开了 ${cuts.length} 条连线`,
            kind: 'info',
          },
        });
      }

      // 收刀：刀光向前跟出一段再消散，和断裂动画同时跑。
      const started = performance.now();
      const duration = 300;
      const tickOvertake = (now: number) => {
        const t = Math.min(1, (now - started) / duration);
        const current = get().slash;
        if (!current) return;
        if (t >= 1) {
          slashTimer = undefined;
          set({ slash: null });
          return;
        }
        set({ slash: { ...current, overtake: t } });
        slashTimer = requestAnimationFrame(tickOvertake);
      };
      slashTimer = requestAnimationFrame(tickOvertake);
    },

    abandonSlash() {
      if (slashTimer) cancelAnimationFrame(slashTimer);
      slashTimer = undefined;
      set({ slash: null });
    },

    notify(text, kind = 'info') {
      set({ status: { text, kind } });
    },
  };
});
