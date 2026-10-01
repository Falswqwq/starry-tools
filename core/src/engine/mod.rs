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
use crate::image_io::ImageValue;
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

    let mut indegree: HashMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
    let mut successors: HashMap<&str, Vec<&str>> = HashMap::new();

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
        successors
            .entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        if let Some(degree) = indegree.get_mut(edge.target.as_str()) {
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
        None => full_order,
    };

    let output_dir = prepare_output_dir(output_root, workflow, only_node.is_none());

    let mut values: HashMap<String, ValueMap> = HashMap::new();
    let mut failed: HashSet<String> = HashSet::new();
    let mut results: Vec<NodeRunResult> = Vec::with_capacity(execution.len());
    let mut previews_left = MAX_PREVIEWS;

    for (position, node_id) in execution.iter().enumerate() {
        let node = workflow.node(node_id).expect("执行顺序里的节点一定存在");
        let spec = registry.get(&node.kind).expect("前面已经校验过节点类型");
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
            if failed.contains(edge.source.as_str()) {
                blocked_by = Some(edge.source.clone());
                continue;
            }
            let Some(value) = values
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
            failed.insert(node_id.clone());
            let result = NodeRunResult {
                node_id: node_id.clone(),
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
            results.push(result);
            continue;
        }

        if let Some(message) = type_error {
            failed.insert(node_id.clone());
            let result = NodeRunResult {
                node_id: node_id.clone(),
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
            results.push(result);
            continue;
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
                        if previews_left > 0 {
                            if let Ok(url) = image.preview_data_url(PREVIEW_MAX_DIM) {
                                previews_left -= 1;
                                preview = Some(url);
                            }
                        }
                        if !spec.kind.is_source {
                            if let Some(dir) = &output_dir {
                                let file = dir.join(output_file_name(
                                    position,
                                    &spec.kind.name,
                                    &port.label,
                                    multiple_ports,
                                    image,
                                ));
                                if image.save_to(&file).is_ok() {
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

                values.insert(node_id.clone(), outputs);
                let result = NodeRunResult {
                    node_id: node_id.clone(),
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
                results.push(result);
            }
            Err(error) => {
                failed.insert(node_id.clone());
                let result = NodeRunResult {
                    node_id: node_id.clone(),
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
                results.push(result);
            }
        }
    }

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
    image: &ImageValue,
) -> String {
    // 只有多输出端口的节点才需要在文件名里带上端口名。
    let stem = if multiple_ports {
        format!(
            "{:02}-{}-{}",
            position + 1,
            file_slug(node_name),
            file_slug(port_label)
        )
    } else {
        format!("{:02}-{}", position + 1, file_slug(node_name))
    };
    format!("{stem}.{}", image.format().extension())
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
