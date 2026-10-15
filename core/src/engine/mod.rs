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
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::error::AppError;
use crate::interaction::Interaction;
use crate::model::node_kind::{PortDef, PARAM_PORT_PREFIX};
use crate::model::params::Params;
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::model::workflow::{now_millis, Workflow};
use crate::progress::Progress;
use crate::registry::{registry, Registry};

mod artifacts;
mod control;
pub(crate) use artifacts::validate_file_name;
#[cfg(test)]
use control::loop_sets;
use control::{build_program, loop_ordering, sequence_ordering, Program};

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

    let order = topological_order(registry, workflow, &mut issues);

    Analysis {
        resolved,
        index_of,
        issues,
        order,
    }
}

/// Kahn 算法。有环就返回 `None` 并记一条错误。
fn topological_order(
    registry: &Registry,
    workflow: &Workflow,
    issues: &mut Vec<Issue>,
) -> Option<Vec<String>> {
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
    candidates.extend(sequence_ordering(registry, workflow));
    candidates.extend(loop_ordering(registry, workflow));

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
    artifacts: &'a mut artifacts::Artifacts,
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
    /// 参数连线与普通输入共用失败传播和运行期类型检查。
    fn collect_inputs(&self, node_id: &str) -> Result<(Params, ValueMap), (NodeStatus, String)> {
        let node = self.workflow.node(node_id).expect("analyzed node");
        let spec = self.registry.get(&node.kind).expect("analyzed kind");
        let mut params = spec.kind.default_params();
        params.extend(node.params.clone());
        let mut inputs = ValueMap::new();
        for edge in self.workflow.incoming(node_id) {
            let value = self
                .values
                .get(&edge.source)
                .and_then(|ports| ports.get(&edge.source_port));
            if self.failed.contains(&edge.source) || value.is_none() {
                return Err((
                    NodeStatus::Skipped,
                    format!("上游节点 {} 没有产出，跳过了", short_id(&edge.source)),
                ));
            }
            let value = value.unwrap();
            if let Some(param_id) = edge.target_port.strip_prefix(PARAM_PORT_PREFIX) {
                if let Some(json) = value.to_param_json() {
                    params.insert(param_id.to_string(), json);
                }
            } else {
                inputs.insert(edge.target_port.clone(), value.clone());
            }
        }
        // 连线以静态声明的端口为契约；参数覆盖可能隐藏端口，但不能绕过其类型检查。
        let ports = spec.kind.inputs_for(&node.params);
        for edge in self.workflow.incoming(node_id) {
            let value = &self.values[&edge.source][&edge.source_port];
            if let Some(port) = ports.iter().find(|port| port.id == edge.target_port) {
                let actual = value.port_type();
                if !port.ty.strictly_accepts(actual) {
                    return Err((
                        NodeStatus::Failed,
                        format!(
                            "输入「{}」声明要{}，实际拿到的是{}",
                            port.label,
                            port.ty.label(),
                            actual.label()
                        ),
                    ));
                }
            }
        }
        Ok((params, inputs))
    }

    fn start_result(&self, node_id: &str) -> NodeRunResult {
        if let Some(progress) = self.progress {
            progress.started(node_id);
        }
        let node = self.workflow.node(node_id).expect("analyzed node");
        let spec = self.registry.get(&node.kind).expect("analyzed kind");
        NodeRunResult {
            node_id: node_id.to_string(),
            kind: node.kind.clone(),
            name: spec.kind.name.clone(),
            status: NodeStatus::Ok,
            error: None,
            warnings: Vec::new(),
            elapsed_ms: 0,
            outputs: Vec::new(),
        }
    }

    fn run_node(&mut self, node_id: &str, iteration: &[usize], aggregate: bool) {
        let mut result = self.start_result(node_id);
        let (params, inputs) = match self.collect_inputs(node_id) {
            Ok(prepared) => prepared,
            Err((status, error)) => {
                result.status = status;
                result.error = Some(error);
                self.finish_result(result, aggregate);
                return;
            }
        };
        let node = self.workflow.node(node_id).expect("analyzed node");
        let spec = self.registry.get(&node.kind).expect("analyzed kind");
        let started = Instant::now();
        let outcome = (spec.run)(&mut NodeArgs {
            node_id,
            node_name: &spec.kind.name,
            params: &params,
            inputs: &inputs,
            warnings: &mut result.warnings,
            interaction: self.interaction,
            progress: self.progress,
        });
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok(outputs) => {
                let output_ports = spec.outputs_for(&params);
                let multiple_ports = output_ports.len() > 1;
                for port in output_ports {
                    let Some(value) = outputs.get(&port.id) else {
                        continue;
                    };
                    let mut preview = None;
                    let mut path = None;
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
                    if !spec.kind.is_source {
                        if let Some((bytes, extension)) = value.product() {
                            let filename = output_file_name(
                                self.position_of.get(node_id).copied().unwrap_or(0),
                                &spec.kind.name,
                                &port.label,
                                multiple_ports,
                                extension,
                                iteration,
                            );
                            match self.artifacts.write_product(&filename, bytes) {
                                Ok(file) => path = Some(file.to_string_lossy().to_string()),
                                Err(error) => {
                                    result.warnings.push(format!("自动保存产物失败：{error}"))
                                }
                            }
                        }
                    }
                    result.outputs.push(PortResult {
                        port_id: port.id,
                        label: port.label,
                        ty: value.port_type(),
                        summary: value.describe(),
                        preview,
                        path,
                        palette,
                    });
                }
                self.values.insert(node_id.to_string(), outputs);
            }
            Err(error) => {
                result.status = NodeStatus::Failed;
                result.error = Some(error.0);
            }
        }
        self.finish_result(result, aggregate);
    }

    /// 单轮失败状态与累计报告分开：后续轮次成功不抹掉历史错误。
    fn finish_result(&mut self, result: NodeRunResult, aggregate: bool) {
        if result.status == NodeStatus::Ok {
            self.failed.remove(&result.node_id);
        } else {
            self.failed.insert(result.node_id.clone());
            self.values.remove(&result.node_id);
        }
        if let Some(progress) = self.progress {
            progress.finished(result.clone());
        }
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
    if rank(new.status) >= rank(existing.status) {
        existing.status = new.status;
        if new.error.is_some() {
            existing.error = new.error;
        }
    }
    existing.warnings.extend(new.warnings);
    if !new.outputs.is_empty() {
        existing.outputs = new.outputs;
    }
}

/// 执行工作流，带着一条与界面通话的通道 —— 紫色节点会在中途停下来问用户。
pub fn run_with(
    workflow: &Workflow,
    only_node: Option<&str>,
    output_root: &Path,
    interaction: Option<&Interaction>,
    progress: Option<&Progress>,
) -> Result<RunReport, AppError> {
    run_with_registry(
        registry(),
        workflow,
        only_node,
        output_root,
        interaction,
        progress,
    )
}

fn run_with_registry(
    registry: &Registry,
    workflow: &Workflow,
    only_node: Option<&str>,
    output_root: &Path,
    interaction: Option<&Interaction>,
    progress: Option<&Progress>,
) -> Result<RunReport, AppError> {
    let started = Instant::now();
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
        Some(target) => control::ancestors_of(registry, workflow, target, &full_order),
        None => full_order.clone(),
    };

    let mut artifacts = artifacts::Artifacts::prepare(output_root, workflow, only_node.is_none())?;
    let output_dir = artifacts.dir().to_path_buf();
    let program = build_program(registry, workflow, &execution);

    let mut exec = Exec {
        workflow,
        registry,
        artifacts: &mut artifacts,
        interaction,
        progress,
        program: &program,
        position_of: full_order
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
        output_dir: Some(output_dir.to_string_lossy().to_string()),
        finished_at: now_millis(),
    })
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
    let mut cleaned = String::new();
    for c in text
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control() && !"/\\:*?\"<>|".contains(*c))
        .take(48)
    {
        if cleaned.len() + c.len_utf8() > 96 {
            break;
        }
        cleaned.push(c);
    }
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "node".to_string()
    } else {
        trimmed.to_string()
    }
}

fn short_id(node_id: &str) -> String {
    node_id.chars().take(6).collect()
}

#[cfg(test)]
mod tests;
