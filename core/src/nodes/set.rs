//! 显式写入节点：把值送进执行线，再接到变量节点的 Set 口。
//!
//! 类型由参数选择（数字 / 文本 / 布尔）；输入和输出端口随选择现算。
//! 节点本身不保存值，只在执行线上把上游数据原样传下去。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params::{self, Params};
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "set";

const PARAM_TYPE: &str = "ty";
const PORT_VALUE: &str = "value";
const PORT_OUT: &str = "out";

const TYPE_NUMBER: &str = "number";
const TYPE_TEXT: &str = "text";
const TYPE_BOOL: &str = "bool";

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "Set".into(),
            category: "变量".into(),
            description: "把数字、文本或布尔值送上执行线，再连接到变量节点的 Set 口。".into(),
            inputs: vec![PortDef::new(PORT_VALUE, "变量输入", PortType::Number).required()],
            outputs: vec![PortDef::new(PORT_OUT, "输出", PortType::Number)],
            params: vec![ParamDef::new(
                PARAM_TYPE,
                "变量类型",
                ParamSpec::Select {
                    default: TYPE_NUMBER.into(),
                    options: vec![
                        SelectOption::new(TYPE_NUMBER, "数字").hint("NUM"),
                        SelectOption::new(TYPE_TEXT, "文本").hint("TXT"),
                        SelectOption::new(TYPE_BOOL, "布尔").hint("BOOL"),
                    ],
                },
            )
            .described("决定变量输入和输出的类型。")],
            notes: vec![
                "Set 本身不保存值；它把输入原样输出到变量节点的 Set 口。".into(),
                "Set 在执行线上：先由 Exec 触发，再沿 then 继续。".into(),
            ],
        },
        resolve_outputs,
        run,
    )
    .resolve_inputs(resolve_inputs)
}

fn value_type(params: &Params) -> PortType {
    match params::string(params, PARAM_TYPE, TYPE_NUMBER).as_str() {
        TYPE_TEXT => PortType::Text,
        TYPE_BOOL => PortType::Bool,
        _ => PortType::Number,
    }
}

fn resolve_inputs(params: &Params) -> Vec<PortDef> {
    vec![PortDef::new(PORT_VALUE, "变量输入", value_type(params)).required()]
}

fn resolve_outputs(params: &Params) -> Vec<PortDef> {
    vec![PortDef::new(PORT_OUT, "输出", value_type(params))]
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let value = args.input(PORT_VALUE)?.clone();
    Ok([(PORT_OUT.to_string(), value)].into_iter().collect())
}
