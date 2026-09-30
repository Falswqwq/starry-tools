/**
 * 拖拽两侧的回归测试。
 *
 * 有两个角色，别搞混了：
 *   * 吸附在指针上的胶囊（`DragGhostView`）—— 显示节点名和输入输出类型；
 *   * 节点库里被拖走的那张卡片（`ToolCard`）—— 被一个「向外」的箭头盖住。
 *
 * 这两处都只在拖动那一瞬间出现，出了问题屏幕上也看不出原因，所以把结构
 * 和 CSS 类名都钉住。
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';

import type { NodeKind } from '../lib/types';
import { DragGhostView, ToolCard } from './NodeLibrary';

const kind: NodeKind = {
  id: 'convert_to_png',
  name: '图像格式转换',
  category: '图像',
  description: '把任何能解码的图像重新编码成 PNG。',
  isSource: false,
  inputs: [{ id: 'image', label: '图像', ty: { image: 'any' }, required: true }],
  outputs: [{ id: 'image', label: 'PNG 图像', ty: { image: 'png' }, required: false }],
  params: [
    {
      id: 'compression',
      label: '压缩档位',
      control: 'select',
      default: 'default',
      options: [{ value: 'fast', label: '快速' }],
    },
  ],
  notes: ['GIF 只取第一帧。'],
  defaults: { compression: 'default' },
};

const css = readFileSync('src/styles/app.css', 'utf8');

function card(dragging: boolean, open = false): string {
  return renderToStaticMarkup(
    <ToolCard
      kind={kind}
      open={open}
      dragging={dragging}
      onOpenChange={() => undefined}
      onPointerDown={() => undefined}
      onPlaceAtCentre={() => undefined}
      suppressClick={{ current: false }}
    />,
  );
}

/** 渲染结果里用到的类名，都必须在 app.css 里有对应规则。 */
function assertClassesExist(markup: string, label: string) {
  const classes = new Set(
    [...markup.matchAll(/class="([^"]+)"/g)].flatMap((match) => match[1]!.split(/\s+/)),
  );
  for (const className of classes) {
    // lucide 是自己带的图标类；nodrag / nowheel 是 React Flow 的约定，不是样式。
    if (!className || className.startsWith('lucide')) continue;
    if (['nodrag', 'nowheel', 'nopan'].includes(className)) continue;
    assert.match(
      css,
      new RegExp(`\\.${className}[\\s,{:]`),
      `${label} 用了 .${className}，但 app.css 里没有这条规则`,
    );
  }
}

// ---------------------------------------------------------- 指针上的胶囊 ---

test('胶囊显示节点名', () => {
  const html = renderToStaticMarkup(<DragGhostView kind={kind} x={120} y={340} />);
  assert.match(html, /drag-ghost__name/);
  assert.match(html, /图像格式转换/);
});

test('胶囊带输入输出类型，但不带那个向外箭头', () => {
  const html = renderToStaticMarkup(<DragGhostView kind={kind} x={0} y={0} />);
  assert.match(html, /drag-ghost__types/);
  assert.match(html, /IMG/, '输入类型徽标应该在');
  assert.match(html, /PNG/, '输出类型徽标应该在');
  // 箭头是卡片那侧的活儿，别跑到胶囊上来。
  assert.doesNotMatch(html, /drag-ghost__arrow/);
  assert.doesNotMatch(html, /card__out/);
});

test('胶囊位置跟着指针走，放下时带 is-leaving', () => {
  const dragging = renderToStaticMarkup(<DragGhostView kind={kind} x={120} y={340} />);
  assert.match(dragging, /left:120px/);
  assert.match(dragging, /top:340px/);
  assert.doesNotMatch(dragging, /is-leaving/);

  const leaving = renderToStaticMarkup(<DragGhostView kind={kind} x={0} y={0} leaving />);
  assert.match(leaving, /is-leaving/);
});

// ------------------------------------------------------------ 库里那张卡 ---

test('拖动时卡片被向外箭头盖住', () => {
  const html = card(true);
  assert.match(html, /is-dragging-out/);
  assert.match(html, /card__out/);
  assert.match(html, /<svg/, '箭头只是个空壳，图标没渲染出来');
  assert.match(html, /lucide-arrow-right/, '画的不是向右的箭头');
});

test('没在拖动时卡片上不该有箭头', () => {
  const html = card(false);
  assert.doesNotMatch(html, /card__out/);
  assert.doesNotMatch(html, /is-dragging-out/);
});

test('展开着的卡片拖动时也照样被盖住', () => {
  const html = card(true, true);
  assert.match(html, /is-dragging-out/);
  assert.match(html, /card__out/);
});

test('箭头是绝对定位的，否则盖不住卡片', () => {
  assert.match(
    css,
    /\.card__out\s*\{[^}]*position:\s*absolute/,
    '箭头得是绝对定位才能盖住卡片内容',
  );
  assert.match(
    css,
    /\.card__summary\s*\{[^}]*position:\s*relative/,
    '箭头相对定位的参照物是 card__summary，它得是 relative',
  );
});

// ------------------------------------------------------------ 类名一致性 ---

test('胶囊和卡片用到的每个类名都在 app.css 里定义过', () => {
  // 这一条盯的是「改了一边的类名、另一边忘了改」—— 这类问题在屏幕上的表现
  // 只是少了个箭头或者样式没生效，很难查。
  assertClassesExist(renderToStaticMarkup(<DragGhostView kind={kind} x={0} y={0} />), '胶囊');
  assertClassesExist(card(true, false), '卡片（收起）');
  assertClassesExist(card(true, true), '卡片（展开）');
});
