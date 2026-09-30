/**
 * 界面骨架的回归测试。
 *
 * 盯着的是「TSX 里写了一个类名，app.css 里却没有这条规则」—— 这类问题在屏幕上
 * 的表现只是「看着有点不对」，不报错、不崩，很容易漏过去。（之前就漏过一次：
 * 主题换色时删掉了 `.chrome-btn` 的规则，三个按钮还在用它。）
 *
 * 做法是把组件渲染成静态 HTML，把它用到的每个类名拿去 app.css 里对一遍。
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { ReactFlowProvider } from '@xyflow/react';
import { renderToStaticMarkup } from 'react-dom/server';

import { TopLeftChrome, TopRightChrome } from './Chrome';
import { RunStatus } from './RunStatus';
import { TooltipProvider } from '../ui/Tooltip';

const css = readFileSync('src/styles/app.css', 'utf8');

/** 这些是别人家的约定，不是我们定义的样式 */
const FOREIGN = new Set(['nodrag', 'nowheel', 'nopan']);

/** 渲染结果里用到的每个类名，都必须在 app.css 里有对应规则。 */
export function assertClassesExist(markup: string, label: string) {
  const classes = new Set(
    [...markup.matchAll(/class="([^"]+)"/g)].flatMap((match) => match[1]!.split(/\s+/)),
  );
  assert.ok(classes.size > 0, `${label} 没渲染出任何类名，测试本身可能失效了`);

  for (const className of classes) {
    if (!className || className.startsWith('lucide')) continue;
    if (FOREIGN.has(className)) continue;
    assert.match(
      css,
      new RegExp(`\\.${className}[\\s,{:]`),
      `${label} 用了 .${className}，但 app.css 里没有这条规则`,
    );
  }
}

/** 和 App 里一样的三层壳：提示、画布上下文。 */
function render(node: React.ReactNode): string {
  return renderToStaticMarkup(
    <TooltipProvider>
      <ReactFlowProvider>{node}</ReactFlowProvider>
    </TooltipProvider>,
  );
}

test('左上角那几个控件用到的类名都在 app.css 里', () => {
  assertClassesExist(render(<TopLeftChrome />), '左上角');
});

test('右上角那几个控件用到的类名都在 app.css 里', () => {
  assertClassesExist(render(<TopRightChrome />), '右上角');
});

test('底部状态药丸用到的类名都在 app.css 里', () => {
  assertClassesExist(render(<RunStatus />), '状态药丸');
});

test('顶栏只是按钮排成一行，没有外面那层胶囊壳', () => {
  const html = render(
    <>
      <TopLeftChrome />
      <TopRightChrome />
    </>,
  );
  assert.match(html, /class="toolbar toolbar--left"/);
  assert.match(html, /class="toolbar toolbar--right"/);
  assert.doesNotMatch(html, /chrome-stack/, '胶囊壳应该已经去掉了');
});

test('标题里的品牌字样已经拿掉', () => {
  const html = render(<TopLeftChrome />);
  assert.doesNotMatch(html, /StarryTools/);
  assert.match(html, /工作流名称/, '工作流名输入框要留着');
});

test('工作流名和说明按钮之间不再夹着未保存提示点', () => {
  const html = render(<TopLeftChrome />);
  assert.doesNotMatch(html, /class="dirty/, '提示点应该挂在右上角的保存按钮下面');

  // 输入框和说明按钮紧挨着
  const input = html.indexOf('class="wfname"');
  const info = html.indexOf('aria-label="说明与概要"');
  assert.ok(input >= 0 && info > input, '说明按钮应该在输入框右边');
  assert.doesNotMatch(html.slice(input, info), /dirty/);
});

test('未保存提示点挂在保存按钮下面', () => {
  const html = render(<TopRightChrome />);
  const slot = html.indexOf('class="save-slot"');
  // 取到下一个兄弟（运行按钮）为止，这一段就是 save-slot 的内容
  const next = html.indexOf('ui-btn--primary');
  assert.ok(slot >= 0 && next > slot, 'save-slot 应该排在运行按钮前面');

  const inside = html.slice(slot, next);
  assert.match(inside, /ui-btn--icon/, '里面应该是那个保存按钮');
  assert.match(inside, /class="dirty/, '提示点应该在 save-slot 里面');
});

test('说明 / 保存 / 加载 三个按钮一样大', () => {
  const left = render(<TopLeftChrome />);
  const right = render(<TopRightChrome />);
  const count = (html: string, needle: string) => html.split(needle).length - 1;

  // 右上：加载 + 保存，都是图标方块按钮
  assert.equal(count(right, 'ui-btn--icon'), 2, '加载和保存应该都是同一个尺寸');
  assert.doesNotMatch(right, /ui-btn--md/, '右上角不该混进默认尺寸的按钮');
  // 左上：说明按钮也是同一款。（节点库是有字的那种，不在此列）
  assert.equal(count(left, 'ui-btn--icon'), 1, '说明按钮应该和保存/加载同尺寸');
});

test('未保存提示点落在保存按钮下沿之外，不和它重叠', () => {
  const rule = /\.dirty\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
  const bottom = /bottom:\s*(-?[\d.]+)px/.exec(rule)?.[1];
  const height = /height:\s*([\d.]+)px/.exec(rule)?.[1];
  assert.ok(bottom && height, '.dirty 要有 bottom 和 height');
  assert.ok(
    Math.abs(Number(bottom)) >= Number(height),
    `点要整个落在按钮外：bottom ${bottom}px，点高 ${height}px`,
  );
});

test('节点标题和「起点」标签挨在一起', () => {
  // 组件本身要 React Flow 的节点上下文才能渲染，这里只锁定布局约定：
  // 标题和标签装在同一个 flex 容器里，由这个容器去占满右边。
  assert.match(css, /\.node__heading\s*\{[^}]*display:\s*flex/);
  assert.match(css, /\.node__heading\s*\{[^}]*flex:\s*1/);
  assert.match(css, /\.node__title\s*\{[^}]*text-overflow:\s*ellipsis/);
  assert.doesNotMatch(
    css,
    /\.node__title\s*\{[^}]*flex:\s*1/,
    '标题自己不该再撑满 —— 那样「起点」又被顶到右边去了',
  );
});

test('只有一款自带字体', () => {
  assert.match(css, /@font-face/);
  assert.equal((css.match(/@font-face/g) ?? []).length, 1, '自带字体只留一款');
  assert.match(css, /--font-mono:/);
  assert.doesNotMatch(css, /--font-figure/, '不该再有第二款西文字体');
});

test('主题是浅色，而且只有蓝一个强调色', () => {
  assert.match(css, /color-scheme:\s*light/);
  assert.match(css, /--canvas:\s*#f/i, '画布应该是浅色');
  // 旧的暖金色主题不该有残留
  assert.doesNotMatch(css, /--gold/);
  assert.doesNotMatch(css, /--paper/);
});
