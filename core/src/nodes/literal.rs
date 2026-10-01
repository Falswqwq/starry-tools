//! 「字面量」节点 —— 文本 / 数字 / 布尔。
//!
//! 它们是**基本类型**的源头：一个节点就装一个值，输出那个值。就是普通的节点样式
//! （标题栏 + 端口列 + 参数区），只是没有输入。
//!
//! 这类节点的意义在于：参数端口（把参数改从上游取）需要一个「值从哪来」的地方，
//! 而这些节点就是那个地方 —— 比如把「重命名」的文件名接到一个文本节点上。
//!
//! 三个节点共用同一套定义，只有控件的种类和输出的类型不同，所以写成一个函数生成。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params;
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND_TEXT: &str = "literal_text";
pub const KIND_NUMBER: &str = "literal_number";
pub const KIND_BOOL: &str = "literal_bool";

const PARAM_VALUE: &str = "value";

pub fn text_spec() -> NodeSpec {
    NodeSpec::fixed(
        literal(
            KIND_TEXT,
            "文本",
            "一段文本。接到别的节点的参数端口上，就能把那个参数改从这里取。",
            ParamSpec::Text {
                default: String::new(),
                multiline: false,
                placeholder: Some("写点什么…".into()),
            },
            PortType::Text,
        ),
        run_text,
    )
}

pub fn number_spec() -> NodeSpec {
    NodeSpec::fixed(
        literal(
            KIND_NUMBER,
            "数字",
            "一个数字。接到别的节点的参数端口上，就能把那个参数改从这里取。",
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
}

pub fn bool_spec() -> NodeSpec {
    NodeSpec::fixed(
        literal(
            KIND_BOOL,
            "布尔",
            "真或假。接到别的节点的参数端口上，就能把那个开关改从这里取。",
            ParamSpec::Bool { default: false },
            PortType::Bool,
        ),
        run_bool,
    )
}

fn literal(id: &str, name: &str, description: &str, spec: ParamSpec, output: PortType) -> NodeKind {
    NodeKind {
        id: id.into(),
        name: name.into(),
        category: "来源".into(),
        description: description.into(),
        is_source: true,
        // 起点节点：没有输入端口；输出端口的类型就是它装的那种基本类型。
        inputs: Vec::new(),
        // 端口名和参数名都叫「值」—— 节点名（标题栏）已经说明是什么了，不重复一遍。
        outputs: vec![PortDef::new("out", "值", output)],
        params: vec![ParamDef::new(PARAM_VALUE, "", spec)],
        notes: vec![
            "它本身就是一个值，所以没有输入，也不用配置什么。".into(),
            "接到别的节点的**参数端口**（参数名前面那个小圆点）上，\
             那个参数就会改从这里取，原来那个输入组件会变成不可用。"
                .into(),
        ],
    }
}

fn run_text(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let text = params::string(args.params, PARAM_VALUE, "");
    let mut outputs = ValueMap::new();
    outputs.insert("out".to_string(), Value::text(text));
    Ok(outputs)
}

fn run_number(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let number = params::number(args.params, PARAM_VALUE, 0.0);
    let mut outputs = ValueMap::new();
    outputs.insert("out".to_string(), Value::Number(number));
    Ok(outputs)
}

fn run_bool(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let value = params::boolean(args.params, PARAM_VALUE, false);
    let mut outputs = ValueMap::new();
    outputs.insert("out".to_string(), Value::Bool(value));
    Ok(outputs)
}
