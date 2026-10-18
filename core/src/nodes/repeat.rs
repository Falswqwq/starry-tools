//! 同步 Repeat：把下游分支重复执行固定次数。
//!
//! 每轮从 `index` 输出当前序号（0 起）；全部成功后走 `finally`。
//! 和「遍历」一样，数据连线不扩张循环作用域。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "repeat";

/// 引擎用来认出「当前轮次」的数据输出。
pub const INDEX_PORT: &str = "index";
/// 循环体执行输出。
pub const BODY_PORT: &str = "body";
/// 全部完成后的执行输出。
pub const FINALLY_PORT: &str = "finally";
/// 次数参数 id。
pub const COUNT_PARAM: &str = "times";

/// 上限与参数控件保持一致，异常存档也不会让界面卡死。
pub const MAX_REPEAT: i64 = 10_000;

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "重复执行".into(),
            category: "逻辑".into(),
            description: "把接在循环体上的分支重复执行指定次数。每轮输出当前序号，\
                          全部完成后走结束。"
                .into(),
            inputs: vec![],
            outputs: vec![
                PortDef::exec(BODY_PORT, "循环体"),
                PortDef::new(INDEX_PORT, "索引", PortType::Number),
                PortDef::exec(FINALLY_PORT, "结束"),
            ],
            params: vec![ParamDef::new(
                COUNT_PARAM,
                "次数",
                ParamSpec::Number {
                    default: 3.0,
                    min: 0.0,
                    max: MAX_REPEAT as f64,
                    step: 1.0,
                    integer: true,
                    unit: Some("次".into()),
                },
            )
            .described("重复多少轮；填 0 表示只走结束，不执行循环体。")],
            notes: vec![
                "同步 Repeat：每轮从索引输出当前序号（0 起），全部完成后走结束。".into(),
                "某一轮失败会停止后续轮次，不再触发结束。".into(),
            ],
        },
        run,
    )
    .control_flow(crate::registry::ControlFlow::Repeat {
        count_param: COUNT_PARAM.into(),
        index_output: INDEX_PORT.into(),
        body_output: BODY_PORT.into(),
        finally_output: FINALLY_PORT.into(),
    })
    .accent(super::LOGIC_ACCENT)
}

/// 引擎自己按次数跑循环体；这里是没有下游时的兜底：直接把结束发出去。
fn run(_args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    Ok([(FINALLY_PORT.to_string(), Value::Void)]
        .into_iter()
        .collect())
}
