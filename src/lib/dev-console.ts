/**
 * 开发期挂在 `window.starry` 上的几个小工具。
 *
 * 为什么需要它：这个应用的数据全来自 Rust 那边（工作流、节点元数据、静态检查结果），
 * 直接在浏览器里打开时这些 IPC 全都会失败 —— 画布在，但是空的，也就没法拿去用
 * Chrome 那些更好用的绘制工具查「为什么拖起来卡」。所以留了一条搬运的路：
 *
 * ```js
 * // 1. 在 Tauri 窗口里把图搭好（或者 starry.seed(80) 塞一批），等静态检查跑完
 * copy(starry.grab())          // 整张图进了剪贴板
 * // 2. 在浏览器里
 * starry.load('粘进来')         // 画布和 Tauri 里一模一样
 * ```
 *
 * 只在开发构建里装（`main.tsx` 里用 `import.meta.env.DEV` 包着），生产构建会被摇掉。
 */

import { api } from './api';
import { patchFromJson, snapshotOf, snapshotToJson } from './dev-snapshot';
import type { ToolNode } from './types';
import { useStore } from '../state/store';

/** 撒节点时的摆放间距，够宽不重叠就行。 */
const CELL = { x: 300, y: 280, columns: 8 };

const tools = {
  /** 逃生口：直接拿整个 store 或命令包装。 */
  store: useStore,
  api,

  /**
   * 把当前这张图整份拷出来（返回 JSON 字符串，配合剪贴板用）。
   *
   * 带上静态检查结果是因为卡片上的端口那一行是它画出来的 —— 少了它，搬过去的卡片
   * 会明显比真的轻，拿来做绘制性能对比就不准了。
   */
  grab(): string {
    return snapshotToJson(snapshotOf(useStore.getState()));
  },

  /** 装上 `grab()` 拿到的 JSON。 */
  load(json: string): void {
    const patch = patchFromJson(json);
    useStore.setState(patch);
    const kinds = patch.kinds as unknown[];
    const nodes = patch.nodes as unknown[];
    console.info(`已装载 ${kinds.length} 种节点元数据、${nodes.length} 个节点`);
  },

  /**
   * 现场塞一批节点，用真实的元数据和默认参数。
   *
   * 会**替换**掉画布上现有的东西（不是追加），这样每次复现出来的图都一样。
   * 在 Tauri 里跑完之后会自动请后端重算一次静态检查；在浏览器里那次调用会失败，
   * 只留下一条状态提示，画布照常 —— 代价是卡片会少画一行端口。
   */
  seed(count = 40): void {
    const state = useStore.getState();
    const kinds = Object.values(state.kindById);
    if (kinds.length === 0) {
      console.warn('还没有节点元数据，先在 Tauri 里 starry.grab() 再到这里 starry.load(...)');
      return;
    }

    const nodes: ToolNode[] = Array.from({ length: count }, (_, index) => {
      const kind = kinds[index % kinds.length];
      return {
        id: `seed-${index}`,
        type: 'tool',
        position: {
          x: (index % CELL.columns) * CELL.x,
          y: Math.floor(index / CELL.columns) * CELL.y,
        },
        data: { kind: kind.id, params: { ...kind.defaults } },
      };
    });

    useStore.setState({ nodes, edges: [], selectedId: null, resolved: null });
    void state.resolveNow();
    console.info(`已铺开 ${count} 个节点`);
  },
};

/** 装到 `window.starry` 上。只在开发构建里调。 */
export function installDevConsole(): void {
  (globalThis as { starry?: typeof tools }).starry = tools;
}
