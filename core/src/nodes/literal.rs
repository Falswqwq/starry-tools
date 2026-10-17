//! 变量节点：本地保存一个值，带 Get / Set 两个数据口。
//!
//! 它们不接入执行线：Get 被别的节点读取时才求值；Set 允许接多个上游，
//! 按上游实际运行的顺序写入，后写到的覆盖先到的。没有写入时使用节点上的本地值。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params;
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::{NodeSpec, VariablePorts};

pub const KIND_TEXT: &str = "literal_text";
pub const KIND_NUMBER: &str = "literal_number";
pub const KIND_BOOL: &str = "literal_bool";

const PARAM_VALUE: &str = "value";
const PORT_GET: &str = "out";
const PORT_SET: &str = "set";

pub fn text_spec() -> NodeSpec {
    NodeSpec::fixed(
        variable(
            KIND_TEXT,
            "文本",
            "保存一段文本。Get 供下游读取，Set 可由多个上游写入。",
            ParamSpec::Text {
                default: String::new(),
                // 多行：文本可以有多行，框也会跟着内容长。
                multiline: true,
                placeholder: Some("写点什么…".into()),
            },
            PortType::Text,
        ),
        run_text,
    )
    .accent(super::DATA_ACCENT)
    .variable(VariablePorts::new(PARAM_VALUE, PORT_GET, PORT_SET))
    .pure()
}

pub fn number_spec() -> NodeSpec {
    NodeSpec::fixed(
        variable(
            KIND_NUMBER,
            "数字",
            "保存一个数字。Get 供下游读取，Set 可由多个上游写入。",
            ParamSpec::Number {
                default: 0.0,
                min: f64::MIN,
                max: f64::MAX,
                step: 1.0,
                integer: false,
                unit: None,
            },
            PortType::Number,
        ),
        run_number,
    )
    .accent(super::DATA_ACCENT)
    .variable(VariablePorts::new(PARAM_VALUE, PORT_GET, PORT_SET))
    .pure()
}

pub fn bool_spec() -> NodeSpec {
    NodeSpec::fixed(
        variable(
            KIND_BOOL,
            "布尔",
            "保存真或假。Get 供下游读取，Set 可由多个上游写入。",
            ParamSpec::Bool { default: false },
            PortType::Bool,
        ),
        run_bool,
    )
    .accent(super::DATA_ACCENT)
    .variable(VariablePorts::new(PARAM_VALUE, PORT_GET, PORT_SET))
    .pure()
}

fn variable(id: &str, name: &str, description: &str, spec: ParamSpec, ty: PortType) -> NodeKind {
    NodeKind {
        id: id.into(),
        name: name.into(),
        category: "变量".into(),
        description: description.into(),
        // Set 可以接多个上游；运行中后写到的值生效。
        inputs: vec![PortDef::new(PORT_SET, "Set", ty)
            .multi()
            .hint("可接多个上游；按运行顺序以后写到的为准")],
        // Get 是普通数据输出，可以同时被多个节点读取。
        outputs: vec![PortDef::new(PORT_GET, "Get", ty)],
        // 本地值始终可编辑；Set 连线只覆盖本次求值，不改写参数。
        params: vec![ParamDef::new(PARAM_VALUE, "", spec).local_only()],
        notes: vec![
            "变量节点不接执行线：Get 被读取时按需求值。".into(),
            "Set 可接多个上游，按运行顺序以最后一个为准；未接 Set 时使用本地值。".into(),
        ],
    }
}

fn run_text(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let text = params::string(args.params, PARAM_VALUE, "");
    Ok(ValueMap::from([(PORT_GET.to_string(), Value::text(text))]))
}

fn run_number(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let number = params::number(args.params, PARAM_VALUE, 0.0);
    Ok(ValueMap::from([(
        PORT_GET.to_string(),
        Value::Number(number),
    )]))
}

fn run_bool(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let value = params::boolean(args.params, PARAM_VALUE, false);
    Ok(ValueMap::from([(PORT_GET.to_string(), Value::Bool(value))]))
}
