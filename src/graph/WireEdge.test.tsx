/**
 * 连线渲染的回归测试。
 *
 * 盯的是一个只在屏幕上看得出来、看代码看不出来的毛病：**开放的 SVG 路径默认会被
 * 填成黑色**。一条贝塞尔连线被隐式闭合之后，曲线和弦围出来的那块就被填实了 ——
 * 断开动画里那两条自己画的路径就这么黑过一次。
 */

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';

import { SeveredWire } from './WireEdge';

const css = readFileSync('src/styles/app.css', 'utf8');

const severing = {
  edgeId: 'e1',
  point: { x: 12, y: 34 },
  distance: 10,
  length: 60,
  progress: 0.3,
};

const bezier = 'M0 0 C20 -30 40 30 60 0';

test('断开的连线不能有填充', () => {
  const html = renderToStaticMarkup(<SeveredWire path={bezier} severing={severing} />);

  const paths = [...html.matchAll(/<path[^>]*>/g)].map((match) => match[0]);
  assert.equal(paths.length, 2, '应该是断成两截');

  for (const path of paths) {
    assert.match(path, /fill="none"/, `这条路径没写 fill="none"，会被填黑：${path}`);
  }
});

test('.wire 这条规则本身也有 fill:none', () => {
  // 双保险：JSX 里写了属性，类上也得有 —— 免得以后又有人画一条裸路径忘了写。
  assert.match(
    css,
    /\.wire\s*\{[^}]*fill:\s*none/,
    'app.css 里的 .wire 少了 fill:none，开放路径会被默认填黑',
  );
});

test('断成两截用的是同一条路径，只是各画一段', () => {
  const html = renderToStaticMarkup(<SeveredWire path={bezier} severing={severing} />);
  for (const match of html.matchAll(/d="([^"]+)"/g)) {
    assert.equal(match[1], bezier);
  }
});

test('两截的 dasharray / dashoffset 是算出来的', () => {
  const half = renderToStaticMarkup(
    <SeveredWire path={bezier} severing={{ ...severing, distance: 10, length: 60, progress: 0 }} />,
  );
  // progress=0：左半截画满 [0,10]，右半截画满 [10,60]
  assert.match(half, /stroke-dasharray:10 61/);
  assert.match(half, /stroke-dasharray:50 61/);
  assert.match(half, /stroke-dashoffset:-10/);
});

test('progress 推到头时两截都缩没了', () => {
  const done = renderToStaticMarkup(
    <SeveredWire path={bezier} severing={{ ...severing, distance: 10, length: 60, progress: 1 }} />,
  );
  // 左半截 [0,0]、右半截 [60,60]，都是零长度
  assert.match(done, /stroke-dasharray:0 61/);
  assert.match(done, /stroke-dashoffset:-60/);
});
