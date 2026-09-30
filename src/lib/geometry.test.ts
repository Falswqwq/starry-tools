/** 刀光几何的测试：切得准不准，全看这几个函数。 */

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { extendSegment, firstIntersection, segmentsIntersect } from './geometry';

const p = (x: number, y: number) => ({ x, y });

test('两条线段交叉', () => {
  assert.equal(segmentsIntersect(p(0, 0), p(10, 10), p(0, 10), p(10, 0)), true);
});

test('平移开就不相交', () => {
  assert.equal(segmentsIntersect(p(0, 0), p(10, 10), p(20, 0), p(30, 10)), false);
});

test('线段延长后才相交，不算相交', () => {
  // 竖线在 x=20，横线只到 x=10
  assert.equal(segmentsIntersect(p(0, 5), p(10, 5), p(20, 0), p(20, 10)), false);
});

test('端点搭在另一条线段上算相交', () => {
  assert.equal(segmentsIntersect(p(0, 0), p(10, 0), p(10, 0), p(10, 10)), true);
  // 但搭在延长线上不算
  assert.equal(segmentsIntersect(p(0, 0), p(10, 0), p(20, 0), p(20, 10)), false);
});

test('共线重叠算相交', () => {
  assert.equal(segmentsIntersect(p(0, 0), p(10, 0), p(5, 0), p(15, 0)), true);
  assert.equal(segmentsIntersect(p(0, 0), p(10, 0), p(11, 0), p(15, 0)), false);
});

test('平行不相交', () => {
  assert.equal(segmentsIntersect(p(0, 0), p(10, 0), p(0, 5), p(10, 5)), false);
});

test('折线：给出第一处相交的位置和沿线距离', () => {
  // 一条从 (0,0) 到 (0,100) 的折线，被一条水平线在 y=30 处切到
  const polyline = [p(0, 0), p(0, 30), p(0, 100)];
  const hit = firstIntersection(polyline, p(-10, 30), p(10, 30));
  assert.ok(hit);
  assert.deepEqual(hit.point, p(0, 30));
  assert.equal(hit.distance, 30);
});

test('折线：没被切到就返回 null', () => {
  const polyline = [p(0, 0), p(0, 100)];
  assert.equal(firstIntersection(polyline, p(50, 0), p(50, 100)), null);
});

test('折线：切在中间某一段时，距离要把前面几段算进去', () => {
  // 三段：长度 10、10、10，在第三段的中点被切
  const polyline = [p(0, 0), p(10, 0), p(20, 0), p(30, 0)];
  const hit = firstIntersection(polyline, p(25, -5), p(25, 5));
  assert.ok(hit);
  assert.deepEqual(hit.point, p(25, 0));
  assert.equal(hit.distance, 25);
});

test('折线：被切到多次时只报最靠前的那处', () => {
  const polyline = [p(0, 0), p(10, 0), p(20, 0), p(30, 0)];
  // 这条斜线从 (5,-5) 划到 (25,5)，穿过 x 轴的位置是 t=0.5，也就是 x=15
  const hit = firstIntersection(polyline, p(5, -5), p(25, 5));
  assert.ok(hit);
  assert.deepEqual(hit.point, p(15, 0));
  assert.equal(hit.distance, 15);
});

test('折线：被切成几段时，报的是从起点数过来最近的那处', () => {
  // 一条 v 字形的线，用横线切，第一个交点在左边那条腿上
  const polyline = [p(0, 100), p(20, 0), p(40, 100)];
  const hit = firstIntersection(polyline, p(-10, 50), p(50, 50));
  assert.ok(hit);
  assert.deepEqual(hit.point, p(10, 50));
});

test('收刀时沿方向延长', () => {
  const extended = extendSegment(p(0, 0), p(10, 0), 20);
  assert.deepEqual(extended, p(30, 0));
  // 零长度的线段不该算出 NaN
  assert.deepEqual(extendSegment(p(3, 4), p(3, 4), 10), p(3, 4));
});
