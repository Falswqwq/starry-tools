/** 编辑器里的图 ⇄ 落盘的工作流，以及各种从 `resolved` 里查东西的小工具。 */

import type {
  Issue,
  ResolvedNode,
  ResolvedWorkflow,
  Workflow,
  WorkflowEdge,
} from './types';
import type { ToolNode, WireEdge } from './types';

export type WorkflowMeta = {
  id: string;
  name: string;
  description: string;
  createdAt: number | null;
  updatedAt: number | null;
};

export function metaOf(workflow: Workflow): WorkflowMeta {
  return {
    id: workflow.id,
    name: workflow.name,
    description: workflow.description,
    createdAt: workflow.createdAt ?? null,
    updatedAt: workflow.updatedAt ?? null,
  };
}

export function emptyMeta(): WorkflowMeta {
  return { id: '', name: '未命名工作流', description: '', createdAt: null, updatedAt: null };
}

export function nodesFrom(workflow: Workflow): ToolNode[] {
  return workflow.nodes.map((node) => ({
    id: node.id,
    type: 'tool',
    position: node.position,
    data: { kind: node.kind, params: { ...node.params } },
  }));
}

export function edgesFrom(workflow: Workflow): WireEdge[] {
  return workflow.edges.map((edge) => ({
    id: edge.id,
    type: 'wire',
    source: edge.source,
    sourceHandle: edge.sourcePort,
    target: edge.target,
    targetHandle: edge.targetPort,
  }));
}

export function toWorkflow(meta: WorkflowMeta, nodes: ToolNode[], edges: WireEdge[]): Workflow {
  return {
    id: meta.id,
    name: meta.name,
    description: meta.description,
    version: 1,
    nodes: nodes.map((node) => ({
      id: node.id,
      kind: node.data.kind,
      position: { x: Math.round(node.position.x), y: Math.round(node.position.y) },
      params: node.data.params,
    })),
    edges: edges.flatMap<WorkflowEdge>((edge) =>
      edge.sourceHandle && edge.targetHandle
        ? [
            {
              id: edge.id,
              source: edge.source,
              sourcePort: edge.sourceHandle,
              target: edge.target,
              targetPort: edge.targetHandle,
            },
          ]
        : [],
    ),
    createdAt: meta.createdAt,
    updatedAt: meta.updatedAt,
  };
}

/** 一个 id 很短、够用就行的节点 id。 */
export function uid(prefix: string): string {
  const globalCrypto = globalThis.crypto;
  if (globalCrypto && typeof globalCrypto.randomUUID === 'function') {
    return globalCrypto.randomUUID();
  }
  return `${prefix}-${Math.random().toString(36).slice(2, 10)}${Date.now().toString(36)}`;
}

// ---------------------------------------------------------------------------
// 从 `resolved` / `edges` 里查东西
//
// 下面这几个都是**热路径**：节点卡片每个端口、每条连线渲染时都要问一次。所以不能
// 每次去扫一遍整个数组 —— 节点一多，那种写法就是「重渲染一次 = 节点数 × 边数」。
// 改成按对象引用缓存一份索引，只要那份数据没换就命中。
// ---------------------------------------------------------------------------

export function resolvedNode(
  resolved: ResolvedWorkflow | null,
  nodeId: string,
): ResolvedNode | undefined {
  return resolved?.nodes.find((node) => node.nodeId === nodeId);
}

const NOTHING: Issue[] = [];

/** 把一次静态检查的结果按「问谁」分好组。 */
type IssueIndex = {
  byNode: Map<string, Issue[]>;
  byEdge: Map<string, Issue[]>;
  byPort: Map<string, Issue[]>;
  global: Issue[];
};

const issueIndexes = new WeakMap<ResolvedWorkflow, IssueIndex>();

function issueIndex(resolved: ResolvedWorkflow): IssueIndex {
  const cached = issueIndexes.get(resolved);
  if (cached) return cached;

  const index: IssueIndex = {
    byNode: new Map(),
    byEdge: new Map(),
    byPort: new Map(),
    global: [],
  };
  const push = (map: Map<string, Issue[]>, key: string, issue: Issue) => {
    const list = map.get(key);
    if (list) list.push(issue);
    else map.set(key, [issue]);
  };

  for (const issue of resolved.issues) {
    if (issue.nodeId) {
      push(index.byNode, issue.nodeId, issue);
      if (issue.portId) push(index.byPort, portKey(issue.nodeId, issue.portId), issue);
    }
    if (issue.edgeId) push(index.byEdge, issue.edgeId, issue);
    if (!issue.nodeId && !issue.edgeId) index.global.push(issue);
  }

  issueIndexes.set(resolved, index);
  return index;
}

const portKey = (nodeId: string, portId: string) => `${nodeId}\u0000${portId}`;

export function issuesForNode(resolved: ResolvedWorkflow | null, nodeId: string): Issue[] {
  if (!resolved) return NOTHING;
  return issueIndex(resolved).byNode.get(nodeId) ?? NOTHING;
}

export function issuesForEdge(resolved: ResolvedWorkflow | null, edgeId: string): Issue[] {
  if (!resolved) return NOTHING;
  return issueIndex(resolved).byEdge.get(edgeId) ?? NOTHING;
}

export function issuesForPort(
  resolved: ResolvedWorkflow | null,
  nodeId: string,
  portId: string,
): Issue[] {
  if (!resolved) return NOTHING;
  return issueIndex(resolved).byPort.get(portKey(nodeId, portId)) ?? NOTHING;
}

/** 和某个节点、某条连线都无关的问题（例如「有环」）。 */
export function globalIssues(resolved: ResolvedWorkflow | null): Issue[] {
  if (!resolved) return NOTHING;
  return issueIndex(resolved).global;
}

/** 一个节点上「连了哪些端口」，拼成两个 `|` 分隔的字符串。 */
export type LinkedPorts = { inputs: string; outputs: string };

/**
 * 扫一遍连线，算出每个节点连了哪些端口。
 *
 * 节点卡片就是拿这个来点亮端口的。以前是每个节点自己去 `edges.filter(...)`，
 * 那意味着每次 store 变动都要跑 N 次 O(边数) 的扫描 —— 拖动时会变成
 * O(节点数 × 边数) 每帧。现在整体算一次，查表是 O(1)。
 */
export function computeLinkedPorts(edges: readonly WireEdge[]): Record<string, LinkedPorts> {
  const linked: Record<string, LinkedPorts> = {};
  const at = (id: string) => (linked[id] ??= { inputs: '', outputs: '' });

  for (const edge of edges) {
    if (edge.targetHandle) {
      const entry = at(edge.target);
      entry.inputs = entry.inputs ? `${entry.inputs}|${edge.targetHandle}` : edge.targetHandle;
    }
    if (edge.sourceHandle) {
      const entry = at(edge.source);
      entry.outputs = entry.outputs ? `${entry.outputs}|${edge.sourceHandle}` : edge.sourceHandle;
    }
  }

  return linked;
}

const linkedCaches = new WeakMap<readonly WireEdge[], Record<string, LinkedPorts>>();

/** `computeLinkedPorts` 的缓存版：同一个 `edges` 数组只算一次。 */
export function linkedPorts(edges: readonly WireEdge[]): Record<string, LinkedPorts> {
  let linked = linkedCaches.get(edges);
  if (!linked) {
    linked = computeLinkedPorts(edges);
    linkedCaches.set(edges, linked);
  }
  return linked;
}

/** 图改了一下之后，编辑器该做什么。 */
export type GraphChangeEffect =
  /** 什么都不用做。 */
  | 'none'
  /** 标一下「有未保存的改动」，但不用重新检查。 */
  | 'touch'
  /** 图的结构变了，得请后端重新解析一遍。 */
  | 'recheck';

/**
 * 这次节点改动要不要重新做静态检查。
 *
 * 关键在于：**位置和尺寸不影响类型检查** —— 挪一个节点不可能改变端口类型或连线
 * 合法性。以前拖动一停下来就会请后端把整张图重解析一遍，回来后所有卡片一起重渲染，
 * 就是那一下卡顿的来源。
 *
 * 尺寸（`dimensions`）也不标脏：那是 React Flow 量出来多高多宽，工作流里不存这个，
 * 刚打开一张存档时量一下不该让「未保存」的蓝点亮起来。
 */
export function nodeChangeEffect(changes: readonly { type: string }[]): GraphChangeEffect {
  let effect: GraphChangeEffect = 'none';
  for (const change of changes) {
    switch (change.type) {
      case 'add':
      case 'remove':
      case 'replace':
        return 'recheck';
      case 'position':
        effect = 'touch';
        break;
      default:
        break;
    }
  }
  return effect;
}

/** 连线的改动：只有点选不用重算。 */
export function edgeChangeEffect(changes: readonly { type: string }[]): GraphChangeEffect {
  return changes.some((change) => change.type !== 'select') ? 'recheck' : 'none';
}

export function formatTime(millis: number | null | undefined): string {
  if (!millis) return '—';
  const date = new Date(millis);
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** 输入节点上「这个文件是什么」那行字。 */
export function describeImage(info: { width: number; height: number; format: string; fileSize: number }): string {
  return `${info.width} × ${info.height} · ${info.format.toUpperCase()} · ${formatBytes(info.fileSize)}`;
}
