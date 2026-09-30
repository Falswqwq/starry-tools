/**
 * 与 Rust 侧一一对应的数据结构。
 *
 * 这些形状由 `src-tauri/src/engine/tests.rs` 里的 `kind_metadata_is_a_stable_contract`
 * 和 `src-tauri/src/model/port_type.rs` 里的 `json_shape_is_stable` 守着，
 * 改 rust 那边的话那两处测试会先红。
 */

export type ImageFormat =
  | 'any'
  | 'png'
  | 'jpeg'
  | 'gif'
  | 'webp'
  | 'bmp'
  | 'tiff'
  | 'ico'
  | 'qoi'
  | 'tga'
  | 'pnm';

/**
 * 端口的类型。 `Image(Png)` 在 JSON 里是 `{ image: 'png' }`；
 * `Any` 是通配，什么都能接，序列化成字符串 `'any'`。
 */
export type PortType = 'any' | 'text' | 'number' | 'bool' | { image: ImageFormat };

export type PortDef = {
  id: string;
  label: string;
  ty: PortType;
  required: boolean;
  hint?: string;
};

export type SelectOption = { value: string; label: string; hint?: string };

/** 参数的控件种类，决定渲染成什么。 */
export type ParamControl = 'number' | 'slider' | 'text' | 'select' | 'bool' | 'file';

/** 一个参数在什么条件下才显示。`allOf` 里的每一条都要同时满足。 */
export type VisibleWhen = {
  param: string;
  anyOf: string[];
  allOf?: VisibleWhen[];
};

export type ParamDef = {
  id: string;
  label: string;
  description?: string;
  /** 只有跟着这个参数取值变化才显示。 */
  visibleWhen?: VisibleWhen;
  control: ParamControl;
  default?: number | string | boolean;
  /* number */
  min?: number;
  max?: number;
  step?: number;
  integer?: boolean;
  unit?: string;
  /* slider 与 number 共用上面这几个字段 */
  /* text */
  multiline?: boolean;
  placeholder?: string;
  /* select */
  options?: SelectOption[];
  /* file */
  dialogTitle?: string;
  extensions?: string[];
  /** 为真时选的是一个目录，而不是文件。 */
  directory?: boolean;
};

export type NodeKind = {
  id: string;
  name: string;
  category: string;
  description: string;
  isSource: boolean;
  inputs: PortDef[];
  outputs: PortDef[];
  params: ParamDef[];
  /** 展开工具卡片时列的「注意事项」。 */
  notes: string[];
  /** 新建节点时的初始参数，由后端从参数声明里推出来。 */
  defaults: Record<string, unknown>;
};

export type Params = Record<string, unknown>;

export type Position = { x: number; y: number };

export type NodeInstance = {
  id: string;
  kind: string;
  position: Position;
  params: Params;
};

export type WorkflowEdge = {
  id: string;
  source: string;
  sourcePort: string;
  target: string;
  targetPort: string;
};

export type Workflow = {
  id: string;
  name: string;
  description: string;
  version: number;
  nodes: NodeInstance[];
  edges: WorkflowEdge[];
  createdAt: number | null;
  updatedAt: number | null;
};

export type WorkflowSummary = {
  id: string;
  name: string;
  description: string;
  nodeCount: number;
  edgeCount: number;
  createdAt: number | null;
  updatedAt: number | null;
};

export type Severity = 'error' | 'warning';

export type Issue = {
  severity: Severity;
  message: string;
  nodeId?: string;
  portId?: string;
  edgeId?: string;
};

export type ResolvedNode = {
  nodeId: string;
  kind: string;
  isSource: boolean;
  inputs: PortDef[];
  outputs: PortDef[];
};

export type ResolvedWorkflow = {
  nodes: ResolvedNode[];
  issues: Issue[];
  runnable: boolean;
};

export type NodeStatus = 'ok' | 'failed' | 'skipped';

export type PortResult = {
  portId: string;
  label: string;
  ty: PortType;
  summary: string;
  preview?: string;
  path?: string;
};

export type NodeRunResult = {
  nodeId: string;
  kind: string;
  name: string;
  status: NodeStatus;
  error?: string;
  warnings: string[];
  elapsedMs: number;
  outputs: PortResult[];
};

export type RunReport = {
  ok: boolean;
  durationMs: number;
  /** 实际执行顺序，用来放「信号流过图」的动画。 */
  order: string[];
  nodes: NodeRunResult[];
  issues: Issue[];
  outputDir?: string;
  finishedAt: number;
};

export type ImageInfo = {
  format: ImageFormat;
  width: number;
  height: number;
  fileName: string;
  fileSize: number;
  preview?: string;
};

export type AppInfo = {
  name: string;
  version: string;
  workflowDir: string;
  outputDir: string;
};

/** React Flow 画布上的节点。 */
export type ToolNodeData = {
  kind: string;
  params: Params;
  [key: string]: unknown;
};

export type ToolNode = import('@xyflow/react').Node<ToolNodeData, 'tool'>;
export type WireEdge = import('@xyflow/react').Edge;
