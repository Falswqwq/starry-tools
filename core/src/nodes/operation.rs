//! 「运算」节点 —— 两个对象的基本运算。
//!
//! 数值运算（加 / 减 / 乘 / 除 / 取余 / 取整）要求两个输入都是数字；字符串拼接要求
//! 两个输入都是文本。输入与输出的类型跟着运算方式走 —— 由 `resolve_inputs` /
//! `resolve_outputs` 现算，所以界面上切到「字符串拼接」时端口会立刻变成文本。
//!
//! 运算不是纯表达式，而是显式调度节点：由 Exec 触发，跑完后沿 `then` 继续。
//! 这样运算结果写变量时能明确排在执行线上，不再依赖“被谁拉取才求值”。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params::{self, Params};
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "operation";

const PARAM_OP: &str = "op";

const OP_ADD: &str = "add";
const OP_SUBTRACT: &str = "subtract";
const OP_MULTIPLY: &str = "multiply";
const OP_DIVIDE: &str = "divide";
const OP_REMAINDER: &str = "remainder";
const OP_TRUNCATE: &str = "truncate";
const OP_CONCAT: &str = "concat";

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "运算".into(),
            category: "变量".into(),
            description: "对两个对象做基本运算：加减乘除、取余、取整，或把两段文本拼起来。".into(),
            // 节点库卡片上的模板：默认是数值加法；真实端口由 `op` 现算。
            inputs: vec![
                PortDef::new("a", "A", PortType::Number).required(),
                PortDef::new("b", "B", PortType::Number).required(),
            ],
            outputs: vec![PortDef::new("out", "结果", PortType::Number)],
            params: vec![ParamDef::new(
                PARAM_OP,
                "运算",
                ParamSpec::Select {
                    default: OP_ADD.into(),
                    options: vec![
                        SelectOption::new(OP_ADD, "加").hint("A + B"),
                        SelectOption::new(OP_SUBTRACT, "减").hint("A − B"),
                        SelectOption::new(OP_MULTIPLY, "乘").hint("A × B"),
                        SelectOption::new(OP_DIVIDE, "除").hint("A ÷ B（浮点）"),
                        SelectOption::new(OP_REMAINDER, "取余").hint("A 除以 B 的余数"),
                        SelectOption::new(OP_TRUNCATE, "取整").hint("A ÷ B 的整数部分"),
                        SelectOption::new(OP_CONCAT, "字符串拼接").hint("把两段文本接起来"),
                    ],
                },
            )
            .described("数值运算要求两个输入都是数字；字符串拼接要求两个都是文本。")],
            notes: vec![
                "两个输入都是**必填**的 —— 它们就是运算的两个对象。".into(),
                "选「字符串拼接」时，两个输入和输出都会变成文本；其余运算都是数字。".into(),
                "除数为 0 会报错，不会静默产出无穷大。".into(),
            ],
        },
        resolve_outputs,
        run,
    )
    .resolve_inputs(resolve_inputs)
}

/// 这个节点当前是不是字符串拼接（决定端口是文本还是数字）。
fn is_concat(params: &Params) -> bool {
    params::string(params, PARAM_OP, OP_ADD) == OP_CONCAT
}

fn operand_type(params: &Params) -> PortType {
    if is_concat(params) {
        PortType::Text
    } else {
        PortType::Number
    }
}

fn resolve_inputs(params: &Params) -> Vec<PortDef> {
    let ty = operand_type(params);
    vec![
        PortDef::new("a", "A", ty).required(),
        PortDef::new("b", "B", ty).required(),
    ]
}

fn resolve_outputs(params: &Params) -> Vec<PortDef> {
    vec![PortDef::new("out", "结果", operand_type(params))]
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let op = params::string(args.params, PARAM_OP, OP_ADD);
    let mut outputs = ValueMap::new();

    if op == OP_CONCAT {
        let text = format!(
            "{}{}",
            args.input("a")?.as_text()?,
            args.input("b")?.as_text()?
        );
        outputs.insert("out".to_string(), Value::text(text));
        return Ok(outputs);
    }

    let a = args.input("a")?.as_number()?;
    let b = args.input("b")?.as_number()?;
    let value = match op.as_str() {
        OP_ADD => a + b,
        OP_SUBTRACT => a - b,
        OP_MULTIPLY => a * b,
        OP_DIVIDE => divide(a, b)?,
        OP_REMAINDER => remainder(a, b)?,
        OP_TRUNCATE => divide(a, b)?.trunc(),
        // 认不出的 op（老存档 / 手改的 JSON）按默认的加法处理，不悄悄输出别的东西。
        _ => a + b,
    };
    outputs.insert("out".to_string(), Value::Number(value));
    Ok(outputs)
}

fn divide(a: f64, b: f64) -> Result<f64, NodeError> {
    if b == 0.0 {
        return Err(NodeError::new("除数不能为 0"));
    }
    Ok(a / b)
}

fn remainder(a: f64, b: f64) -> Result<f64, NodeError> {
    if b == 0.0 {
        return Err(NodeError::new("除数不能为 0"));
    }
    Ok(a % b)
}
