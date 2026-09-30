/** 后端命令的类型化包装。命令名和 `src-tauri/src/commands.rs` 一一对应。 */

import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';

import type {
  AppInfo,
  ImageInfo,
  NodeKind,
  ResolvedWorkflow,
  RunReport,
  Workflow,
  WorkflowSummary,
} from './types';

export const api = {
  nodeKinds: () => invoke<NodeKind[]>('node_kinds'),
  createWorkflow: (name?: string) => invoke<Workflow>('create_workflow', { name: name ?? null }),
  resolveWorkflow: (workflow: Workflow) =>
    invoke<ResolvedWorkflow>('resolve_workflow', { workflow }),
  runWorkflow: (workflow: Workflow, onlyNode?: string) =>
    invoke<RunReport>('run_workflow', { workflow, onlyNode: onlyNode ?? null }),
  listWorkflows: () => invoke<WorkflowSummary[]>('list_workflows'),
  loadWorkflow: (id: string) => invoke<Workflow>('load_workflow', { id }),
  saveWorkflow: (workflow: Workflow) => invoke<Workflow>('save_workflow', { workflow }),
  deleteWorkflow: (id: string) => invoke<void>('delete_workflow', { id }),
  inspectImage: (path: string) => invoke<ImageInfo>('inspect_image', { path }),
  exportFile: (source: string, destination: string) =>
    invoke<string>('export_file', { source, destination }),
  revealPath: (path: string) => invoke<void>('reveal_path', { path }),
  appInfo: () => invoke<AppInfo>('app_info'),
};

/** 打开系统的文件（或目录）选择框，返回绝对路径。 */
export async function pickFile(
  extensions: string[],
  title: string,
  directory = false,
): Promise<string | null> {
  const picked = await open({
    multiple: false,
    directory,
    title,
    // 选目录时给格式过滤没有意义，还会把目录灰掉。
    ...(directory ? {} : { filters: [{ name: '图像', extensions }] }),
  });
  return typeof picked === 'string' ? picked : null;
}

/** 打开系统的另存为对话框，返回目标路径。 */
export async function pickDestination(defaultName: string): Promise<string | null> {
  const picked = await save({ defaultPath: defaultName });
  return picked ?? null;
}

/** 把后端抛回来的错误整理成一句能显示的话。 */
export function messageOf(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
