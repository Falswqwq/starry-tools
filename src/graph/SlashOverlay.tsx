/**
 * 刀光本身。
 *
 * 画在画布之上的一个覆盖层里（屏幕坐标），不随画布缩放位移 —— 刀口要始终锐利。
 * 收刀时向前跟出一段再消散，这就是「划出去」的那一下。
 */

import { createPortal } from 'react-dom';

import { extendSegment, type Point } from '../lib/geometry';
import { useStore } from '../state/store';

/** 收刀时刀尖再往前送多远 */
const OVERTAKE_REACH = 160;

const easeOutCubic = (t: number) => 1 - (1 - t) ** 3;

export function SlashOverlay() {
  const slash = useStore((state) => state.slash);
  const doomed = useStore((state) => state.slash?.doomed.length ?? 0);

  if (!slash) return null;

  const overtake = easeOutCubic(slash.overtake);
  const tip: Point = extendSegment(slash.from, slash.to, OVERTAKE_REACH * overtake);
  // 跟出的时候同时淡下去、变细，像力道用完了。
  const fade = 1 - overtake ** 1.6;
  const scale = 1 - overtake * 0.45;
  const hit = doomed > 0;

  return createPortal(
    <svg className="slash" aria-hidden="true">
      <defs>
        {/* 尾巴透明、刀尖最亮，看起来才像在动 */}
        <linearGradient
          id="slash-fade"
          gradientUnits="userSpaceOnUse"
          x1={slash.from.x}
          y1={slash.from.y}
          x2={tip.x}
          y2={tip.y}
        >
          <stop offset="0" className="slash__tail" />
          <stop offset="0.45" className="slash__mid" />
          <stop offset="1" className="slash__tip" />
        </linearGradient>
      </defs>

      <g style={{ opacity: fade }}>
        <line
          className="slash__glow"
          x1={slash.from.x}
          y1={slash.from.y}
          x2={tip.x}
          y2={tip.y}
          strokeWidth={16 * scale}
        />
        <line
          className="slash__core"
          x1={slash.from.x}
          y1={slash.from.y}
          x2={tip.x}
          y2={tip.y}
          strokeWidth={3.5 * scale}
          stroke={hit ? 'url(#slash-fade)' : undefined}
        />
        <circle className="slash__tip-dot" cx={tip.x} cy={tip.y} r={(hit ? 4.5 : 3) * scale} />
      </g>
    </svg>,
    document.body,
  );
}
