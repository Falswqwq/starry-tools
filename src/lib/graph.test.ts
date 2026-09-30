/**
 * 前端侧的数据形状测试。
 *
 * 这里守的是**前端和后端之间的那条缝**：`toWorkflow` 产出的键名必须是 Rust
 * 那边 `Workflow` 反序列化器认得的名字，端口相容规则也必须和
 * `src-tauri/src/model/port_type.rs` 的 `accepts` 一致。
 *
 * 跑法：npm test
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { edgesFrom, metaOf, nodesFrom, toWorkflow, uid } from './graph';
import {
  computeLinkedPorts,
  edgeChangeEffect,
  globalIssues,
  issuesForEdge,
  issuesForNode,
  issuesForPort,
  linkedPorts,
  nodeChangeEffect,
} from './graph';
import { accepts, portBadge, portKey } from './ports';
import type { Issue, ResolvedWorkflow, ToolNode, WireEdge } from './types';

const meta = {
  id: 'wf-1',
  name: '像素画放大',
  description: '',
  createdAt: null,
  updatedAt: null,
};

function canvas(): { nodes: ToolNode[]; edges: WireEdge[] } {
  return {
    nodes: [
      {
        id: 'n1',
        type: 'tool',
        position: { x: 12.7, y: -3.2 },
        data: { kind: 'input', params: { valueType: 'image', path: '/tmp/a.png' } },
      },
      {
        id: 'n2',
        type: 'tool',
        position: { x: 320, y: 40 },
        data: { kind: 'convert_to_png', params: { compression: 'best' } },
      },
    ],
    edges: [
      {
        id: 'e1',
        type: 'wire',
        source: 'n1',
        sourceHandle: 'out',
        target: 'n2',
        targetHandle: 'image',
      },
    ],
  };
}

test('toWorkflow 用的键名和 Rust 侧一致', () => {
  const { nodes, edges } = canvas();
  const workflow = toWorkflow(meta, nodes, edges);

  assert.deepEqual(Object.keys(workflow).sort(), [
    'createdAt',
    'description',
    'edges',
    'id',
    'name',
    'nodes',
    'updatedAt',
    'version',
  ]);
  assert.deepEqual(Object.keys(workflow.nodes[0]!).sort(), ['id', 'kind', 'params', 'position']);
  assert.deepEqual(Object.keys(workflow.edges[0]!).sort(), [
    'id',
    'source',
    'sourcePort',
    'target',
    'targetPort',
  ]);

  // 位置取整，免得每次保存都因为浮点尾巴产生 diff。
  assert.deepEqual(workflow.nodes[0]!.position, { x: 13, y: -3 });
  assert.equal(workflow.edges[0]!.sourcePort, 'out');
  assert.equal(workflow.edges[0]!.targetPort, 'image');
});

test('画布和工作流之间来回转换不掉东西', () => {
  const { nodes, edges } = canvas();
  const workflow = toWorkflow(meta, nodes, edges);
  const restored = toWorkflow(metaOf(workflow), nodesFrom(workflow), edgesFrom(workflow));
  assert.deepEqual(restored, workflow);
});

test('没有连上两端的边不会被写进工作流', () => {
  const { nodes } = canvas();
  const dangling: WireEdge = {
    id: 'e9',
    type: 'wire',
    source: 'n1',
    target: 'n2',
    sourceHandle: null,
    targetHandle: null,
  };
  assert.equal(toWorkflow(meta, nodes, [dangling]).edges.length, 0);
});

test('端口相容规则与后端一致', () => {
  // 同类型可以接。
  assert.equal(accepts({ image: 'png' }, { image: 'png' }), true);
  assert.equal(accepts('text', 'text'), true);

  // Image(Png) 和 Image(Jpeg) 是两种类型。
  assert.equal(accepts({ image: 'png' }, { image: 'jpeg' }), false);
  assert.equal(accepts({ image: 'jpeg' }, { image: 'png' }), false);

  // 「格式未知」两头都放行，等运行期再确认。
  assert.equal(accepts({ image: 'any' }, { image: 'jpeg' }), true);
  assert.equal(accepts({ image: 'png' }, { image: 'any' }), true);

  // 跨类型族一律不行。
  assert.equal(accepts('text', { image: 'png' }), false);
  assert.equal(accepts('number', 'text'), false);

  // 通配两头都放行 —— 「重命名」这类不管类型的节点靠它。
  assert.equal(accepts('any', 'text'), true);
  assert.equal(accepts('any', { image: 'png' }), true);
  assert.equal(accepts('text', 'any'), true);
  assert.equal(accepts({ image: 'png' }, 'any'), true);
});

test('端口徽标认得具体格式', () => {
  assert.equal(portBadge({ image: 'png' }), 'PNG');
  assert.equal(portBadge({ image: 'jpeg' }), 'JPG');
  assert.equal(portBadge({ image: 'any' }), 'IMG');
  assert.equal(portBadge('text'), 'TXT');
  assert.equal(portKey({ image: 'webp' }), 'image:webp');
});

// ---------------------------------------------------------------------------
// 拖动热路径上的两块优化。它们盯的是一个很具体的毛病：节点一多，拖起来就卡。
// ---------------------------------------------------------------------------

const wire = (id: string, source: string, sourceHandle: string, target: string) => ({
  id,
  source,
  sourceHandle,
  target,
  targetHandle: 'image',
});

test('每个节点连了哪些端口，一次算清楚', () => {
  const links = computeLinkedPorts([
    wire('e1', 'a', 'out', 'b'),
    wire('e2', 'a', 'image', 'c'),
    wire('e3', 'c', 'image', 'b'),
  ]);

  assert.equal(links.a?.outputs, 'out|image');
  assert.equal(links.a?.inputs, '');
  assert.equal(links.b?.inputs, 'image|image');
  assert.equal(links.c?.inputs, 'image');
  assert.equal(links.c?.outputs, 'image');
  assert.equal(links.nowhere, undefined, '没连线的节点就不该出现在表里');
});

test('连线没换就直接命中缓存', () => {
  // 这条是性能本身：拖动时 `edges` 数组不会换，所以每个节点的查表必须是纯命中，
  // 不能又去扫一遍连线 —— 那就是「节点数 × 边数」每帧。
  const edges = [wire('e1', 'a', 'out', 'b')];
  const first = linkedPorts(edges);
  assert.equal(linkedPorts(edges), first, '同一个数组该拿到同一个对象');
  assert.notEqual(linkedPorts([...edges]), first, '换了数组就得重算');
});

test('位置变化只标脏，不重新检查', () => {
  assert.equal(nodeChangeEffect([{ type: 'position' }]), 'touch');
  assert.equal(nodeChangeEffect([{ type: 'select' }]), 'none');
  assert.equal(nodeChangeEffect([{ type: 'dimensions' }]), 'none');
  // 结构变了就一定要重算。
  assert.equal(nodeChangeEffect([{ type: 'position' }, { type: 'remove' }]), 'recheck');
  assert.equal(nodeChangeEffect([{ type: 'add' }]), 'recheck');
  assert.equal(nodeChangeEffect([{ type: 'replace' }]), 'recheck');
  assert.equal(nodeChangeEffect([]), 'none');
});

test('连线只有点选不用重算', () => {
  assert.equal(edgeChangeEffect([{ type: 'select' }]), 'none');
  assert.equal(edgeChangeEffect([{ type: 'remove' }]), 'recheck');
  assert.equal(edgeChangeEffect([{ type: 'add' }]), 'recheck');
});

function resolvedWith(issues: Issue[]): ResolvedWorkflow {
  return { nodes: [], issues, runnable: issues.length === 0 };
}

test('问题按「问谁」分好组，查出来和以前一样', () => {
  const resolved = resolvedWith([
    { severity: 'error', message: '端口问题', nodeId: 'a', portId: 'image' },
    { severity: 'warning', message: '节点问题', nodeId: 'a' },
    { severity: 'error', message: '连线问题', edgeId: 'e1' },
    { severity: 'error', message: '有环' },
  ]);

  assert.deepEqual(
    issuesForPort(resolved, 'a', 'image').map((issue) => issue.message),
    ['端口问题'],
  );
  assert.deepEqual(
    issuesForNode(resolved, 'a').map((issue) => issue.message),
    ['端口问题', '节点问题'],
  );
  assert.deepEqual(
    issuesForEdge(resolved, 'e1').map((issue) => issue.message),
    ['连线问题'],
  );
  assert.deepEqual(
    globalIssues(resolved).map((issue) => issue.message),
    ['有环'],
  );

  // 没问到的返回空，不能因为共用一张空表就串了。
  assert.equal(issuesForPort(resolved, 'b', 'image').length, 0);
  assert.equal(issuesForPort(null, 'a', 'image').length, 0);
});

test('节点 id 每次都不一样', () => {
  const ids = new Set(Array.from({ length: 200 }, () => uid('n')));
  assert.equal(ids.size, 200);
});
