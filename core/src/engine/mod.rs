//! 工作流的检查与执行。
//!
//! 这里做两件事：
//!
//! 1. [`resolve`] —— 静态检查。把每个节点的端口类型算出来（输入节点要读文件
//!    头才知道格式），顺便列出所有问题。编辑器每改一下就调它一次，用来画
//!    端口徽标、判断连线能不能接、给节点打红标。
//! 2. [`run`] —— 真的把图画一遍。按拓扑序逐个执行节点，运行期再严格核对一遍
//!    类型，把每个节点的产物（预览 + 落盘文件）收集成报告。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::error::AppError;
use crate::interaction::Interaction;
use crate::model::node_kind::{PortDef, PARAM_PORT_PREFIX};
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::model::workflow::{now_millis, Workflow};
use crate::progress::Progress;
use crate::registry::{registry, Registry};

/// 节点缩略图的长边上限。
const PREVIEW_MAX_DIM: u32 = 256;
/// 一次运行最多生成多少张缩略图，免得报告大得离谱。
const MAX_PREVIEWS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_id: Option<String>,
}

impl Issue {
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            node_id: None,
            port_id: None,
            edge_id: None,
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            ..Issue::error("")
        }
    }

    fn at_node(mut self, node_id: &str) -> Self {
        self.node_id = Some(node_id.to_string());
        self
    }

    fn at_port(mut self, node_id: &str, port_id: &str) -> Self {
        self.node_id = Some(node_id.to_string());
        self.port_id = Some(port_id.to_string());
        self
    }

    fn at_edge(mut self, edge_id: &str) -> Self {
        self.edge_id = Some(edge_id.to_string());
        self
    }
}

/// 一个节点在给定参数下的真实端口。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedNode {
    pub node_id: String,
    pub kind: String,
    pub is_source: bool,
    pub inputs: Vec<PortDef>,
    pub outputs: Vec<PortDef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedWorkflow {
    pub nodes: Vec<ResolvedNode>,
    pub issues: Vec<Issue>,
    /// 没有任何 error 级问题时才算能跑。
    pub runnable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    Ok,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortResult {
    pub port_id: String,
    pub label: String,
    pub ty: PortType,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 输出是一段色板文本（hex 一行一个）时，把颜色拆好放着 ——
    /// 界面据它画一排小色块。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palette: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeRunResult {
    pub node_id: String,
    pub kind: String,
    pub name: String,
    pub status: NodeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
    pub outputs: Vec<PortResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub ok: bool,
    pub duration_ms: u64,
    /// 实际执行的顺序，前端拿它放「信号流过图」的动画。
    pub order: Vec<String>,
    pub nodes: Vec<NodeRunResult>,
    pub issues: Vec<Issue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    pub finished_at: i64,
}

// ---------------------------------------------------------------------------
// 静态检查
// ---------------------------------------------------------------------------

struct Analysis {
    resolved: Vec<ResolvedNode>,
    index_of: HashMap<String, usize>,
    issues: Vec<Issue>,
    order: Option<Vec<String>>,
}

impl Analysis {
    fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == Severity::Error)
    }

    fn node(&self, node_id: &str) -> Option<&ResolvedNode> {
        self.index_of
            .get(node_id)
            .map(|index| &self.resolved[*index])
    }
}

fn port_type_map(ports: &[PortDef]) -> HashMap<String, PortType> {
    ports
        .iter()
        .map(|port| (port.id.clone(), port.ty))
        .collect()
}

fn analyze(registry: &Registry, workflow: &Workflow) -> Analysis {
    let mut issues = Vec::new();
    let mut resolved: Vec<ResolvedNode> = Vec::with_capacity(workflow.nodes.len());
    let mut index_of: HashMap<String, usize> = HashMap::new();
    let mut output_types: HashMap<String, HashMap<String, PortType>> = HashMap::new();

    let mut seen_ids: HashSet<&str> = HashSet::new();
    for node in &workflow.nodes {
        if !seen_ids.insert(node.id.as_str()) {
            issues.push(Issue::error(format!("节点 id 重复：{}", node.id)).at_node(&node.id));
            continue;
        }

        let definition = match registry.get(&node.kind) {
            Some(spec) => ResolvedNode {
                node_id: node.id.clone(),
                // 用归一后的 id：老存档里写的是老 id，前端要拿它去查元数据。
                kind: spec.kind.id.clone(),
                is_source: spec.kind.is_source,
                // 含**参数端口**（每个能被上游喂的参数各一个）——
                // 它们也是真正的输入端口，连线、类型检查都走同一套。
                inputs: spec.kind.inputs_for(&node.params),
                outputs: spec.outputs_for(&node.params),
            },
            None => {
                issues.push(
                    Issue::error(format!("未知的节点类型「{}」", node.kind)).at_node(&node.id),
                );
                ResolvedNode {
                    node_id: node.id.clone(),
                    kind: node.kind.clone(),
                    is_source: false,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                }
            }
        };

        output_types.insert(node.id.clone(), port_type_map(&definition.outputs));
        index_of.insert(node.id.clone(), resolved.len());
        resolved.push(definition);
    }

    // 输入端口声明：node_id -> (port_id -> 端口定义)
    let mut declared_inputs: HashMap<&str, &Vec<PortDef>> = HashMap::new();
    for node in &workflow.nodes {
        if let Some(entry) = index_of.get(&node.id).map(|index| &resolved[*index].inputs) {
            declared_inputs.insert(node.id.as_str(), entry);
        }
    }

    let mut filled: HashSet<(String, String)> = HashSet::new();
    for edge in &workflow.edges {
        let Some(source_ports) = output_types.get(&edge.source) else {
            issues.push(
                Issue::error(format!("连线的起点节点不存在：{}", edge.source)).at_edge(&edge.id),
            );
            continue;
        };
        let Some(source_type) = source_ports.get(&edge.source_port).copied() else {
            issues.push(
                Issue::error(format!(
                    "节点 {} 上没有名为「{}」的输出端口",
                    edge.source, edge.source_port
                ))
                .at_edge(&edge.id),
            );
            continue;
        };
        let Some(target_inputs) = declared_inputs.get(edge.target.as_str()) else {
            issues.push(
                Issue::error(format!("连线的终点节点不存在：{}", edge.target)).at_edge(&edge.id),
            );
            continue;
        };
        let Some(target_port) = target_inputs
            .iter()
            .find(|port| port.id == edge.target_port)
        else {
            issues.push(
                Issue::error(format!(
                    "节点 {} 上没有名为「{}」的输入端口",
                    edge.target, edge.target_port
                ))
                .at_edge(&edge.id),
            );
            continue;
        };

        if !filled.insert((edge.target.clone(), edge.target_port.clone())) {
            issues.push(
                Issue::error(format!("输入端口「{}」连接了多条线", target_port.label))
                    .at_edge(&edge.id),
            );
        }

        if !target_port.ty.accepts(source_type) {
            issues.push(
                Issue::error(format!(
                    "类型不匹配：「{}」需要{}，而连过来的是{}",
                    target_port.label,
                    target_port.ty.label(),
                    source_type.label()
                ))
                .at_edge(&edge.id)
                .at_port(&edge.target, &edge.target_port),
            );
        }
    }

    for node in &workflow.nodes {
        let Some(spec) = registry.get(&node.kind) else {
            continue;
        };
        for port in &spec.kind.inputs {
            if port.required && !filled.contains(&(node.id.clone(), port.id.clone())) {
                issues.push(
                    Issue::error(format!("必填的输入「{}」还没有连上", port.label))
                        .at_port(&node.id, &port.id),
                );
            }
        }
        if !spec.kind.inputs.is_empty() && workflow.incoming(&node.id).next().is_none() {
            issues.push(
                Issue::warning(format!("「{}」还没有连接任何输入", spec.kind.name))
                    .at_node(&node.id),
            );
        }
    }

    let order = topological_order(workflow, &mut issues);

    Analysis {
        resolved,
        index_of,
        issues,
        order,
    }
}

/// Kahn 算法。有环就返回 `None` 并记一条错误。
fn topological_order(workflow: &Workflow, issues: &mut Vec<Issue>) -> Option<Vec<String>> {
    let ids: Vec<&str> = workflow.nodes.iter().map(|node| node.id.as_str()).collect();
    let known: HashSet<&str> = ids.iter().copied().collect();

    // 先收集所有有向边（去重）：真实的连线 + 「顺序执行」加上的**排序约束**。
    // 去掉重复边很关键 —— 同一条边数两遍会把入度算大，拓扑序就错了。
    let mut candidates: Vec<(&str, &str)> = Vec::new();
    for edge in &workflow.edges {
        if !known.contains(edge.source.as_str()) || !known.contains(edge.target.as_str()) {
            continue;
        }
        if edge.source == edge.target {
            issues.push(
                Issue::error("节点不能把输出接回自己")
                    .at_edge(&edge.id)
                    .at_node(&edge.source),
            );
            continue;
        }
        candidates.push((edge.source.as_str(), edge.target.as_str()));
    }
    candidates.extend(sequence_ordering(workflow));
    candidates.extend(loop_ordering(workflow));

    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    let mut edges: Vec<(&str, &str)> = Vec::new();
    for (from, to) in candidates {
        if from != to && seen.insert((from, to)) {
            edges.push((from, to));
        }
    }

    let mut indegree: HashMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
    let mut successors: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, to) in edges {
        successors.entry(from).or_default().push(to);
        if let Some(degree) = indegree.get_mut(to) {
            *degree += 1;
        }
    }

    let mut queue: VecDeque<&str> = ids
        .iter()
        .copied()
        .filter(|id| indegree.get(id).copied().unwrap_or(0) == 0)
        .collect();
    let mut order = Vec::with_capacity(ids.len());

    while let Some(id) = queue.pop_front() {
        order.push(id.to_string());
        if let Some(next_nodes) = successors.get(id) {
            for next in next_nodes {
                if let Some(degree) = indegree.get_mut(next) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(next);
                    }
                }
            }
        }
    }

    if order.len() != ids.len() {
        issues.push(Issue::error("工作流里存在循环，无法确定执行顺序"));
        return None;
    }
    Some(order)
}

/// 「顺序执行」节点带来的排序约束：把第 k 个输出的直接下游排到第 k+1 个的直接下游之前。
///
/// 返回一串要额外加进拓扑图的有向边。只连**直接下游** —— 它们的后继自然跟着排在后面，
/// 因此不用把整条分支两两连起来（那样边数会爆、也更容易造出假环）。
fn sequence_ordering(workflow: &Workflow) -> Vec<(&str, &str)> {
    let mut extra: Vec<(&str, &str)> = Vec::new();
    for node in &workflow.nodes {
        let Some(spec) = registry().get(&node.kind) else {
            continue;
        };
        if spec.kind.id != crate::nodes::sequence::KIND {
            continue;
        }
        // 按序号排好的输出端口 id：out1、out2、……
        let mut ports: Vec<String> = spec
            .outputs_for(&node.params)
            .into_iter()
            .map(|port| port.id)
            .filter(|id| id.starts_with("out"))
            .collect();
        ports.sort_by_key(|id| id.trim_start_matches("out").parse::<usize>().unwrap_or(0));

        // 每个输出上的直接下游节点（通常一个）。
        let consumers = |port: &str| -> Vec<&str> {
            workflow
                .edges
                .iter()
                .filter(|edge| edge.source == node.id && edge.source_port == port)
                .map(|edge| edge.target.as_str())
                .collect()
        };
        for pair in ports.windows(2) {
            // 只有当**前一个**输出是阻塞的（开关开着），才把后一段排到它后面。
            // 开关关掉就是非阻塞：这一段不施加任何先后约束。
            if !output_blocks(node, &pair[0]) {
                continue;
            }
            for from in consumers(&pair[0]) {
                for to in consumers(&pair[1]) {
                    extra.push((from, to));
                }
            }
        }
    }
    extra
}

/// 「顺序执行」的某个输出是不是阻塞的。开关存在参数 `block{序号}` 里；缺省（没设过）当真。
fn output_blocks(node: &crate::model::workflow::NodeInstance, port: &str) -> bool {
    let Some(index) = port.trim_start_matches("out").parse::<usize>().ok() else {
        return true;
    };
    node.params
        .get(&format!("block{index}"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

/// 静态检查一遍，顺便给出每个节点的真实端口类型。
pub fn resolve(workflow: &Workflow) -> ResolvedWorkflow {
    let analysis = analyze(registry(), workflow);
    let runnable = !analysis.has_errors();
    ResolvedWorkflow {
        nodes: analysis.resolved,
        issues: analysis.issues,
        runnable,
    }
}

// ---------------------------------------------------------------------------
// 执行
// ---------------------------------------------------------------------------

/// 执行工作流。
///
/// `only_node` 给定时只跑「这个节点以及它所有的上游」，用来单步调试。
///
/// 没有界面可对话（测试、无头运行）时走这条路：紫色节点一律报错。
pub fn run(
    workflow: &Workflow,
    only_node: Option<&str>,
    output_root: &Path,
) -> Result<RunReport, AppError> {
    run_with(workflow, only_node, output_root, None, None)
}

/// 一次运行的执行状态。把「跑一个节点」抽成方法，好让「遍历」的循环体可以反复跑。
struct Exec<'a> {
    workflow: &'a Workflow,
    registry: &'a Registry,
    analysis: &'a Analysis,
    output_dir: Option<&'a Path>,
    interaction: Option<&'a Interaction>,
    progress: Option<&'a Progress>,
    /// 循环布局（哪些节点是循环、各自的直接作用域、哪些被循环控制）。
    program: &'a Program,
    /// 节点在 `full_order` 里的下标（自动产物命名用）。
    position_of: HashMap<String, usize>,
    values: HashMap<String, ValueMap>,
    failed: HashSet<String>,
    results: Vec<NodeRunResult>,
    previews_left: usize,
}

impl Exec<'_> {
    /// 跑一个节点：收集输入、跑它、存结果。
    ///
    /// `iteration` 是它所属的循环路径（外层到内层的轮次，空 = 不在循环里）；
    /// 自动落盘的产物用它区分文件名。
    /// `aggregate` 为真（循环体）时把结果**并**进已有的同名结果，而不是新增一条。
    fn run_node(&mut self, node_id: &str, iteration: &[usize], aggregate: bool) {
        // 把这些共享引用拷出来 —— 这样下面改 `self.values` 等字段时不再和它们抢借用。
        let workflow = self.workflow;
        let registry = self.registry;
        let analysis = self.analysis;
        let output_dir = self.output_dir;
        let interaction = self.interaction;
        let progress = self.progress;
        let position = self.position_of.get(node_id).copied().unwrap_or(0);

        let Some(node) = workflow.node(node_id) else {
            return;
        };
        let Some(spec) = registry.get(&node.kind) else {
            return;
        };
        // 先报「开始」，界面把光标移到这个节点上。
        if let Some(progress) = progress {
            progress.started(node_id);
        }

        let mut warnings: Vec<String> = Vec::new();
        let mut inputs: ValueMap = ValueMap::new();
        // 参数可以被上游喂：连到 `param:<id>` 上的值会覆盖掉这个参数。
        // 覆盖之后才拿去算输出端口、才交给节点 —— 所以「输出跟着参数变」的节点
        // 也照样对（比如「图像格式转换」的目标格式接了一段字符串）。
        let mut params = node.params.clone();
        let mut blocked_by: Option<String> = None;
        let mut type_error: Option<String> = None;

        for edge in workflow.incoming(node_id) {
            if self.failed.contains(edge.source.as_str()) {
                blocked_by = Some(edge.source.clone());
                continue;
            }
            let Some(value) = self
                .values
                .get(&edge.source)
                .and_then(|outputs| outputs.get(&edge.source_port))
            else {
                blocked_by = Some(edge.source.clone());
                continue;
            };
            // 运行期的类型核对：编辑期允许「格式未知」，到这里必须对上。
            if let Some(declared) = analysis.node(node_id).and_then(|resolved| {
                resolved
                    .inputs
                    .iter()
                    .find(|port| port.id == edge.target_port)
            }) {
                let actual = value.port_type();
                if !declared.ty.strictly_accepts(actual) {
                    type_error = Some(format!(
                        "输入「{}」声明要{}，实际拿到的是{}",
                        declared.label,
                        declared.ty.label(),
                        actual.label()
                    ));
                }
            }

            // 参数端口：值覆盖到那个参数上，不当成节点的输入数据。
            if let Some(param_id) = edge.target_port.strip_prefix(PARAM_PORT_PREFIX) {
                if let Some(json) = value.to_param_json() {
                    params.insert(param_id.to_string(), json);
                }
                continue;
            }
            inputs.insert(edge.target_port.clone(), value.clone());
        }

        if let Some(upstream) = blocked_by {
            self.failed.insert(node_id.to_string());
            let result = NodeRunResult {
                node_id: node_id.to_string(),
                kind: node.kind.clone(),
                name: spec.kind.name.clone(),
                status: NodeStatus::Skipped,
                error: Some(format!("上游节点 {} 没有产出，跳过了", short_id(&upstream))),
                warnings,
                elapsed_ms: 0,
                outputs: Vec::new(),
            };
            if let Some(progress) = progress {
                progress.finished(result.clone());
            }
            self.publish(result, aggregate);
            return;
        }

        if let Some(message) = type_error {
            self.failed.insert(node_id.to_string());
            let result = NodeRunResult {
                node_id: node_id.to_string(),
                kind: node.kind.clone(),
                name: spec.kind.name.clone(),
                status: NodeStatus::Failed,
                error: Some(message),
                warnings,
                elapsed_ms: 0,
                outputs: Vec::new(),
            };
            if let Some(progress) = progress {
                progress.finished(result.clone());
            }
            self.publish(result, aggregate);
            return;
        }

        let node_started = Instant::now();
        let outcome = {
            let mut args = NodeArgs {
                node_id,
                node_name: &spec.kind.name,
                params: &params,
                inputs: &inputs,
                warnings: &mut warnings,
                interaction,
                progress,
            };
            (spec.run)(&mut args)
        };
        let elapsed_ms = node_started.elapsed().as_millis() as u64;

        match outcome {
            Ok(outputs) => {
                let mut port_results = Vec::new();
                let output_ports = spec.outputs_for(&params);
                let multiple_ports = output_ports.len() > 1;
                for port in output_ports {
                    let Some(value) = outputs.get(&port.id) else {
                        continue;
                    };
                    let mut preview = None;
                    let mut path = None;
                    // 色板就是一段 hex 文本 —— 拆出来给界面画小色块。
                    let palette = match value.inner() {
                        Value::Text(text) => crate::model::value::parse_palette(text),
                        _ => None,
                    };

                    if let Value::Image(image) = value.inner() {
                        if self.previews_left > 0 {
                            if let Ok(url) = image.preview_data_url(PREVIEW_MAX_DIM) {
                                self.previews_left -= 1;
                                preview = Some(url);
                            }
                        }
                    }

                    // 产物（图像或媒体）自动落到本次运行的输出目录，方便对照。
                    if let Some((bytes, extension)) = value.product() {
                        if !spec.kind.is_source {
                            if let Some(dir) = output_dir {
                                let file = dir.join(output_file_name(
                                    position,
                                    &spec.kind.name,
                                    &port.label,
                                    multiple_ports,
                                    extension,
                                    iteration,
                                ));
                                if std::fs::write(&file, bytes).is_ok() {
                                    path = Some(file.to_string_lossy().to_string());
                                }
                            }
                        }
                    }

                    port_results.push(PortResult {
                        port_id: port.id.clone(),
                        label: port.label.clone(),
                        ty: value.port_type(),
                        summary: value.describe(),
                        preview,
                        path,
                        palette,
                    });
                }

                self.values.insert(node_id.to_string(), outputs);
                let result = NodeRunResult {
                    node_id: node_id.to_string(),
                    kind: node.kind.clone(),
                    name: spec.kind.name.clone(),
                    status: NodeStatus::Ok,
                    error: None,
                    warnings,
                    elapsed_ms,
                    outputs: port_results,
                };
                if let Some(progress) = progress {
                    progress.finished(result.clone());
                }
                self.publish(result, aggregate);
            }
            Err(error) => {
                self.failed.insert(node_id.to_string());
                let result = NodeRunResult {
                    node_id: node_id.to_string(),
                    kind: node.kind.clone(),
                    name: spec.kind.name.clone(),
                    status: NodeStatus::Failed,
                    error: Some(error.0),
                    warnings,
                    elapsed_ms,
                    outputs: Vec::new(),
                };
                if let Some(progress) = progress {
                    progress.finished(result.clone());
                }
                self.publish(result, aggregate);
            }
        }
    }

    /// 按给定顺序（拓扑序）跑一段节点。
    ///
    /// 段里如果有关内层「遍历」节点，就递归跑它（并跳过它自己的作用域 —— 那些节点
    /// 不在本段的 `ids` 里，因为它们是内层循环的 `direct`）。
    fn run_scope(&mut self, ids: &[String], path: &[usize], aggregate: bool) {
        for id in ids {
            if self.program.loops.contains(id) {
                if let Some(scope) = self.program.direct.get(id) {
                    let scope = scope.clone();
                    self.run_loop(id, &scope, path, aggregate);
                }
            } else {
                self.run_node(id, path, aggregate);
            }
        }
    }

    /// 跑一个「遍历」节点：对每个元素把它**直接控制**的那段跑一遍，最后留出「结束」。
    fn run_loop(&mut self, node_id: &str, scope: &[String], path: &[usize], aggregate: bool) {
        let workflow = self.workflow;
        let registry = self.registry;
        let progress = self.progress;
        let Some(node) = workflow.node(node_id) else {
            return;
        };
        let Some(spec) = registry.get(&node.kind) else {
            return;
        };
        if let Some(progress) = progress {
            progress.started(node_id);
        }
        let started = Instant::now();

        // 取「列表」输入。
        let mut list: Vec<Value> = Vec::new();
        let mut blocked: Option<String> = None;
        for edge in workflow.incoming(node_id) {
            if edge.target_port != "list" {
                continue;
            }
            if self.failed.contains(edge.source.as_str()) {
                blocked = Some(edge.source.clone());
                break;
            }
            match self
                .values
                .get(&edge.source)
                .and_then(|outputs| outputs.get(&edge.source_port))
            {
                Some(value) => match value.as_list() {
                    Ok(items) => list = items.to_vec(),
                    Err(_) => blocked = Some(edge.source.clone()),
                },
                None => blocked = Some(edge.source.clone()),
            }
        }

        let result = if let Some(upstream) = blocked {
            self.failed.insert(node_id.to_string());
            // 循环体也标成跳过（跑一遍空作用域，下游会因缺产出而跳过）。
            self.run_scope(scope, path, true);
            NodeRunResult {
                node_id: node_id.to_string(),
                kind: node.kind.clone(),
                name: spec.kind.name.clone(),
                status: NodeStatus::Skipped,
                error: Some(format!("上游节点 {} 没有产出，跳过了", short_id(&upstream))),
                warnings: Vec::new(),
                elapsed_ms: 0,
                outputs: Vec::new(),
            }
        } else {
            let limit = {
                let max = crate::model::params::integer(&node.params, "max", 0);
                if max > 0 {
                    (max as usize).min(list.len())
                } else {
                    list.len()
                }
            };
            // 「结束」先放上去，循环体与下游都看得到。
            let mut outputs = ValueMap::new();
            outputs.insert(crate::nodes::iterate::FINALLY_PORT.to_string(), Value::Void);
            self.values.insert(node_id.to_string(), outputs);

            if list.is_empty() {
                // 没元素也跑一遍空作用域，好让循环体在报告里有一条「跳过」。
                self.run_scope(scope, path, true);
            } else {
                for (index, element) in list.iter().take(limit).enumerate() {
                    if let Some(outputs) = self.values.get_mut(node_id) {
                        outputs.insert(
                            crate::nodes::iterate::ITEM_PORT.to_string(),
                            element.clone(),
                        );
                    }
                    // 把本层的轮次接到路径后面 —— 内层循环的产物文件名会带上整条链。
                    let mut child_path = path.to_vec();
                    child_path.push(index);
                    self.run_scope(scope, &child_path, true);
                }
            }

            NodeRunResult {
                node_id: node_id.to_string(),
                kind: node.kind.clone(),
                name: spec.kind.name.clone(),
                status: NodeStatus::Ok,
                error: None,
                warnings: vec![format!("遍历了 {limit} 个元素")],
                elapsed_ms: started.elapsed().as_millis() as u64,
                outputs: Vec::new(),
            }
        };

        if let Some(progress) = progress {
            progress.finished(result.clone());
        }
        self.publish(result, aggregate);
    }

    /// 记一条结果；`aggregate` 为真时并进已有的同名结果（循环体反复跑）。
    fn publish(&mut self, result: NodeRunResult, aggregate: bool) {
        if aggregate {
            if let Some(existing) = self
                .results
                .iter_mut()
                .find(|item| item.node_id == result.node_id)
            {
                merge_result(existing, result);
                return;
            }
        }
        self.results.push(result);
    }
}

/// 把一个循环体节点新一轮的结果并进上一轮的。
fn merge_result(existing: &mut NodeRunResult, new: NodeRunResult) {
    existing.elapsed_ms += new.elapsed_ms;
    // 出错 > 跳过 > 完成，取更“重”的那个状态。
    let rank = |status: NodeStatus| match status {
        NodeStatus::Ok => 0,
        NodeStatus::Skipped => 1,
        NodeStatus::Failed => 2,
    };
    if rank(new.status) > rank(existing.status) {
        existing.status = new.status;
    }
    if new.error.is_some() {
        existing.error = new.error;
    }
    existing.warnings.extend(new.warnings);
    if !new.outputs.is_empty() {
        existing.outputs = new.outputs;
    }
}

/// 一个「遍历」节点的两个作用域集合。
struct LoopSets {
    /// 从它的「元素」输出出发能到达的所有节点（含各层嵌套）。
    reach: HashSet<String>,
    /// **它自己直接控制**的节点：`reach` 里挖掉各内层循环的 `reach`。
    /// 内层循环节点本身在这一层里（它 ∈ reach、∉ 自己的 reach）。
    direct: HashSet<String>,
}

/// 找出所有「遍历」节点及各自的 reach / direct。
fn loop_sets(workflow: &Workflow) -> HashMap<String, LoopSets> {
    let registry = registry();
    // 先算每个遍历节点的 reach。
    let mut reach: HashMap<String, HashSet<String>> = HashMap::new();
    for node in &workflow.nodes {
        let Some(spec) = registry.get(&node.kind) else {
            continue;
        };
        if spec.kind.id != crate::nodes::iterate::KIND {
            continue;
        }
        reach.insert(
            node.id.clone(),
            downstream_of(workflow, &node.id, crate::nodes::iterate::ITEM_PORT),
        );
    }
    // direct(L) = reach(L) \ ∪ reach(K)（K 是 reach(L) 里任何内层循环）。
    reach
        .iter()
        .map(|(id, own)| {
            let mut direct = own.clone();
            for (inner, inner_reach) in &reach {
                if inner != id && own.contains(inner) {
                    for node in inner_reach {
                        direct.remove(node);
                    }
                }
            }
            (
                id.clone(),
                LoopSets {
                    reach: own.clone(),
                    direct,
                },
            )
        })
        .collect()
}

/// 运行期的循环布局：每个遍历节点的直接作用域（拓扑序）、循环节点集合、
/// 以及所有被循环控制的节点（这些不进最外层作用域）。
struct Program {
    direct: HashMap<String, Vec<String>>,
    loops: HashSet<String>,
    controlled: HashSet<String>,
}

/// 用最终的拓扑序把 `loop_sets` 落地成可执行的布局。
fn build_program(workflow: &Workflow, order: &[String]) -> Program {
    let sets = loop_sets(workflow);
    let loops: HashSet<String> = sets.keys().cloned().collect();
    let controlled: HashSet<String> = sets
        .values()
        .flat_map(|group| group.reach.iter().cloned())
        .collect();
    let direct = sets
        .iter()
        .map(|(id, group)| {
            let scope = order
                .iter()
                .filter(|node| group.direct.contains(*node))
                .cloned()
                .collect();
            (id.clone(), scope)
        })
        .collect();
    Program {
        direct,
        loops,
        controlled,
    }
}

/// 从某个输出端口出发，沿连线能到达的所有下游节点。
fn downstream_of(workflow: &Workflow, node_id: &str, port: &str) -> HashSet<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = workflow
        .edges
        .iter()
        .filter(|edge| edge.source == node_id && edge.source_port == port)
        .map(|edge| edge.target.clone())
        .collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for edge in &workflow.edges {
            if edge.source == id {
                stack.push(edge.target.clone());
            }
        }
    }
    seen
}

/// 「遍历」带来的排序约束：每个循环的**直接**作用域里，来自循环外面（不在本循环 `reach` 里）
/// 的依赖都排到该循环之前 —— 这样循环开跑时它要用的外部值都已算好。
/// 对嵌套的内层循环同样成立（内层的外部依赖里，属于外层直接作用域的那些会排到内层之前）。
fn loop_ordering(workflow: &Workflow) -> Vec<(&str, &str)> {
    let sets = loop_sets(workflow);
    let mut extra = Vec::new();
    for node in &workflow.nodes {
        let Some(group) = sets.get(&node.id) else {
            continue;
        };
        for member in &group.direct {
            for edge in &workflow.edges {
                if edge.target != *member {
                    continue;
                }
                let source = edge.source.as_str();
                if source != node.id && !group.reach.contains(source) {
                    extra.push((source, node.id.as_str()));
                }
            }
        }
    }
    extra
}

/// 执行工作流，带着一条与界面通话的通道 —— 紫色节点会在中途停下来问用户。
pub fn run_with(
    workflow: &Workflow,
    only_node: Option<&str>,
    output_root: &Path,
    interaction: Option<&Interaction>,
    progress: Option<&Progress>,
) -> Result<RunReport, AppError> {
    let started = Instant::now();
    let registry = registry();
    let analysis = analyze(registry, workflow);

    if analysis.has_errors() {
        return Ok(RunReport {
            ok: false,
            duration_ms: 0,
            order: Vec::new(),
            nodes: Vec::new(),
            issues: analysis.issues,
            output_dir: None,
            finished_at: now_millis(),
        });
    }

    let full_order = analysis.order.clone().unwrap_or_default();
    let execution = match only_node {
        Some(target) if !analysis.index_of.contains_key(target) => {
            let mut report = RunReport {
                ok: false,
                duration_ms: 0,
                order: Vec::new(),
                nodes: Vec::new(),
                issues: analysis.issues,
                output_dir: None,
                finished_at: now_millis(),
            };
            report
                .issues
                .push(Issue::error(format!("找不到节点：{target}")));
            return Ok(report);
        }
        Some(target) => ancestors_of(workflow, target, &full_order),
        None => full_order.clone(),
    };

    let output_dir = prepare_output_dir(output_root, workflow, only_node.is_none());

    // 循环布局；最外层作用域 = 执行顺序里**不被任何循环控制**的那些节点
    // （循环体、以及内层循环，都由各自的循环负责跑）。
    let program = build_program(workflow, &full_order);

    let mut exec = Exec {
        workflow,
        registry,
        analysis: &analysis,
        output_dir: output_dir.as_deref(),
        interaction,
        progress,
        program: &program,
        position_of: execution
            .iter()
            .enumerate()
            .map(|(index, id)| (id.clone(), index))
            .collect(),
        values: HashMap::new(),
        failed: HashSet::new(),
        results: Vec::with_capacity(execution.len()),
        previews_left: MAX_PREVIEWS,
    };

    let top_scope: Vec<String> = execution
        .iter()
        .filter(|id| !program.controlled.contains(*id))
        .cloned()
        .collect();
    exec.run_scope(&top_scope, &[], false);

    let results = std::mem::take(&mut exec.results);
    drop(exec);

    let ok = results.iter().all(|result| result.status == NodeStatus::Ok);

    Ok(RunReport {
        ok,
        duration_ms: started.elapsed().as_millis() as u64,
        order: execution,
        nodes: results,
        issues: analysis.issues,
        output_dir: output_dir.map(|dir| dir.to_string_lossy().to_string()),
        finished_at: now_millis(),
    })
}

/// 目标节点加上它所有的上游，按拓扑序排列。
fn ancestors_of(workflow: &Workflow, target: &str, full_order: &[String]) -> Vec<String> {
    let mut needed: HashSet<&str> = HashSet::new();
    let mut stack = vec![target];
    while let Some(id) = stack.pop() {
        if !needed.insert(id) {
            continue;
        }
        for edge in workflow.incoming(id) {
            stack.push(edge.source.as_str());
        }
    }
    full_order
        .iter()
        .filter(|id| needed.contains(id.as_str()))
        .cloned()
        .collect()
}

fn prepare_output_dir(
    output_root: &Path,
    workflow: &Workflow,
    clear_stale: bool,
) -> Option<PathBuf> {
    let name = if workflow.name.trim().is_empty() {
        workflow.id.clone()
    } else {
        file_slug(&workflow.name)
    };
    let dir = output_root.join(name);
    std::fs::create_dir_all(&dir).ok()?;
    // 只跑一部分时不要清目录，否则会把别的节点的产物抹掉。
    if clear_stale {
        clear_previous_outputs(&dir);
    }
    Some(dir)
}

/// 上一次运行留下的产物，这次运行前先清掉，免得新旧混杂。
fn clear_previous_outputs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let bytes = name.as_bytes();
        let is_ours = bytes.len() > 3
            && bytes[0].is_ascii_digit()
            && bytes[1].is_ascii_digit()
            && bytes[2] == b'-';
        if is_ours {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn output_file_name(
    position: usize,
    node_name: &str,
    port_label: &str,
    multiple_ports: bool,
    extension: &str,
    iteration: &[usize],
) -> String {
    // 循环体每跑一轮都会重复同一个节点 —— 文件名里带上轮次路径（嵌套时是 001-002-），
    // 才不会被下一轮盖掉。
    let turn: String = iteration
        .iter()
        .map(|index| format!("{:03}-", index + 1))
        .collect();
    // 只有多输出端口的节点才需要在文件名里带上端口名。
    let stem = if multiple_ports {
        format!(
            "{turn}{:02}-{}-{}",
            position + 1,
            file_slug(node_name),
            file_slug(port_label)
        )
    } else {
        format!("{turn}{:02}-{}", position + 1, file_slug(node_name))
    };
    format!("{stem}.{extension}")
}

fn file_slug(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && !"/\\:*?\"<>|".contains(*c))
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    if trimmed.is_empty() {
        "node".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn short_id(node_id: &str) -> String {
    node_id.chars().take(6).collect()
}

#[cfg(test)]
mod tests;
