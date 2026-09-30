/**
 * 「在浏览器里跑」那条路要用的搬运工具。
 *
 * 它本身没多少逻辑，但搬过去的东西少一样，画布就和 Tauri 里的不一样（比如少了
 * `resolved`，卡片就不画端口那行），拿去做绘制性能对比会得出错误结论 —— 所以钉一下。
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { patchFromJson, snapshotOf, snapshotToJson } from './dev-snapshot';
import type { NodeKind, ToolNode } from './types';

const meta = {
  id: 'wf-1',
  name: '像素画放大',
  description: '',
  createdAt: null,
  updatedAt: null,
};

const kind: NodeKind = {
  id: 'compress_image',
  name: '图像压缩',
  category: '图像',
  description: '把图像编码得更小。',
  isSource: false,
  inputs: [{ id: 'image', label: '图像', ty: { image: 'any' } as const, required: true }],
  outputs: [{ id: 'image', label: 'PNG 图像', ty: { image: 'png' } as const, required: false }],
  params: [],
  notes: [],
  defaults: { mode: 'lossless' },
};

const node: ToolNode = {
  id: 'n1',
  type: 'tool',
  position: { x: 10, y: 20 },
  data: { kind: 'compress_image', params: { mode: 'lossless' } },
};

test('搬过去再接回来，几个关键切片一个不少', () => {
  const source = {
    meta,
    nodes: [node],
    edges: [],
    kinds: [kind],
    resolved: { nodes: [], issues: [], runnable: true },
  };

  const patch = patchFromJson(snapshotToJson(snapshotOf(source)));

  assert.deepEqual(patch.meta, meta);
  assert.deepEqual(patch.nodes, [node]);
  assert.deepEqual(patch.kinds, [kind]);
  assert.deepEqual(patch.resolved, { nodes: [], issues: [], runnable: true });
  assert.equal(patch.ready, true, '装完就该是 ready，不然顶栏不显示');
  assert.equal(patch.dirty, false, '刚搬过来的图不该是「未保存」');
});

test('kindById 是从 kinds 推出来的，不用另存一份', () => {
  const patch = patchFromJson(
    snapshotToJson(snapshotOf({ meta, nodes: [], edges: [], kinds: [kind], resolved: null })),
  );
  assert.deepEqual(Object.keys(patch.kindById as object), ['compress_image']);
});

test('少带东西也不算致命，缺的给个安全的默认值', () => {
  const patch = patchFromJson(JSON.stringify({ meta, nodes: [node], kinds: [kind] }));
  assert.deepEqual(patch.edges, []);
  assert.equal(patch.resolved, null);
});

test('贴错东西会直接报错，而不是塞进去一堆 undefined', () => {
  assert.throws(() => patchFromJson('{}'), /快照/);
  assert.throws(() => patchFromJson('不是 JSON'));
});
