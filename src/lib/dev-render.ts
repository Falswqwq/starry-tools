/**
 * 组件渲染计数 —— 查「拖起来卡」时的第一件工具。
 *
 * 用法：在组件顶部调一次 `countRender('节点卡片')`，控制台就会每秒打一张表，写着每个
 * 标记这一秒渲染了多少次。数字合不合常理一眼就看得出来 —— 拖 100 个节点时
 * `节点卡片` 该是 0～2，而不该是 6000。
 *
 * 计数本身一直开着（就是一次 Map 自增）；**每秒自动打表**只在开发构建里跑，
 * 生产构建里那一块会被摇掉。
 *
 * 想临时静音：`starryRenders.watch = false`；
 * 想看此刻的数字：`starryRenders.report()`；清零：`starryRenders.reset()`。
 */

const counts = new Map<string, number>();

let timer: ReturnType<typeof setInterval> | undefined;

// Vite 会把这一句替成字面量 `false`（开发时是 `true`），生产构建里下面那块就整块
// 被摇掉了。测试用 esbuild 打包，那份 define 写在 package.json 的 test 脚本里。
const DEV = import.meta.env.DEV;

/** 记一次渲染。 */
export function countRender(tag: string): void {
  counts.set(tag, (counts.get(tag) ?? 0) + 1);
  if (!timer) start();
}

/** 把这一秒的计数打出来，然后清零。 */
export function report(): void {
  if (counts.size === 0) return;
  const rows = [...counts]
    .map(([标记, 渲染次数]) => ({ 标记, 每秒渲染: 渲染次数 }))
    .sort((left, right) => right.每秒渲染 - left.每秒渲染);
  counts.clear();
  console.table(rows);
}

export const renderStats = {
  /** 关掉每秒那张表。计数还在，`report()` 还能手动看。 */
  watch: true,
  report,
  reset(): void {
    counts.clear();
  },
};

function start(): void {
  if (!DEV) return;
  timer = setInterval(() => {
    if (renderStats.watch) report();
    else counts.clear();
  }, 1000);
  // 万一被 node 环境加载到，别因为这个定时器把进程挂住。
  (timer as { unref?: () => void }).unref?.();
  (globalThis as { starryRenders?: typeof renderStats }).starryRenders = renderStats;
}
