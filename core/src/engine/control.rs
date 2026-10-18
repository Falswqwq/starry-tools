//! Explicit synchronous Exec dispatch. Data edges never schedule effectful nodes.
use super::*;
use crate::error::NodeError;
use crate::registry::ControlFlow;

pub(super) fn validate(
    registry: &Registry,
    workflow: &Workflow,
    resolved: &[ResolvedNode],
    issues: &mut Vec<Issue>,
) {
    let mut entries = 0;
    for node in &workflow.nodes {
        let Some(spec) = registry.get(&node.kind) else {
            continue;
        };
        entries += usize::from(spec.entry);
        let Some(ports) = resolved.iter().find(|r| r.node_id == node.id) else {
            continue;
        };
        if spec.pure
            && (ports.inputs.iter().chain(&ports.outputs).any(|p| p.exec)
                || spec.control_flow.is_some()
                || spec.entry)
        {
            issues.push(Issue::error("pure 节点不能具有 Exec/入口/控制流契约").at_node(&node.id));
        }
        if let Some(variable) = &spec.variable {
            let get = ports
                .outputs
                .iter()
                .find(|p| p.id == variable.get && !p.exec);
            let set = ports
                .inputs
                .iter()
                .find(|p| p.id == variable.set && !p.exec);
            let param = spec
                .kind
                .params
                .iter()
                .any(|p| p.id == variable.value_param);
            let typed = get.zip(set).is_some_and(|(get, set)| get.ty == set.ty);
            if !typed || !param {
                issues.push(Issue::error("变量注册契约与端口/参数声明不一致").at_node(&node.id));
            }
        }
        if let Some(flow) = &spec.control_flow {
            let valid = match flow {
                ControlFlow::Sequence { output_prefix, .. } => {
                    ports.outputs.iter().filter(|p| p.exec).all(|p| {
                        p.id.strip_prefix(output_prefix)
                            .is_some_and(|i| i.parse::<usize>().is_ok())
                    })
                }
                ControlFlow::Repeat {
                    count_param,
                    index_output,
                    body_output,
                    finally_output,
                } => {
                    let count_exists = spec.kind.params.iter().any(|param| {
                        param.id == *count_param
                            && matches!(
                                param.spec,
                                crate::model::node_kind::ParamSpec::Number { integer: true, .. }
                            )
                    });
                    count_exists
                        && ports.outputs.iter().any(|p| {
                            p.id == *index_output
                                && !p.exec
                                && p.ty == crate::model::port_type::PortType::Number
                        })
                        && ports.outputs.iter().any(|p| p.id == *body_output && p.exec)
                        && ports
                            .outputs
                            .iter()
                            .any(|p| p.id == *finally_output && p.exec)
                        && body_output != finally_output
                }
                ControlFlow::Loop {
                    list_input,
                    item_output,
                    body_output,
                    finally_output,
                    ..
                } => {
                    ports
                        .inputs
                        .iter()
                        .any(|p| p.id == *list_input && !p.exec && p.ty.is_list())
                        && ports
                            .outputs
                            .iter()
                            .any(|p| p.id == *item_output && !p.exec)
                        && ports.outputs.iter().any(|p| p.id == *body_output && p.exec)
                        && ports
                            .outputs
                            .iter()
                            .any(|p| p.id == *finally_output && p.exec)
                        && body_output != finally_output
                }
            };
            if !valid {
                issues.push(Issue::error("控制流注册契约与端口声明不一致").at_node(&node.id));
            }
        }
        if spec.control_flow.is_none() && ports.outputs.iter().filter(|p| p.exec).count() > 1 {
            issues.push(
                Issue::error("多个 Exec 输出必须声明 Sequence/Loop 控制流协议").at_node(&node.id),
            );
        }
        for port in ports.outputs.iter().filter(|p| p.exec) {
            if workflow
                .outgoing(&node.id)
                .filter(|e| e.source_port == port.id)
                .count()
                > 1
            {
                issues.push(
                    Issue::error("每个 Exec 输出仅允许一条线；请使用 Sequence 显式排序")
                        .at_port(&node.id, &port.id),
                );
            }
        }
    }
    if entries == 0
        && workflow
            .nodes
            .iter()
            .any(|n| registry.get(&n.kind).is_some_and(|s| !s.pure))
    {
        issues.push(Issue::warning("没有入口；未被 Exec 触发的节点不会运行"));
    }
    if entries > 1 {
        issues.push(Issue::error("只允许一个入口；多个入口的顺序不明确"));
    }
}

impl Exec<'_> {
    fn fail_control(&mut self, id: &str, message: impl Into<String>) {
        let mut result = self.start_result(id);
        result.status = NodeStatus::Failed;
        result.error = Some(message.into());
        self.finish_result(result, false);
    }

    fn emit(&mut self, id: &str, port: &str, path: &[usize]) {
        let targets: Vec<_> = self
            .workflow
            .outgoing(id)
            .filter(|e| e.source_port == port)
            .map(|e| e.target.clone())
            .collect();
        for target in targets {
            self.trigger(&target, path);
        }
    }

    pub(super) fn trigger(&mut self, id: &str, path: &[usize]) {
        self.steps += 1;
        if self.steps > 100_000 || self.active.len() >= 256 || !self.active.insert(id.to_owned()) {
            self.fail_control(id, "Exec 环或执行预算/嵌套深度超限");
            return;
        }
        let node = self.workflow.node(id).unwrap();
        let spec = self.registry.get(&node.kind).unwrap();
        let flow = spec.control_flow.clone();
        match flow {
            Some(ControlFlow::Loop {
                list_input,
                item_output,
                body_output,
                finally_output,
                max_param,
            }) => {
                self.trace.push(id.to_owned());
                let prepared = self.collect_inputs(id);
                match prepared {
                    Err((_, error)) => self.fail_control(id, error),
                    Ok((params, inputs)) => {
                        match inputs
                            .get(&list_input)
                            .ok_or_else(|| NodeError("Loop 缺少列表".into()))
                            .and_then(Value::as_list)
                        {
                            Err(error) => self.fail_control(id, error.0),
                            Ok(items) => {
                                let max = crate::model::params::integer(&params, &max_param, 0)
                                    .max(0) as usize;
                                let items: Vec<_> = items
                                    .iter()
                                    .take(if max == 0 { usize::MAX } else { max })
                                    .cloned()
                                    .collect();
                                let result = self.start_result(id);
                                self.finish_result(result, false);
                                for (i, item) in items.into_iter().enumerate() {
                                    // Restore the enclosing frame, never leak inner iteration data.
                                    let saved = self.values.clone();
                                    self.values.insert(
                                        id.to_owned(),
                                        [(item_output.clone(), item)].into_iter().collect(),
                                    );
                                    let mut inner = path.to_vec();
                                    inner.push(i);
                                    self.emit(id, &body_output, &inner);
                                    self.values = saved;
                                    if self.results.iter().any(|r| r.status != NodeStatus::Ok) {
                                        break;
                                    }
                                }
                                self.values.remove(id);
                                if self.results.iter().all(|r| r.status == NodeStatus::Ok) {
                                    self.emit(id, &finally_output, path);
                                }
                            }
                        }
                    }
                }
            }
            Some(ControlFlow::Repeat {
                count_param,
                index_output,
                body_output,
                finally_output,
            }) => {
                self.trace.push(id.to_owned());
                match self.collect_inputs(id) {
                    Err((_, error)) => self.fail_control(id, error),
                    Ok((params, _)) => {
                        // 上限只是防御异常存档；正常参数控件已经限制得更小。
                        let count = crate::model::params::integer(&params, &count_param, 0)
                            .clamp(0, 100_000) as usize;
                        let result = self.start_result(id);
                        self.finish_result(result, false);
                        for i in 0..count {
                            // 每轮隔离数据帧，结束后恢复外层值。
                            let saved = self.values.clone();
                            self.values.insert(
                                id.to_owned(),
                                [(index_output.clone(), Value::Number(i as f64))]
                                    .into_iter()
                                    .collect(),
                            );
                            let mut inner = path.to_vec();
                            inner.push(i);
                            self.emit(id, &body_output, &inner);
                            self.values = saved;
                            if self.results.iter().any(|r| r.status != NodeStatus::Ok) {
                                break;
                            }
                        }
                        self.values.remove(id);
                        if self.results.iter().all(|r| r.status == NodeStatus::Ok) {
                            self.emit(id, &finally_output, path);
                        }
                    }
                }
            }
            flow => {
                self.run_node(id, path, false);
                if !self.failed.contains(id) {
                    let mut ports: Vec<_> = spec
                        .outputs_for(&node.params)
                        .into_iter()
                        .filter(|p| p.exec)
                        .collect();
                    if let Some(ControlFlow::Sequence { output_prefix, .. }) = flow {
                        ports.retain(|p| p.id.starts_with(&output_prefix));
                        ports.sort_by_key(|p| {
                            p.id.strip_prefix(&output_prefix)
                                .and_then(|v| v.parse::<usize>().ok())
                                .unwrap_or(usize::MAX)
                        });
                    }
                    for port in ports {
                        self.emit(id, &port.id, path);
                        if self.results.iter().any(|r| r.status != NodeStatus::Ok) {
                            break;
                        }
                    }
                }
            }
        }
        self.active.remove(id);
    }
}
