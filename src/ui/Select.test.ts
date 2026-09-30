/**
 * 下拉框的回归测试。
 *
 * 盯的是一个很隐蔽的布局坑：Radix 的 `ItemIndicator` 在**未选中**时只渲染 `null`。
 * 如果把它当成栅格的第一列（`grid-template-columns: 13px 1fr auto`），
 * 未选中项的 `ItemText` 就会被自动排进那个 13px 的窄列，文字一个一个字竖着排下来。
 * 表现是「只有选中项看着正常」，很容易被当成字体问题。
 *
 * 这里把结构钉住：对勾必须包在常驻的占位格里，而且布局不能用靠位置分列的办法。
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const css = readFileSync('src/styles/app.css', 'utf8');
const source = readFileSync('src/ui/Select.tsx', 'utf8');

test('下拉项用 flex 排版，不靠位置分列', () => {
  assert.match(
    css,
    /\.ui-menu__item\s*\{[^}]*display:\s*flex/,
    '下拉项必须是 flex —— 用 grid 按位置分列的话，某一格不渲染就会把文字挤变形',
  );
  assert.doesNotMatch(
    css,
    /\.ui-menu__item\s*\{[^}]*grid-template-columns/,
    '不要再退回 grid-template-columns 的写法',
  );
});

test('对勾的位子固定留着，不管这一项选没选中', () => {
  assert.match(
    css,
    /\.ui-menu__check\s*\{[^}]*width:\s*13px/,
    '对勾格子要有固定宽度',
  );
  assert.match(css, /\.ui-menu__check\s*\{[^}]*flex:\s*none/);

  // 占位格要常驻渲染，ItemIndicator 只是它里面的内容。
  assert.match(
    source,
    /<span className="ui-menu__check">[\s\S]{0,120}?<Primitive\.ItemIndicator>/,
    'ItemIndicator 必须包在常驻的 .ui-menu__check 里',
  );
  assert.doesNotMatch(
    source,
    /\{[^}]{0,40}&&\s*<span className="ui-menu__check">/,
    '占位格不能是条件渲染的 —— 那正是这个 bug 的成因',
  );
});

test('提示文字靠右，不挤占标签', () => {
  assert.match(css, /\.ui-menu__hint\s*\{[^}]*margin-left:\s*auto/);
});
