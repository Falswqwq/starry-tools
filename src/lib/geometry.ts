/**
 * 刀光的几何：线段相交与"沿折线第一处相交在哪"。
 *
 * 刻意写成不碰 DOM 的纯函数 —— 判断哪几条连线该被切断，是这件事里唯一会出错的
 * 部分，值得单独测。
 */

export type Point = { x: number; y: number };

export type PolylineHit = {
  /** 相交点 */
  point: Point;
  /** 从折线起点量到相交点的距离 */
  distance: number;
};

const orient = (p: Point, q: Point, r: Point) =>
  (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);

const within = (p: Point, q: Point, r: Point) =>
  Math.min(p.x, q.x) <= r.x &&
  r.x <= Math.max(p.x, q.x) &&
  Math.min(p.y, q.y) <= r.y &&
  r.y <= Math.max(p.y, q.y);

/** 两条线段是否相交（允许端点搭在另一条上）。 */
export function segmentsIntersect(a: Point, b: Point, c: Point, d: Point): boolean {
  const s1 = Math.sign(orient(a, b, c));
  const s2 = Math.sign(orient(a, b, d));
  const s3 = Math.sign(orient(c, d, a));
  const s4 = Math.sign(orient(c, d, b));

  // 严格相交：两端各在对方两侧。
  if (s1 * s2 < 0 && s3 * s4 < 0) return true;

  // 退化：某个端点正好落在线段上。
  return (
    (s1 === 0 && within(a, b, c)) ||
    (s2 === 0 && within(a, b, d)) ||
    (s3 === 0 && within(c, d, a)) ||
    (s4 === 0 && within(c, d, b))
  );
}

const distance = (p: Point, q: Point) => Math.hypot(q.x - p.x, q.y - p.y);

/** 两条线段的交点；平行或共线时返回 null（那种情况按端点处理就够了）。 */
function intersectionPoint(a: Point, b: Point, c: Point, d: Point): Point | null {
  const r = { x: b.x - a.x, y: b.y - a.y };
  const s = { x: d.x - c.x, y: d.y - c.y };
  const denominator = r.x * s.y - r.y * s.x;
  if (denominator === 0) return null;

  const t = ((c.x - a.x) * s.y - (c.y - a.y) * s.x) / denominator;
  return { x: a.x + t * r.x, y: a.y + t * r.y };
}

/**
 * 折线与线段的第一处相交。
 *
 * 连线在画布上是贝塞尔曲线，采样成折线之后用它来判交；返回的距离用来决定
 * 「从哪一刀两断」，所以按折线上的累计长度算。
 */
export function firstIntersection(points: Point[], a: Point, b: Point): PolylineHit | null {
  let travelled = 0;

  for (let i = 1; i < points.length; i++) {
    const p = points[i - 1]!;
    const q = points[i]!;
    if (segmentsIntersect(a, b, p, q)) {
      const hit = intersectionPoint(a, b, p, q) ?? p;
      return { point: hit, distance: travelled + distance(p, hit) };
    }
    travelled += distance(p, q);
  }

  return null;
}

/** 沿方向把线段延长，用来做收刀时的「跟出」。 */
export function extendSegment(from: Point, to: Point, extra: number): Point {
  const length = distance(from, to);
  if (length === 0) return to;
  const scale = (length + extra) / length;
  return { x: from.x + (to.x - from.x) * scale, y: from.y + (to.y - from.y) * scale };
}
