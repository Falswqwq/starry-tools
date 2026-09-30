/**
 * 参数可见性的测试。
 *
 * 盯的是「跟着别的参数变」这件事 —— 节点上有的参数只在特定组合下才有意义
 * （比如压缩节点的「颜色数上限」只在有损 + 调色板时才该出现）。
 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isVisible } from './params';
import type { ParamDef } from './types';

const plain: ParamDef = { id: 'a', label: '甲', control: 'text', default: '' };

test('没有条件的参数一律显示', () => {
  assert.equal(isVisible(plain, {}), true);
});

test('单条件：跟着一个参数取值走', () => {
  const def: ParamDef = {
    ...plain,
    visibleWhen: { param: 'mode', anyOf: ['lossy'] },
  };
  assert.equal(isVisible(def, { mode: 'lossy' }), true);
  assert.equal(isVisible(def, { mode: 'lossless' }), false);
  assert.equal(isVisible(def, {}), false, '参数还没落值时就不该显示');
});

test('多条件：全都得满足', () => {
  const def: ParamDef = {
    ...plain,
    visibleWhen: {
      param: 'mode',
      anyOf: ['lossy'],
      allOf: [{ param: 'lossyFormat', anyOf: ['palette'] }],
    },
  };
  assert.equal(isVisible(def, { mode: 'lossy', lossyFormat: 'palette' }), true);
  assert.equal(isVisible(def, { mode: 'lossy', lossyFormat: 'jpeg' }), false);
  assert.equal(isVisible(def, { mode: 'lossless', lossyFormat: 'palette' }), false);
});

test('嵌套的「而且」也能一层层判下去', () => {
  const def: ParamDef = {
    ...plain,
    visibleWhen: {
      param: 'a',
      anyOf: ['1'],
      allOf: [
        {
          param: 'b',
          anyOf: ['2'],
          allOf: [{ param: 'c', anyOf: ['3'] }],
        },
      ],
    },
  };
  assert.equal(isVisible(def, { a: '1', b: '2', c: '3' }), true);
  assert.equal(isVisible(def, { a: '1', b: '2', c: '9' }), false);
  assert.equal(isVisible(def, { a: '1', b: '9', c: '3' }), false);
});

test('数字和布尔的取值也能比', () => {
  const def: ParamDef = {
    ...plain,
    visibleWhen: { param: 'colors', anyOf: ['4'] },
  };
  assert.equal(isVisible(def, { colors: 4 }), true);
  assert.equal(isVisible(def, { colors: 5 }), false);
});
