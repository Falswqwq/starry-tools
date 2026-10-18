//! 工作流的检查与执行。
//!
//! 这里做两件事：
//!
//! 1. [`resolve`] —— 静态检查。把每个节点的端口类型算出来（输入节点要读文件
//!    头才知道格式），顺便列出所有问题。编辑器每改一下就调它一次，用来画
//!    端口徽标、判断连线能不能接、给节点打红标。
//! 2. [`run`] —— 真的把图画一遍。从入口沿显式 Exec 执行节点，纯数据按需求值，运行期再严格核对一遍
//!    类型，把每个节点的产物（预览 + 落盘文件）收集成报告。

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::error::AppError;
use crate::interaction::Interaction;
use crate::model::node_kind::PortDef;
use crate::model::params::Params;
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::model::workflow::{now_millis, Workflow};
use crate::progress::Progress;
use crate::registry::{registry, Registry};

mod artifacts;
mod control;
pub(crate) use artifacts::validate_file_name;

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
    /// 成功运行结束时的变量节点状态（node_id -> 标量值）。
    /// 调用方可在成功后写回节点参数，下一轮从新值开始。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub variable_states: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    pub finished_at: i64,
}

// ---------------------------------------------------------------------------
// 静态检查
// ---------------------------------------------------------------------------

struct Analysis {
    resolved: Vec<ResolvedNode>,
    issues: Vec<Issue>,
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
    if workflow.version != crate::model::workflow::CURRENT_VERSION {
        issues.push(Issue::error(
            "工作流版本不支持：请显式迁移到 v2 并重新连接 Exec；不会推测执行顺序",
        ));
    }
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
                // 含**参数端口**（每个能被上游喂的参数各一个）——
                // 它们也是真正的输入端口，连线、类型检查都走同一套。
                inputs: spec.inputs_for(&node.params),
                outputs: spec.outputs_for(&node.params),
            },
            None => {
                issues.push(
                    Issue::error(format!("未知的节点类型「{}」", node.kind)).at_node(&node.id),
                );
                ResolvedNode {
                    node_id: node.id.clone(),
                    kind: node.kind.clone(),
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

        if !target_port.exec {
            let duplicate = !filled.insert((edge.target.clone(), edge.target_port.clone()));
            if duplicate && !target_port.multi {
                issues.push(
                    Issue::error(format!("输入端口「{}」连接了多条线", target_port.label))
                        .at_edge(&edge.id),
                );
            }
        }

        let source_port = resolved[index_of[&edge.source]]
            .outputs
            .iter()
            .find(|p| p.id == edge.source_port)
            .unwrap();
        let type_ok = target_port.ty.accepts(source_type)
            || registry.types().can_convert(source_type, target_port.ty);
        if source_port.exec != target_port.exec || (!target_port.exec && !type_ok) {
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
        for port in &resolved[index_of[&node.id]].inputs {
            if port.required && !filled.contains(&(node.id.clone(), port.id.clone())) {
                issues.push(
                    Issue::error(format!("必填的输入「{}」还没有连上", port.label))
                        .at_port(&node.id, &port.id),
                );
            }
        }
        if spec.variable.is_none()
            && !spec.kind.inputs.is_empty()
            && workflow.incoming(&node.id).next().is_none()
        {
            issues.push(
                Issue::warning(format!("「{}」还没有连接任何输入", spec.kind.name))
                    .at_node(&node.id),
            );
        }
    }

    control::validate(registry, workflow, &resolved, &mut issues);
    Analysis { resolved, issues }
}

/// Resolve ports and validate without executing callbacks.
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
/// `only_node` 仅允许纯节点求值；不会绕过 Exec 执行副作用。
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
    evaluating: HashSet<String>,
    active: HashSet<String>,
    trace: Vec<String>,
    steps: usize,
    /// 节点在 `full_order` 里的下标（自动产物命名用）。
    position_of: HashMap<String, usize>,
    /// 节点本地变量在一次运行中的当前值（node_id -> 值）。
    variable_values: HashMap<String, Value>,
    values: HashMap<String, ValueMap>,
    failed: HashSet<String>,
    results: Vec<NodeRunResult>,
    previews_left: usize,
}

impl Exec<'_> {
    /// 用节点参数里的本地值给所有变量节点铺一次初始状态。
    fn initialize_variables(&mut self) {
        let nodes: Vec<_> = self
            .workflow
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node.kind.clone(), node.params.clone()))
            .collect();
        for (node_id, kind, params) in nodes {
            let Some(spec) = self.registry.get(&kind) else {
                continue;
            };
            let Some(variable) = spec.variable.clone() else {
                continue;
            };
            let mut merged = spec.kind.default_params();
            merged.extend(params);
            let inputs = ValueMap::new();
            let mut warnings = Vec::new();
            let result = (spec.run)(&mut NodeArgs {
                node_id: &node_id,
                node_name: &spec.kind.name,
                params: &merged,
                inputs: &inputs,
                warnings: &mut warnings,
                interaction: self.interaction,
                progress: self.progress,
            });
            let value = result
                .ok()
                .and_then(|outputs| outputs.get(&variable.get).cloned());
            if let Some(value) = value {
                self.variable_values.insert(node_id, value);
            }
        }
    }

    /// 上游节点产出值后，把它写进所有接在变量 Set 口上的目标。
    ///
    /// 多条 Set 连线按上游实际运行的顺序写入，后到的覆盖先到的；
    /// 写入不会反向触发任何节点。
    fn publish_variable_writes(&mut self, node_id: &str, outputs: &ValueMap) -> Result<(), String> {
        let edges: Vec<_> = self.workflow.outgoing(node_id).cloned().collect();
        for edge in edges {
            let Some(source_value) = outputs.get(&edge.source_port) else {
                continue;
            };
            let Some(target) = self.workflow.node(&edge.target) else {
                continue;
            };
            let Some(spec) = self.registry.get(&target.kind) else {
                continue;
            };
            let Some(variable) = spec.variable.as_ref() else {
                continue;
            };
            if variable.set != edge.target_port {
                continue;
            }
            let mut params = spec.kind.default_params();
            params.extend(target.params.clone());
            let Some(port) = spec
                .inputs_for(&params)
                .into_iter()
                .find(|port| port.id == variable.set)
            else {
                continue;
            };
            let value = if port.ty.strictly_accepts(source_value.port_type()) {
                source_value.clone()
            } else {
                match self.registry.types().convert_value(
                    source_value.port_type(),
                    port.ty,
                    source_value,
                ) {
                    Ok(Some(value)) => value,
                    Ok(None) => continue,
                    Err(error) => {
                        return Err(format!("变量 Set 隐式转换失败：{}", error.0));
                    }
                }
            };
            self.variable_values.insert(edge.target.clone(), value);
        }
        Ok(())
    }

    /// 参数连线与普通输入共用失败传播和运行期类型检查。
    fn collect_inputs(
        &mut self,
        node_id: &str,
    ) -> Result<(Params, ValueMap), (NodeStatus, String)> {
        let node = self.workflow.node(node_id).expect("analyzed node");
        let spec = self.registry.get(&node.kind).expect("analyzed kind");
        let mut params = spec.kind.default_params();
        params.extend(node.params.clone());
        let ports = spec.inputs_for(&params);
        let edges: Vec<_> = self.workflow.incoming(node_id).cloned().collect();
        let mut inputs = ValueMap::new();
        for edge in edges {
            let port = ports.iter().find(|p| p.id == edge.target_port).unwrap();
            if port.exec {
                continue;
            }
            // Set 是写入口，不构成 Get 的求值依赖；值由上游运行时直接写入。
            if spec
                .variable
                .as_ref()
                .is_some_and(|variable| variable.set == port.id)
            {
                continue;
            }
            let source = self.workflow.node(&edge.source).unwrap();
            if self.registry.get(&source.kind).unwrap().pure {
                if self.evaluating.len() >= 256 || !self.evaluating.insert(edge.source.clone()) {
                    return Err((NodeStatus::Failed, "纯数据依赖存在循环".into()));
                }
                self.run_node(&edge.source, &[], false);
                self.evaluating.remove(&edge.source);
            }
            let source_value = self
                .values
                .get(&edge.source)
                .and_then(|v| v.get(&edge.source_port))
                .ok_or_else(|| {
                    (
                        NodeStatus::Failed,
                        format!("数据源 {} 未执行或已离开循环作用域", edge.source),
                    )
                })?;
            let value = if port.ty.strictly_accepts(source_value.port_type()) {
                source_value.clone()
            } else {
                match self.registry.types().convert_value(
                    source_value.port_type(),
                    port.ty,
                    source_value,
                ) {
                    Ok(Some(value)) => value,
                    Ok(None) => {
                        return Err((
                            NodeStatus::Failed,
                            format!(
                                "输入 {} 运行时类型不匹配（{} → {}）",
                                port.label,
                                source_value.port_type().label(),
                                port.ty.label()
                            ),
                        ));
                    }
                    Err(error) => {
                        return Err((
                            NodeStatus::Failed,
                            format!("输入 {} 隐式转换失败：{}", port.label, error.0),
                        ));
                    }
                }
            };
            if let Some(param) = &port.param {
                params.insert(
                    param.clone(),
                    value
                        .to_param_json()
                        .ok_or_else(|| (NodeStatus::Failed, "参数值不可序列化".into()))?,
                );
            } else {
                inputs.insert(port.id.clone(), value);
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
        self.trace.push(node_id.to_owned());
        self.values.remove(node_id);
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
        let outcome = if let Some(variable) = &spec.variable {
            self.variable_values
                .get(node_id)
                .cloned()
                .map(|value| ValueMap::from([(variable.get.clone(), value)]))
                .ok_or_else(|| crate::error::NodeError("变量节点没有初始值".into()))
        } else {
            (spec.run)(&mut NodeArgs {
                node_id,
                node_name: &spec.kind.name,
                params: &params,
                inputs: &inputs,
                warnings: &mut result.warnings,
                interaction: self.interaction,
                progress: self.progress,
            })
        };
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok(outputs) => {
                let output_ports = spec.outputs_for(&params);
                let multiple_ports = output_ports.len() > 1;
                for port in output_ports.into_iter().filter(|p| !p.exec) {
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
                    if spec.auto_artifact {
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
                match self.publish_variable_writes(node_id, &outputs) {
                    Ok(()) => {
                        self.values.insert(node_id.to_string(), outputs);
                    }
                    Err(error) => {
                        result.status = NodeStatus::Failed;
                        result.error = Some(error);
                    }
                }
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
            variable_states: Default::default(),
            output_dir: None,
            finished_at: now_millis(),
        });
    }

    if let Some(target) = only_node {
        let allowed = workflow
            .node(target)
            .and_then(|n| registry.get(&n.kind))
            .is_some_and(|s| s.pure);
        if !allowed {
            return Ok(RunReport {
                ok: false,
                duration_ms: 0,
                order: vec![],
                nodes: vec![],
                issues: vec![Issue::error(
                    "单节点调试仅支持纯节点；副作用必须从入口沿 Exec 触发",
                )],
                variable_states: Default::default(),
                output_dir: None,
                finished_at: now_millis(),
            });
        }
    }
    let full_order: Vec<_> = workflow.nodes.iter().map(|n| n.id.clone()).collect();
    let mut artifacts = artifacts::Artifacts::prepare(output_root, workflow, only_node.is_none())?;
    let output_dir = artifacts.dir().to_path_buf();

    let mut exec = Exec {
        workflow,
        registry,
        artifacts: &mut artifacts,
        interaction,
        progress,
        evaluating: HashSet::new(),
        active: HashSet::new(),
        trace: vec![],
        steps: 0,
        position_of: full_order
            .iter()
            .enumerate()
            .map(|(index, id)| (id.clone(), index))
            .collect(),
        variable_values: HashMap::new(),
        values: HashMap::new(),
        failed: HashSet::new(),
        results: Vec::new(),
        previews_left: MAX_PREVIEWS,
    };
    exec.initialize_variables();

    if let Some(target) = only_node {
        exec.evaluating.insert(target.to_owned());
        exec.run_node(target, &[], false);
    } else {
        for node in &workflow.nodes {
            if registry.get(&node.kind).unwrap().entry {
                exec.trigger(&node.id, &[]);
            }
        }
    }
    let variable_states = exec
        .variable_values
        .iter()
        .filter_map(|(node_id, value)| value.to_param_json().map(|value| (node_id.clone(), value)))
        .collect();
    let execution = std::mem::take(&mut exec.trace);
    let results = std::mem::take(&mut exec.results);
    drop(exec);

    let ok = results.iter().all(|result| result.status == NodeStatus::Ok);

    Ok(RunReport {
        ok,
        duration_ms: started.elapsed().as_millis() as u64,
        order: execution,
        nodes: results,
        issues: analysis.issues,
        variable_states,
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod blueprint_tests;
