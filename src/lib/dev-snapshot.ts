/**
 * 在 Tauri 窗口和浏览器之间搬「当前这张图」。
 *
 * 只给开发期用：浏览器里没有 Rust 那半边，工作流、节点元数据、静态检查结果全都拿不到，
 * 所以只能在 Tauri 里把图搭好、整份搬过去。搬运的内容就这几样，形状由
 * `src-tauri/src/commands.rs` 和 `src/lib/types.ts` 定。
 */

import type { WorkflowMeta } from './graph';
import type { NodeKind, ResolvedWorkflow, ToolNode, WireEdge } from './types';

export type GraphSnapshot = {
  meta: WorkflowMeta;
  nodes: ToolNode[];
  edges: WireEdge[];
  kinds: NodeKind[];
  /** 后端的静态检查结果。有它卡片才会画端口那一行，所以一定要带上。 */
  resolved: ResolvedWorkflow | null;
};

/** 从编辑器状态里取出一份快照。 */
export function snapshotOf(source: GraphSnapshot): GraphSnapshot {
  return {
    meta: source.meta,
    nodes: source.nodes,
    edges: source.edges,
    kinds: source.kinds,
    resolved: source.resolved,
  };
}

export function snapshotToJson(snapshot: GraphSnapshot): string {
  return JSON.stringify(snapshot);
}

/**
 * 把一份快照变成可以直接丢给 `store.setState` 的补丁。
 *
 * `kindById` 是从 `kinds` 推出来的，不是快照里存的 —— 少存一份就不会两边不一致。
 */
export function patchFromJson(json: string): Record<string, unknown> {
  const snapshot = JSON.parse(json) as Partial<GraphSnapshot>;
  if (!Array.isArray(snapshot.kinds) || !Array.isArray(snapshot.nodes)) {
    throw new Error('这不是一份 starry.grab() 出来的快照');
  }
  return {
    meta: snapshot.meta,
    nodes: snapshot.nodes,
    edges: snapshot.edges ?? [],
    kinds: snapshot.kinds,
    kindById: Object.fromEntries(snapshot.kinds.map((kind) => [kind.id, kind])),
    resolved: snapshot.resolved ?? null,
    ready: true,
    dirty: false,
  };
}
