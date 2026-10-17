//! 同步 Loop：exec 输入、body 执行输出、item 数据输出及 finally 完成输出。数据连线不扩张循环作用域。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "iterate";

/// 循环体输出端口的 id。引擎按它认出「这是喂给循环体的当前元素」。
pub const ITEM_PORT: &str = "item";
/// 「结束」输出端口的 id。
pub const FINALLY_PORT: &str = "finally";

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "遍历".into(),
            category: "逻辑".into(),
            description: "对列表里的每个元素跑一遍下游那一支：接在它后面的整段子图会\
                          逐个元素执行一次，当前元素从「元素」这个输出喂进去。"
                .into(),
            inputs: vec![PortDef::new("list", "列表", PortType::any_list()).required()],
            outputs: vec![
                PortDef::exec("body", "循环体"),
                PortDef::new(ITEM_PORT, "元素", PortType::Any).required(),
                PortDef::exec(FINALLY_PORT, "结束"),
            ],
            params: vec![ParamDef::new(
                "max",
                "最大遍历数目",
                ParamSpec::Number {
                    default: 0.0,
                    min: 0.0,
                    max: 1_000_000.0,
                    step: 1.0,
                    integer: true,
                    unit: Some("个".into()),
                },
            )
            .described("最多跑多少个元素。填 0 表示不限制，把列表跑完为止。")],
            notes: vec!["同步 Loop：exec 输入、body 执行输出、item 数据输出及 finally 完成输出。数据连线不扩张循环作用域。".into()],
        },
        run,
    )
    .control_flow(crate::registry::ControlFlow::Loop {
        list_input: "list".into(),
        item_output: ITEM_PORT.into(),
        body_output: "body".into(),
        finally_output: FINALLY_PORT.into(),
        max_param: "max".into(),
    })
    // 逻辑类：边线走暗蓝 —— 「这类节点长这样」。
    .accent(super::LOGIC_ACCENT)
}

/// 引擎其实不用这个函数的返回值来跑循环体 —— 它自己按元素逐个跑。这里给的是一份
/// 合理的**兜底**：没有循环体要跟时，至少把「结束」发出去。
fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let mut outputs = ValueMap::new();
    outputs.insert(FINALLY_PORT.to_string(), Value::Void);
    // 有元素的话顺手把第一个放进 item，单独跑这个节点时也不至于空着。
    if let Ok(items) = args.input("list").and_then(Value::as_list) {
        if let Some(first) = items.first() {
            outputs.insert(ITEM_PORT.to_string(), first.clone());
        }
    }
    Ok(outputs)
}
