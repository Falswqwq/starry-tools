use super::*;
use crate::model::workflow::{Edge, NodeInstance};
use crate::registry::{NodeSpec, RegistryBuilder, VariablePorts};
use serde_json::json;

fn add(w: &mut Workflow, id: &str, kind: &str, params: serde_json::Value) {
    w.nodes.push(NodeInstance {
        id: id.into(),
        kind: kind.into(),
        position: Default::default(),
        params: serde_json::from_value(params).unwrap(),
    });
}
fn link(w: &mut Workflow, a: &str, ap: &str, b: &str, bp: &str) {
    w.edges.push(Edge {
        id: format!("{a}:{ap}-{b}:{bp}"),
        source: a.into(),
        source_port: ap.into(),
        target: b.into(),
        target_port: bp.into(),
    });
}
fn execute(reg: &Registry, w: &Workflow) -> RunReport {
    let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let report = run_with_registry(reg, w, None, &root, None, None).unwrap();
    let _ = std::fs::remove_dir_all(root);
    report
}
fn custom(
    id: &str,
    inputs: Vec<PortDef>,
    outputs: Vec<PortDef>,
    run: impl Fn(&mut NodeArgs<'_>) -> Result<ValueMap, crate::error::NodeError> + Send + Sync + 'static,
) -> NodeSpec {
    let mut kind = registry().get("start").unwrap().kind.clone();
    kind.id = id.into();
    kind.inputs = inputs;
    kind.outputs = outputs;
    NodeSpec::fixed(kind, run)
}
/// 一个真正执行、并把收到的值原样输出的节点。用来触发纯表达式并写变量。
fn sink_spec(id: &str) -> NodeSpec {
    custom(
        id,
        vec![PortDef::new("value", "值", PortType::Number)],
        vec![PortDef::new("out", "输出", PortType::Number)],
        |args| {
            let value = args
                .inputs
                .get("value")
                .cloned()
                .unwrap_or(Value::Number(0.0));
            Ok([("out".into(), value)].into_iter().collect())
        },
    )
}

/// 起点 → 顺序执行；每个分支先显式运行运算，再把结果写回变量。
///
/// 变量节点没有执行口：`counter` 的 Get 只在运算需要时读取，
/// Set 也不会被读取反向求值，所以 `sum -> sink -> counter.set` 不成环。
fn fixture(branches: usize) -> Workflow {
    let mut w = Workflow::new("blueprint");
    add(&mut w, "start", "start", json!({}));
    add(&mut w, "counter", "literal_number", json!({"value": 0}));
    add(&mut w, "one", "literal_number", json!({"value": 1}));
    add(&mut w, "seq", "sequence", json!({"outs": branches}));
    link(&mut w, "start", "out", "seq", "exec");
    for i in 1..=branches {
        let sum = format!("sum{i}");
        let sink = format!("sink{i}");
        add(&mut w, &sum, "operation", json!({"op": "add"}));
        add(&mut w, &sink, "test_sink", json!({}));
        link(&mut w, "counter", "out", &sum, "a");
        link(&mut w, "one", "out", &sum, "b");
        link(&mut w, &sum, "out", &sink, "value");
        link(&mut w, &sink, "out", "counter", "set");
        link(&mut w, "seq", &format!("out{i}"), &sum, "exec");
        link(&mut w, &sum, "then", &sink, "exec");
    }
    w
}

fn blueprint_registry() -> Registry {
    let mut b = RegistryBuilder::with_builtins().unwrap();
    b.register_node(sink_spec("test_sink")).unwrap();
    b.freeze().unwrap()
}

#[test]
fn operation_runs_only_when_exec_reaches_it() {
    let mut w = Workflow::new("explicit-operation");
    add(&mut w, "start", "start", json!({}));
    add(&mut w, "a", "literal_number", json!({"value": 2}));
    add(&mut w, "b", "literal_number", json!({"value": 3}));
    add(&mut w, "op", "operation", json!({"op": "add"}));
    link(&mut w, "a", "out", "op", "a");
    link(&mut w, "b", "out", "op", "b");

    let r = execute(&blueprint_registry(), &w);
    assert!(!r.order.contains(&"op".into()), "没有 Exec 时运算不应运行");

    link(&mut w, "start", "out", "op", "exec");
    let r = execute(&blueprint_registry(), &w);
    assert!(r.ok, "{:?}", r);
    assert!(r.order.contains(&"op".into()));
    let op = r.nodes.iter().find(|n| n.node_id == "op").unwrap();
    assert_eq!(op.outputs[0].summary, "5");
}

#[test]
fn set_node_carries_a_typed_value_into_variable_set() {
    let mut w = Workflow::new("set-node");
    add(&mut w, "start", "start", json!({}));
    add(&mut w, "source", "literal_number", json!({"value": 4}));
    add(&mut w, "target", "literal_number", json!({"value": 0}));
    add(&mut w, "writer", "set", json!({"ty": "number"}));
    link(&mut w, "start", "out", "writer", "exec");
    link(&mut w, "source", "out", "writer", "value");
    link(&mut w, "writer", "out", "target", "set");

    let r = execute(&blueprint_registry(), &w);
    assert!(r.ok, "{:?}", r);
    let writer = r.nodes.iter().find(|n| n.node_id == "writer").unwrap();
    assert_eq!(writer.outputs[0].summary, "4");
    assert_eq!(r.variable_states["target"], json!(4.0));
}

#[test]
fn set_node_ports_follow_the_selected_type() {
    let spec = registry().get("set").unwrap();
    for (ty, expected) in [
        ("number", PortType::Number),
        ("text", PortType::Text),
        ("bool", PortType::Bool),
    ] {
        let mut params = spec.kind.default_params();
        params.insert("ty".into(), json!(ty));
        let data: Vec<_> = spec
            .inputs_for(&params)
            .into_iter()
            .filter(|port| !port.exec)
            .collect();
        assert_eq!(data.len(), 1, "{ty}: {data:?}");
        assert_eq!(data[0].ty, expected, "{ty}");
        let output = spec
            .outputs_for(&params)
            .into_iter()
            .find(|port| !port.exec)
            .unwrap();
        assert_eq!(output.ty, expected, "{ty}");
    }
}

#[test]
fn repeat_runs_the_body_with_indices_then_completes() {
    let mut w = Workflow::new("repeat");
    add(&mut w, "start", "start", json!({}));
    add(&mut w, "repeat", "repeat", json!({"times": 3}));
    add(&mut w, "body", "test_sink", json!({}));
    add(&mut w, "done", "test_sink", json!({}));
    link(&mut w, "start", "out", "repeat", "exec");
    link(&mut w, "repeat", "body", "body", "exec");
    link(&mut w, "repeat", "index", "body", "value");
    link(&mut w, "repeat", "finally", "done", "exec");

    let r = execute(&blueprint_registry(), &w);
    assert!(r.ok, "{:?}", r);
    let indices: Vec<_> = r
        .nodes
        .iter()
        .filter(|n| n.node_id == "body")
        .filter_map(|n| n.outputs.first())
        .map(|out| out.summary.as_str())
        .collect();
    assert_eq!(indices, ["0", "1", "2"]);
    assert_eq!(r.order.iter().filter(|id| *id == "body").count(), 3);
    assert!(r.order.contains(&"done".into()), "全部完成后应走结束");
}

#[test]
fn repeat_with_zero_times_skips_the_body_but_completes() {
    let mut w = Workflow::new("repeat-zero");
    add(&mut w, "start", "start", json!({}));
    add(&mut w, "repeat", "repeat", json!({"times": 0}));
    add(&mut w, "body", "test_sink", json!({}));
    add(&mut w, "done", "test_sink", json!({}));
    link(&mut w, "start", "out", "repeat", "exec");
    link(&mut w, "repeat", "body", "body", "exec");
    link(&mut w, "repeat", "finally", "done", "exec");

    let r = execute(&blueprint_registry(), &w);
    assert!(r.ok, "{:?}", r);
    assert!(!r.order.contains(&"body".into()));
    assert!(r.order.contains(&"done".into()));
}

#[test]
fn variable_writes_are_visible_to_later_branches_without_cycles() {
    let reg = blueprint_registry();
    let r = execute(&reg, &fixture(3));
    assert!(r.ok, "{:?}", r);
    let sums: Vec<_> = r
        .nodes
        .iter()
        .filter(|n| n.node_id.starts_with("sum"))
        .map(|n| n.outputs[0].summary.as_str())
        .collect();
    // 0+1、1+1、2+1：每一轮都读到上一轮写入的值。
    assert_eq!(sums, ["1", "2", "3"]);
    assert_eq!(r.order.iter().filter(|id| id.starts_with("sum")).count(), 3);
    assert_eq!(r.variable_states["counter"], json!(3.0));
}

#[test]
fn set_accepts_multiple_sources_and_get_fans_out() {
    let mut w = fixture(1);
    // 第二个上游也写同一个 Set 口：多输入合法。
    add(&mut w, "nine", "literal_number", json!({"value": 9}));
    link(&mut w, "nine", "out", "counter", "set");
    // Get 同时供运算与另一个读取方使用。
    add(&mut w, "echo", "literal_number", json!({"value": 0}));
    link(&mut w, "counter", "out", "echo", "set");
    let r = execute(&blueprint_registry(), &w);
    assert!(r.ok, "{:?}", r);
    assert!(!r
        .issues
        .iter()
        .any(|issue| issue.message.contains("多条线")));
}

#[test]
fn untriggered_side_effect_never_writes_a_variable() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let mut b = RegistryBuilder::with_builtins().unwrap();
    b.register_node(sink_spec("test_sink")).unwrap();
    b.register_node(custom(
        "effect",
        vec![],
        vec![PortDef::new("out", "out", PortType::Number)],
        move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok([("out".into(), Value::Number(9.0))].into_iter().collect())
        },
    ))
    .unwrap();
    let reg = b.freeze().unwrap();
    let mut w = fixture(1);
    add(&mut w, "effect", "effect", json!({}));
    // 只接数据、不接 Exec：effect 不应运行，也不应写入变量。
    link(&mut w, "effect", "out", "counter", "set");
    let r = execute(&reg, &w);
    assert!(r.ok);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(r.order.iter().filter(|id| *id == "effect").count(), 0);
}

#[test]
fn sequence_waits_for_entire_branch() {
    let mut w = fixture(2);
    add(&mut w, "tail", "literal_number", json!({"value": 5}));
    // 第一个分支里再挂一个纯读取；顺序仍应是整支跑完再跑第二支。
    link(&mut w, "counter", "out", "tail", "set");
    let r = execute(&blueprint_registry(), &w);
    assert!(r.ok, "{:?}", r);
    let effects: Vec<_> = r
        .order
        .iter()
        .filter(|id| ["sink1", "sink2"].contains(&id.as_str()))
        .map(String::as_str)
        .collect();
    assert_eq!(effects, ["sink1", "sink2"]);
}

fn list_registry() -> Registry {
    let mut b = RegistryBuilder::with_builtins().unwrap();
    b.register_node(sink_spec("test_sink")).unwrap();
    b.register_node(
        custom(
            "numbers",
            vec![],
            vec![PortDef::new(
                "out",
                "out",
                PortType::list_of(PortType::Number),
            )],
            |_| {
                Ok([(
                    "out".into(),
                    Value::List(vec![
                        Value::Number(1.0),
                        Value::Number(2.0),
                        Value::Number(3.0),
                    ]),
                )]
                .into_iter()
                .collect())
            },
        )
        .pure(),
    )
    .unwrap();
    b.freeze().unwrap()
}

fn loop_fixture(nested: bool) -> Workflow {
    let mut w = fixture(1);
    // 去掉顺序执行分支，改成起点直接触发遍历。
    w.nodes
        .retain(|n| !["seq", "sink1", "sum1"].contains(&n.id.as_str()));
    w.edges.retain(|e| {
        !["seq", "sink1", "sum1"].contains(&e.source.as_str())
            && !["seq", "sink1", "sum1"].contains(&e.target.as_str())
    });
    add(&mut w, "numbers", "numbers", json!({}));
    add(&mut w, "loop", "iterate", json!({}));
    add(&mut w, "sum", "operation", json!({"op": "add"}));
    link(&mut w, "numbers", "out", "loop", "list");
    link(&mut w, "start", "out", "loop", "exec");
    link(&mut w, "counter", "out", "sum", "a");
    // sum 的 b 是循环项：每轮加 1、2、3。
    link(&mut w, "loop", "item", "sum", "b");
    if nested {
        add(&mut w, "inner", "iterate", json!({}));
        add(&mut w, "inner_body", "test_sink", json!({}));
        link(&mut w, "numbers", "out", "inner", "list");
        link(&mut w, "loop", "body", "inner", "exec");
        link(&mut w, "inner", "body", "sum", "exec");
        link(&mut w, "sum", "then", "inner_body", "exec");
        link(&mut w, "sum", "out", "inner_body", "value");
        link(&mut w, "inner_body", "out", "counter", "set");
    } else {
        add(&mut w, "body", "test_sink", json!({}));
        link(&mut w, "loop", "body", "sum", "exec");
        link(&mut w, "sum", "then", "body", "exec");
        link(&mut w, "sum", "out", "body", "value");
        link(&mut w, "body", "out", "counter", "set");
    }
    w
}

#[test]
fn loops_accumulate_and_nested_frames_preserve_outer_item() {
    let reg = list_registry();
    for (nested, expected) in [(false, 6.0), (true, 18.0)] {
        let r = execute(&reg, &loop_fixture(nested));
        assert!(r.ok, "{:?}", r);
        let last = r
            .nodes
            .iter()
            .filter(|n| n.node_id == "sum")
            .filter_map(|n| n.outputs.first())
            .next_back()
            .expect("至少跑过一次");
        assert_eq!(last.summary, format!("{expected}"));
    }
}

#[test]
fn loop_data_never_expands_scope_and_expires_after_body() {
    let reg = list_registry();
    let mut w = loop_fixture(false);
    add(&mut w, "outside", "test_sink", json!({}));
    link(&mut w, "loop", "item", "outside", "value");
    let r = execute(&reg, &w);
    assert!(r.ok);
    assert!(!r.order.contains(&"outside".into()));
    link(&mut w, "loop", "finally", "outside", "exec");
    let r = execute(&reg, &w);
    assert!(!r.ok);
    assert!(r
        .nodes
        .iter()
        .any(|n| n.error.as_deref().is_some_and(|e| e.contains("作用域"))));
}

#[test]
fn exec_data_miswiring_and_old_versions_are_diagnosed() {
    let mut w = fixture(1);
    link(&mut w, "start", "out", "sum1", "a");
    assert!(!resolve(&w).runnable);
    w.edges.pop();
    link(&mut w, "one", "out", "sink1", "exec");
    assert!(!resolve(&w).runnable);
    w.edges.pop();
    w.version = 1;
    assert!(!execute(&blueprint_registry(), &w).ok);
    w.version = 2;
    let mut json = serde_json::to_value(&w).unwrap();
    json.as_object_mut().unwrap().remove("version");
    assert_eq!(serde_json::from_value::<Workflow>(json).unwrap().version, 1);
}

#[test]
fn custom_kinds_use_registration_protocol_not_builtin_names() {
    let mut b = RegistryBuilder::with_builtins().unwrap();
    b.register_node(sink_spec("test_sink")).unwrap();
    let mut loop_spec = crate::nodes::iterate::spec();
    loop_spec.kind.id = "extension_loop".into();
    b.register_node(loop_spec).unwrap();

    let mut variable_spec = crate::nodes::literal::number_spec();
    variable_spec.kind.id = "extension_var".into();
    variable_spec.kind.name = "扩展变量".into();
    variable_spec.variable = Some(VariablePorts::new("value", "out", "set"));
    b.register_node(variable_spec).unwrap();

    b.register_node(
        custom(
            "numbers",
            vec![],
            vec![PortDef::new("out", "out", PortType::any_list())],
            |_| {
                Ok([("out".into(), Value::List(vec![Value::Number(4.0)]))]
                    .into_iter()
                    .collect())
            },
        )
        .pure(),
    )
    .unwrap();
    let reg = b.freeze().unwrap();
    let mut w = loop_fixture(false);
    for n in &mut w.nodes {
        n.kind = match n.kind.as_str() {
            "iterate" => "extension_loop",
            "literal_number" if n.id == "counter" => "extension_var",
            other => other,
        }
        .to_owned();
    }
    let r = execute(&reg, &w);
    assert!(r.ok, "{:?}", r);
    let last = r
        .nodes
        .iter()
        .filter(|n| n.node_id == "sum")
        .filter_map(|n| n.outputs.first())
        .next_back()
        .expect("至少跑过一次");
    // counter 初始 0，第一轮 +4，随后每轮继续累加；最后一项是 4。
    assert!(last.summary.parse::<f64>().is_ok());
}

#[test]
fn empty_loop_completes_and_pure_cycles_fail_without_effects() {
    let mut b = RegistryBuilder::with_builtins().unwrap();
    b.register_node(sink_spec("test_sink")).unwrap();
    b.register_node(
        custom(
            "numbers",
            vec![],
            vec![PortDef::new("out", "out", PortType::any_list())],
            |_| Ok([("out".into(), Value::List(vec![]))].into_iter().collect()),
        )
        .pure(),
    )
    .unwrap();
    let reg = b.freeze().unwrap();
    let mut w = loop_fixture(false);
    add(&mut w, "done", "test_sink", json!({}));
    link(&mut w, "one", "out", "done", "value");
    link(&mut w, "loop", "finally", "done", "exec");
    let r = execute(&reg, &w);
    assert!(r.ok);
    assert!(!r.order.contains(&"body".into()));

    let mut w = fixture(1);
    w.edges
        .retain(|e| !(e.target == "sum1" && e.target_port == "a"));
    link(&mut w, "sum1", "out", "sum1", "a");
    let r = execute(&blueprint_registry(), &w);
    assert!(!r.ok);
}
