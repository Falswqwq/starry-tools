/**
 * 参数控件必须带上 `nodrag`。
 *
 * 这是针对一个真实 bug 的回归测试：React Flow 只在节点内部找 `nodrag` 这个类，
 * 除此之外不看别的。少了它，在 `<select>` 上按下鼠标就开始拖节点，而原生下拉
 * 菜单会把 mouseup 吃掉，拖拽再也停不下来 —— 节点就粘在鼠标上了。
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';

import type { ParamDef } from '../lib/types';
import { ParamField } from './ParamField';

const css = readFileSync('src/styles/app.css', 'utf8');

function render(def: ParamDef, value: unknown = def.default): string {
  return renderToStaticMarkup(
    <ParamField
      def={def}
      value={value}
      onChange={() => undefined}
      onPickFile={() => undefined}
    />,
  );
}

const labelTargets = (html: string) =>
  [...html.matchAll(/<label[^>]*for="([^"]+)"/g)].map((match) => match[1]);

const labelIds = (html: string) =>
  [...html.matchAll(/<label[^>]*id="([^"]+)"/g)].map((match) => match[1]);

const numberParam: ParamDef = {
  id: 'percent',
  label: '缩放比例',
  control: 'number',
  default: 200,
  min: 1,
  max: 1600,
  step: 1,
  integer: false,
  unit: '%',
};

const sliderParam: ParamDef = {
  id: 'loss',
  label: '允许的损耗率',
  control: 'slider',
  default: 15,
  min: 0,
  max: 100,
  step: 1,
  integer: true,
  unit: '%',
};

const textParam: ParamDef = {
  id: 'note',
  label: '备注',
  control: 'text',
  default: '',
  multiline: false,
};

const multiLineParam: ParamDef = { ...textParam, multiline: true };

const selectParam: ParamDef = {
  id: 'compression',
  label: '压缩档位',
  control: 'select',
  default: 'default',
  options: [
    { value: 'fast', label: '快速' },
    { value: 'default', label: '均衡' },
  ],
};

const boolParam: ParamDef = {
  id: 'enabled',
  label: '启用',
  control: 'bool',
  default: true,
};

const fileParam: ParamDef = {
  id: 'path',
  label: '图像文件',
  control: 'file',
  default: '',
  dialogTitle: '选择图像文件',
  extensions: ['png', 'jpg'],
};

const allControls: [string, ParamDef, unknown][] = [
  ['number', numberParam, 200],
  ['slider', sliderParam, 15],
  ['text', textParam, 'hi'],
  ['text(multiline)', multiLineParam, 'hi'],
  ['select', selectParam, 'fast'],
  ['bool', boolParam, true],
  ['file', fileParam, ''],
];

test('每种参数控件都带 nodrag', () => {
  for (const [name, def, value] of allControls) {
    const html = render(def, value);
    assert.match(html, /nodrag/, `${name} 控件少了 nodrag，在它上面按下鼠标会拖走节点`);
  }
});

test('多行文本框还带 nowheel，免得在框内滚轮缩放画布', () => {
  assert.match(render(multiLineParam, 'hi'), /nowheel/);
});

test('单行控件不需要 nowheel', () => {
  assert.doesNotMatch(render(textParam, 'hi'), /nowheel/);
});

test('滑杆走的是 Radix 的 slider，并显示当前读数', () => {
  const html = render(sliderParam, 42);
  assert.match(html, /role="slider"/);
  assert.match(html, /42%/, '读数要把单位和值一起显示出来');
  assert.match(html, /nodrag/);
});

test('开关打开时圆点真的会滑过去', () => {
  // 这是一个只在屏幕上看得出来、看代码看不出来的毛病：
  // 圆点本身有 transform，却没有「打开时挪到另一头」的规则，看着就像没有动画。
  assert.match(
    css,
    /\.ui-switch\[data-state='checked'\]\s+\.ui-switch__thumb\s*\{[^}]*transform:\s*translateX\(\s*\d+px\s*\)/,
    '少了这条规则圆点只会待在原地',
  );
  assert.match(
    css,
    /\.ui-switch__thumb\s*\{[^}]*transition:\s*transform/,
    '位移要有 transition 才有滑动感',
  );
});

test('没选文件时是虚线框，选完后文件名顶在按钮的位置', () => {
  const empty = render(fileParam, '');
  assert.match(empty, /class="file-pick"/, '空着的时候不该带 is-filled');
  assert.match(empty, /file-pick__btn/);
  assert.match(empty, /选择文件…/);

  const filled = render(fileParam, '/tmp/sprites/hero.png');
  assert.match(filled, /class="file-pick is-filled"/);
  assert.match(filled, /file-pick__label"[^>]*>hero\.png/, '文件名要顶在按钮的位置');
  assert.match(filled, /换一个文件/, '鼠标移上去要能重新选');
  // 虚线框只该出现在没选的时候。
  assert.doesNotMatch(filled, /选择文件…/);
});

test('目录参数用的是「选择目录」，不是「选择文件」', () => {
  const directoryParam: ParamDef = {
    id: 'directory',
    label: '目标目录',
    control: 'file',
    default: '',
    dialogTitle: '选择保存目录',
    extensions: [],
    directory: true,
  };

  const empty = render(directoryParam, '');
  assert.match(empty, /选择目录…/);
  assert.doesNotMatch(empty, /选择文件/, '目录参数不该说「选择文件」');
  assert.match(empty, /nodrag/, '按钮也得带 nodrag，否则会拖走节点');

  const chosen = render(directoryParam, '/tmp/exports');
  assert.match(chosen, /换一个目录/);
  assert.match(chosen, /exports/, '选完目录要把名字显示出来');
});

test('label 与控件用同一个 id 绑定', () => {
  const html = render(numberParam, 200);
  const [forId] = labelTargets(html);
  assert.ok(forId, 'label 应该有 for 属性');
  assert.match(html, new RegExp(`id="${forId}"`), 'label 指向的 id 必须真的存在');
});

test('同一棵树上两个同类型的参数不会撞 id', () => {
  // 画布上完全可以有两个「缩放图像」节点，它们的参数不能指向同一个 id，
  // 否则 label 会去点亮隔壁节点里的控件。
  const html = renderToStaticMarkup(
    <>
      <ParamField def={numberParam} value={200} onChange={() => undefined} onPickFile={() => undefined} />
      <ParamField def={numberParam} value={200} onChange={() => undefined} onPickFile={() => undefined} />
    </>,
  );
  const ids = labelTargets(html);
  assert.equal(ids.length, 2);
  assert.notEqual(ids[0], ids[1]);
});

test('可见性由 visibleWhen 决定，这里只检查渲染不炸', () => {
  const conditional: ParamDef = {
    ...textParam,
    visibleWhen: { param: 'valueType', anyOf: ['text'] },
  };
  assert.match(render(conditional, 'hi'), /nodrag/);
});

test('下拉框走的是 Radix 的 combobox，而不是原生 select', () => {
  const html = render(selectParam, 'fast');
  // 原生 <select> 的弹层没法做样式，所以这里必须是自制列表盒。
  assert.match(html, /role="combobox"/);
  const [labelId] = labelIds(html);
  assert.ok(labelId, '下拉框应该有自己的 label');
  assert.match(html, new RegExp(`aria-labelledby="${labelId}"`));
});
