/** 画布本体。除了几个浮动按钮，屏幕上只有它。 */

import {
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  useReactFlow,
  type EdgeTypes,
  type NodeTypes,
} from '@xyflow/react';
import { useEffect } from 'react';

import { countRender } from '../lib/dev-render';
import { firstIntersection, type Point } from '../lib/geometry';
import { useStore } from '../state/store';
import { canvasHost } from './canvas-host';
import { sampleEdges } from './slash';
import { ToolNodeView } from './ToolNodeView';
import { WireEdge } from './WireEdge';

const nodeTypes: NodeTypes = { tool: ToolNodeView };
const edgeTypes: EdgeTypes = { wire: WireEdge };

/**
 * 画布背景那层灰色小点。
 *
 * 它是一整片 SVG pattern 平铺在视口下面 —— 节点在它上面动，底下那片图案
 * 所在的区域就可能要跟着重绘。怀疑它是拖动卡顿的元凶，先关掉验证。
 * 改回 `true` 就回来了（Vite 会热更新，不用重启）。
 */
const SHOW_DOT_BACKGROUND = false;

/**
 * 这几个得提到组件外面。画布每拖一帧就会重渲染一次，写成字面量的话
 * 每帧都是新对象，React Flow 会把它们同步进内部 store，把本来没变的连线全推倒重来。
 */
const DEFAULT_EDGE_OPTIONS = { type: 'wire' };
const CONNECTION_LINE_STYLE = { stroke: '#2563eb', strokeWidth: 2 };
const FIT_VIEW_OPTIONS = { padding: 0.32 };

/**
 * 右键落在这些元素上时不该起刀 —— 节点、连线上右键该弹原生菜单。
 *
 * 注意不能用 `.react-flow__pane` 判断「空白处」：pane 是所有内容的外层容器，
 * 点在节点上时也命中它。只能用排除法。
 */
const NOT_EMPTY = [
  '.react-flow__node',
  '.react-flow__edge',
  '.react-flow__handle',
  '.react-flow__controls',
  '.react-flow__panel',
  '.react-flow__attribution',
].join(', ');

export function Canvas() {
  countRender('画布');
  const nodes = useStore((state) => state.nodes);
  const edges = useStore((state) => state.edges);
  const workflowId = useStore((state) => state.meta.id);
  const slashing = useStore((state) => state.slash !== null);
  const onNodesChange = useStore((state) => state.onNodesChange);
  const onEdgesChange = useStore((state) => state.onEdgesChange);
  const onConnect = useStore((state) => state.onConnect);
  const canConnect = useStore((state) => state.canConnect);
  const forgetNodes = useStore((state) => state.forgetNodes);
  const select = useStore((state) => state.select);

  const { fitView, screenToFlowPosition } = useReactFlow();

  // 换了工作流就把视图重新框好。
  useEffect(() => {
    const timer = setTimeout(() => void fitView({ padding: 0.32, maxZoom: 1.05 }), 60);
    return () => clearTimeout(timer);
  }, [workflowId, fitView]);

  /** 右键按住并滑动 = 划一刀；松手时被切中的连线断开。 */
  function beginSlash(event: React.PointerEvent) {
    if (event.button !== 2) return;
    if ((event.target as Element).closest(NOT_EMPTY)) return;

    // 连线在拖动过程中不会变，采样一次就够。
    const paths = sampleEdges();
    const from = { x: event.clientX, y: event.clientY };
    const flowFrom = screenToFlowPosition(from);
    let flowTo: Point = flowFrom;

    useStore.getState().beginSlash(from.x, from.y);

    // 右键拖动的全程都得按住菜单：它在 pointerup 之后还会冒出来一次。
    const swallowMenu = (menuEvent: Event) => menuEvent.preventDefault();
    window.addEventListener('contextmenu', swallowMenu, true);

    const onMove = (move: PointerEvent) => {
      const tip = { x: move.clientX, y: move.clientY };
      flowTo = screenToFlowPosition(tip);
      const doomed = paths
        .filter((edge) => firstIntersection(edge.points, flowFrom, flowTo) !== null)
        .map((edge) => edge.id);
      useStore.getState().updateSlash(tip.x, tip.y, doomed);
    };

    const detach = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('blur', onCancel);
      window.setTimeout(
        () => window.removeEventListener('contextmenu', swallowMenu, true),
        0,
      );
    };

    const onUp = () => {
      detach();
      const doomed = useStore.getState().slash?.doomed ?? [];
      const cuts = doomed.flatMap((edgeId) => {
        const edge = paths.find((candidate) => candidate.id === edgeId);
        if (!edge) return [];
        const hit = firstIntersection(edge.points, flowFrom, flowTo);
        if (!hit) return [];
        return [{ edgeId, point: hit.point, distance: hit.distance, length: edge.length }];
      });
      useStore.getState().cutEdges(cuts);
    };

    const onCancel = () => {
      detach();
      useStore.getState().abandonSlash();
    };

    const onKeyDown = (keyEvent: KeyboardEvent) => {
      if (keyEvent.key === 'Escape') onCancel();
    };

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    window.addEventListener('keydown', onKeyDown);
    // 鼠标在窗口外面松开时收不到 pointerup，用失焦兼底，免得刀光挂在屏幕上。
    window.addEventListener('blur', onCancel);
  }

  return (
    <div
      className={`canvas ${slashing ? 'is-slashing' : ''}`}
      ref={(element) => {
        canvasHost.element = element;
      }}
      onPointerDown={beginSlash}
    >
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onNodesDelete={(deleted) => forgetNodes(deleted.map((node) => node.id))}
        isValidConnection={canConnect}
        onNodeClick={(_, node) => select(node.id)}
        onPaneClick={() => select(null)}
        // 空白处的右键交给刀光，不弹原生菜单。
        onPaneContextMenu={(event) => event.preventDefault()}
        deleteKeyCode={['Backspace', 'Delete']}
        // 明确只用左键平移，把右键腾出来。
        panOnDrag={[0]}
        // 挪一点点才算拖拽，点选和按控件不会被当成拖动。
        // 参数控件自己带着 nodrag（见 ParamField / Button），这里是第二道防盗门。
        nodeDragThreshold={2}
        minZoom={0.2}
        maxZoom={1.8}
        fitView
        fitViewOptions={FIT_VIEW_OPTIONS}
        defaultEdgeOptions={DEFAULT_EDGE_OPTIONS}
        connectionLineStyle={CONNECTION_LINE_STYLE}
        nodesConnectable
        elevateEdgesOnSelect
      >
        {SHOW_DOT_BACKGROUND && (
          <Background variant={BackgroundVariant.Dots} gap={22} size={1.5} color="#d7dbe1" />
        )}
        <Controls showInteractive={false} position="bottom-left" />
      </ReactFlow>
    </div>
  );
}
