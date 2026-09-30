/**
 * 节点卡片的属性比较。
 *
 * 盯的是拖动：React Flow 每帧都会给节点组件塞新的 `positionAbsoluteX/Y`（还有
 * `dragging`、量出来的宽高）。卡片本身不看坐标 —— 移动是外层 `transform` 干的 ——
 * 所以这些变化不该让卡片重渲染。少了这条，拖一张卡就是在每帧重建它整棵子树。
 */

import type { NodeProps } from '@xyflow/react';
import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { ToolNode } from '../lib/types';
import { sameNodeViewProps } from './ToolNodeView';

const data: ToolNode['data'] = { kind: 'input', params: { valueType: 'image' } };

/** 只填参与比较的那几个字段，其余的不影响判断。 */
function props(overrides: Partial<NodeProps<ToolNode>>): NodeProps<ToolNode> {
  return {
    id: 'n1',
    type: 'tool',
    data,
    selected: false,
    dragging: false,
    positionAbsoluteX: 0,
    positionAbsoluteY: 0,
    ...overrides,
  } as NodeProps<ToolNode>;
}

test('只挪了位置就不该重渲染卡片', () => {
  assert.equal(
    sameNodeViewProps(
      props({ positionAbsoluteX: 10, positionAbsoluteY: 20, dragging: true }),
      props({ positionAbsoluteX: 130, positionAbsoluteY: 240, dragging: true }),
    ),
    true,
  );
});

test('量出来的宽高变了也不算内容变化', () => {
  assert.equal(
    sameNodeViewProps(props({ width: 216, height: 180 }), props({ width: 216, height: 240 })),
    true,
  );
});

test('内容真的变了就得重渲染', () => {
  const before = props({});
  assert.equal(sameNodeViewProps(before, props({ id: 'n2' })), false, '换了节点');
  assert.equal(sameNodeViewProps(before, props({ selected: true })), false, '选中态变了');
  assert.equal(
    sameNodeViewProps(before, props({ data: { kind: 'input', params: { valueType: 'text' } } })),
    false,
    '参数变了（data 换了新对象）必须重渲染',
  );
});
