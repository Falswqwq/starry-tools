/**
 * 连线。
 *
 * 三种状态：
 *   * 平时 / 被刀光扫到（`is-doomed`，会亮起来表示「将被删除」）；
 *   * 正在断开（`Severing`）—— 断口两侧各自缩回去，断口闪一下；
 *   * 跑工作流时信号流过去（`wire__pulse`）。
 */

import { BaseEdge, getBezierPath, type EdgeProps } from '@xyflow/react';
import { memo } from 'react';

import { countRender } from '../lib/dev-render';
import type { Severing } from '../state/store';
import { useStore } from '../state/store';

export const WireEdge = memo(function WireEdge({
  id,
  source,
  selected,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
}: EdgeProps) {
  countRender('连线');
  const [path] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  });

  const invalid = useStore((state) =>
    (state.resolved?.issues ?? []).some(
      (issue) => issue.edgeId === id && issue.severity === 'error',
    ),
  );
  const doomed = useStore((state) => state.slash?.doomed.includes(id) ?? false);
  const severing = useStore((state) => state.severed.find((entry) => entry.edgeId === id));
  const pulse = useStore((state) => state.pulse);

  const sourceIndex = pulse ? pulse.order.indexOf(source) : -1;
  const flowing = pulse !== null && sourceIndex >= 0 && sourceIndex < pulse.step;

  if (severing) return <SeveredWire path={path} severing={severing} />;

  return (
    <>
      {doomed && <BaseEdge path={path} className="wire wire--doomed-glow" />}
      <BaseEdge
        id={id}
        path={path}
        className={[
          'wire',
          invalid ? 'is-invalid' : '',
          doomed ? 'is-doomed' : '',
          selected ? 'is-selected' : '',
        ]
          .filter(Boolean)
          .join(' ')}
      />
      {flowing && <path className="wire__pulse" d={path} />}
    </>
  );
});

/**
 * 正在断开的连线。
 *
 * `stroke-dasharray` / `stroke-dashoffset` 用来只画出路径的一段：
 * 画 [from, to] 就是 dasharray 写 `to - from`、dashoffset 写 `-from`。
 * 两截分别从断口往两端缩，看起来就是被一刀两断后各自弹回去。
 */
export function SeveredWire({ path, severing }: { path: string; severing: Severing }) {
  const { distance, length, progress, point } = severing;
  const eased = 1 - (1 - progress) ** 2;

  const leftEnd = Math.max(distance * (1 - eased), 0);
  const rightStart = Math.min(distance + (length - distance) * eased, length);
  const fade = 1 - Math.max(0, (progress - 0.5) / 0.5);

  return (
    <g className="severing" style={{ opacity: fade }}>
      {/* fill="none" 不能省：开放的贝塞尔会被隐式闭合，默认填黑。
          `.wire` 那条规则里也有，这里再写一遍是为了不依赖 class 还在不在。 */}
      <path
        className="wire wire--severing"
        d={path}
        fill="none"
        style={{ strokeDasharray: `${leftEnd} ${length + 1}`, strokeDashoffset: 0 }}
      />
      <path
        className="wire wire--severing"
        d={path}
        fill="none"
        style={{
          strokeDasharray: `${Math.max(length - rightStart, 0)} ${length + 1}`,
          strokeDashoffset: -rightStart,
        }}
      />
      {/* 断口的那一下光 */}
      <circle
        className="severing__spark"
        cx={point.x}
        cy={point.y}
        r={3 + eased * 20}
        style={{ opacity: Math.max(0, 1 - progress * 2.4) }}
      />
    </g>
  );
}
