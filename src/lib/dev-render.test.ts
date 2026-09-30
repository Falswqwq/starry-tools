/**
 * 渲染计数器的行为。
 *
 * 这东西本身没什么逻辑，但它给出的数字是排查「拖起来卡」时的第一手证据，
 * 数字错了会把人带偏，所以顺手钉一下。
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { countRender, renderStats } from './dev-render';

/** 把 console.table 的调用截下来，别喷到测试输出里。 */
function captureTable<T>(run: () => T): { rows: unknown[]; result: T } {
  const original = console.table;
  const rows: unknown[] = [];
  console.table = ((value: unknown) => {
    rows.push(value);
  }) as typeof console.table;
  try {
    return { rows, result: run() };
  } finally {
    console.table = original;
  }
}

test('计数按标记分开累加', () => {
  renderStats.reset();
  countRender('节点卡片');
  countRender('节点卡片');
  countRender('连线');

  const { rows } = captureTable(() => renderStats.report());
  assert.equal(rows.length, 1);
  assert.deepEqual(rows[0], [
    { 标记: '节点卡片', 每秒渲染: 2 },
    { 标记: '连线', 每秒渲染: 1 },
  ]);
});

test('打完之后清零，下一张表是下一秒的', () => {
  renderStats.reset();
  countRender('节点卡片');
  captureTable(() => renderStats.report());

  const { rows } = captureTable(() => renderStats.report());
  assert.equal(rows.length, 0, '这一秒什么都没渲染就不该打表');
});

test('把「哪个组件最勤」排在前面', () => {
  renderStats.reset();
  countRender('画布');
  for (let i = 0; i < 5; i++) countRender('节点卡片');
  for (let i = 0; i < 3; i++) countRender('参数控件');

  const { rows } = captureTable(() => renderStats.report());
  assert.deepEqual(
    (rows[0] as { 标记: string }[]).map((row) => row.标记),
    ['节点卡片', '参数控件', '画布'],
  );
});
