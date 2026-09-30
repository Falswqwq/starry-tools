/**
 * 参数可见性。
 *
 * 后端在参数上写 `visibleWhen`，说清楚「只在别的参数取某几个值时才显示」。
 * 判断放在这里，是因为节点卡片上要**立刻**跟着变，不能等一次 IPC 往返。
 */

import type { ParamDef, VisibleWhen } from './types';

/** 这条条件（以及它挂着的所有「而且」）成不成立。 */
function matches(when: VisibleWhen, params: Record<string, unknown>): boolean {
  const current = String(params[when.param] ?? '');
  if (!when.anyOf.includes(current)) return false;
  return (when.allOf ?? []).every((nested) => matches(nested, params));
}

/** 这个参数在当前取值下该不该显示。没有条件就一律显示。 */
export function isVisible(def: ParamDef, params: Record<string, unknown>): boolean {
  return def.visibleWhen ? matches(def.visibleWhen, params) : true;
}
