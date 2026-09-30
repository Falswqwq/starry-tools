/** 从画布上读出每条连线的实际渲染路径 —— 判交要拿真曲线，不是自己再算一遍。 */

import type { Point } from '../lib/geometry';

export type EdgePath = {
  id: string;
  /** 采样成折线的点，flow 坐标 */
  points: Point[];
  /** 路径总长，断裂动画要用 */
  length: number;
};

/**
 * 把当前画布上每条连线采样成折线。
 *
 * 连线在拖动过程中不会变，所以一次刀光只采样一次就够了。
 * 坐标是 path 自身的用户坐标，也就是 flow 坐标系（视图缩放挂在更外层），
 * 所以调用方把刀光两端换算成 flow 坐标再拿来比较。
 */
export function sampleEdges(): EdgePath[] {
  const sampled: EdgePath[] = [];

  for (const element of document.querySelectorAll<SVGElement>('.react-flow__edge[data-id]')) {
    const id = element.getAttribute('data-id');
    const path = element.querySelector<SVGPathElement>('path.react-flow__edge-path');
    if (!id || !path) continue;

    let length = 0;
    try {
      length = path.getTotalLength();
    } catch {
      continue;
    }
    if (!Number.isFinite(length) || length <= 0) continue;

    // 曲率大的地方密一点，但别为了几像素的精度采样上千次。
    const steps = Math.max(8, Math.min(64, Math.ceil(length / 8)));
    const points: Point[] = [];
    for (let i = 0; i <= steps; i++) {
      const at = path.getPointAtLength((length * i) / steps);
      points.push({ x: at.x, y: at.y });
    }

    sampled.push({ id, points, length });
  }

  return sampled;
}
