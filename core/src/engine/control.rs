//! 由注册表控制流协议驱动的串行调度与嵌套循环。
use super::*;
use crate::registry::ControlFlow;

pub(super) fn sequence_ordering<'a>(
    registry: &Registry,
    workflow: &'a Workflow,
) -> Vec<(&'a str, &'a str)> {
    let mut extra = Vec::new();
    for node in &workflow.nodes {
        let Some(spec) = registry.get(&node.kind) else {
            continue;
        };
        let Some(ControlFlow::Sequence {
            output_prefix,
            blocking_prefix,
        }) = &spec.control_flow
        else {
            continue;
        };
        let mut ports: Vec<_> = spec
            .outputs_for(&node.params)
            .into_iter()
            .filter_map(|port| {
                let index = port.id.strip_prefix(output_prefix)?.parse::<usize>().ok()?;
                Some((index, port.id))
            })
            .collect();
        ports.sort_by_key(|(index, _)| *index);
        for pair in ports.windows(2) {
            if !node
                .params
                .get(&format!("{blocking_prefix}{}", pair[0].0))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
            {
                continue;
            }
            for from in workflow
                .edges
                .iter()
                .filter(|edge| edge.source == node.id && edge.source_port == pair[0].1)
            {
                for to in workflow
                    .edges
                    .iter()
                    .filter(|edge| edge.source == node.id && edge.source_port == pair[1].1)
                {
                    extra.push((from.target.as_str(), to.target.as_str()));
                }
            }
        }
    }
    extra
}

pub(super) struct LoopSets {
    pub(super) reach: HashSet<String>,
    pub(super) direct: HashSet<String>,
}

pub(super) fn loop_sets(registry: &Registry, workflow: &Workflow) -> HashMap<String, LoopSets> {
    let mut reach = HashMap::new();
    for node in &workflow.nodes {
        let Some(ControlFlow::Loop { item_output, .. }) = registry
            .get(&node.kind)
            .and_then(|spec| spec.control_flow.as_ref())
        else {
            continue;
        };
        reach.insert(
            node.id.clone(),
            downstream_of(workflow, &node.id, item_output),
        );
    }
    reach
        .iter()
        .map(|(id, own)| {
            let mut direct = own.clone();
            for (inner, inner_reach) in &reach {
                if inner != id && own.contains(inner) {
                    direct.retain(|node| !inner_reach.contains(node));
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

pub(super) struct Program {
    direct: HashMap<String, Vec<String>>,
    reach: HashMap<String, HashSet<String>>,
    loops: HashSet<String>,
    pub(super) controlled: HashSet<String>,
}

pub(super) fn build_program(
    registry: &Registry,
    workflow: &Workflow,
    execution: &[String],
) -> Program {
    let sets = loop_sets(registry, workflow);
    Program {
        loops: sets.keys().cloned().collect(),
        controlled: sets
            .values()
            .flat_map(|set| set.reach.iter().cloned())
            .collect(),
        direct: sets
            .iter()
            .map(|(id, set)| {
                (
                    id.clone(),
                    execution
                        .iter()
                        .filter(|node| set.direct.contains(*node))
                        .cloned()
                        .collect(),
                )
            })
            .collect(),
        reach: sets.into_iter().map(|(id, set)| (id, set.reach)).collect(),
    }
}

fn downstream_of(workflow: &Workflow, node_id: &str, port: &str) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut stack: Vec<_> = workflow
        .edges
        .iter()
        .filter(|edge| edge.source == node_id && edge.source_port == port)
        .map(|edge| edge.target.clone())
        .collect();
    while let Some(id) = stack.pop() {
        if seen.insert(id.clone()) {
            stack.extend(
                workflow
                    .edges
                    .iter()
                    .filter(|edge| edge.source == id)
                    .map(|edge| edge.target.clone()),
            );
        }
    }
    seen
}

pub(super) fn loop_ordering<'a>(
    registry: &Registry,
    workflow: &'a Workflow,
) -> Vec<(&'a str, &'a str)> {
    let sets = loop_sets(registry, workflow);
    let mut extra = Vec::new();
    for node in &workflow.nodes {
        let Some(group) = sets.get(&node.id) else {
            continue;
        };
        // 内层循环用到的外部输入也必须在外层开始之前就绪。
        for edge in &workflow.edges {
            if group.reach.contains(&edge.target)
                && edge.source != node.id
                && !group.reach.contains(&edge.source)
            {
                extra.push((edge.source.as_str(), node.id.as_str()));
            }
        }
    }
    extra
}

/// 只运行目标祖先；消费 finally 时包含完整循环体，但 item 分支不扩张到兄弟分支。
pub(super) fn ancestors_of(
    registry: &Registry,
    workflow: &Workflow,
    target: &str,
    order: &[String],
) -> Vec<String> {
    let sets = loop_sets(registry, workflow);
    let mut needed = HashSet::new();
    let mut stack = vec![target.to_string()];
    while let Some(id) = stack.pop() {
        if !needed.insert(id.clone()) {
            continue;
        }
        for edge in workflow.incoming(&id) {
            stack.push(edge.source.clone());
            let source = workflow
                .node(&edge.source)
                .and_then(|node| registry.get(&node.kind));
            if let Some(ControlFlow::Loop { finally_output, .. }) =
                source.and_then(|spec| spec.control_flow.as_ref())
            {
                if edge.source_port == *finally_output {
                    stack.extend(sets[&edge.source].reach.iter().cloned());
                }
            }
        }
    }
    order
        .iter()
        .filter(|id| needed.contains(*id))
        .cloned()
        .collect()
}

impl Exec<'_> {
    pub(super) fn run_scope(&mut self, ids: &[String], path: &[usize], aggregate: bool) {
        for id in ids {
            if self.program.loops.contains(id) {
                let scope = self.program.direct.get(id).cloned().unwrap_or_default();
                self.run_loop(id, &scope, path, aggregate);
            } else {
                self.run_node(id, path, aggregate);
            }
        }
    }

    fn reset_loop_scope(&mut self, node_id: &str) {
        if let Some(reach) = self.program.reach.get(node_id) {
            for id in reach {
                self.failed.remove(id);
                self.values.remove(id);
            }
        }
    }

    fn run_loop(&mut self, node_id: &str, scope: &[String], path: &[usize], aggregate: bool) {
        let mut result = self.start_result(node_id);
        let started = Instant::now();
        self.reset_loop_scope(node_id);
        self.values.remove(node_id);
        self.failed.remove(node_id);
        let prepared = self.collect_inputs(node_id);
        let workflow = self.workflow;
        let registry = self.registry;
        let node = workflow.node(node_id).expect("analyzed node");
        let spec = registry.get(&node.kind).expect("analyzed kind");
        let Some(ControlFlow::Loop {
            list_input,
            item_output,
            finally_output,
            max_param,
        }) = &spec.control_flow
        else {
            return;
        };
        let prepared = prepared.and_then(|(params, inputs)| {
            let list = inputs
                .get(list_input)
                .ok_or_else(|| (NodeStatus::Failed, format!("缺少列表输入：{list_input}")))?
                .as_list()
                .map_err(|error| (NodeStatus::Failed, error.0))?
                .to_vec();
            Ok((params, list))
        });
        match prepared {
            Err((status, error)) => {
                result.status = status;
                result.error = Some(error);
                self.failed.insert(node_id.to_string());
                self.run_scope(scope, path, true);
            }
            Ok((params, list)) => {
                let max = crate::model::params::integer(&params, max_param, 0);
                let limit = if max > 0 {
                    usize::try_from(max).unwrap_or(usize::MAX).min(list.len())
                } else {
                    list.len()
                };
                let mut outputs = ValueMap::new();
                outputs.insert(finally_output.clone(), Value::Void);
                self.values.insert(node_id.to_string(), outputs);
                for (index, element) in list.into_iter().take(limit).enumerate() {
                    self.reset_loop_scope(node_id);
                    self.values
                        .get_mut(node_id)
                        .unwrap()
                        .insert(item_output.clone(), element);
                    let mut child_path = path.to_vec();
                    child_path.push(index);
                    self.run_scope(scope, &child_path, true);
                }
                result.warnings.push(format!("遍历了 {limit} 个元素"));
            }
        }
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        self.finish_result(result, aggregate);
    }
}
