//! 「遍历」节点 —— 对列表里的**每个元素**跑一遍下游那一支。
//!
//! 它只有一个（循环体）输出 `item`：接在它下游的整段子图会**按元素逐个跑一遍**，
//! 每次把当前元素喂给 `item`。结构像：
//!
//! ```text
//! for item in list {
//!     下游那一支（item 是当前元素）
//! }
//! finally { … }
//! ```
//!
//! 另有可选的「结束」输出 —— 所有元素跑完才发出一个空信号，用来接收尾的节点。
//!
//! 「最多跑几个」由参数 `max` 决定，0 表示不限制（把列表跑完为止）。

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
            is_source: false,
            inputs: vec![PortDef::new("list", "列表", PortType::any_list()).required()],
            outputs: vec![
                PortDef::new(ITEM_PORT, "元素", PortType::Any).required(),
                PortDef::new(FINALLY_PORT, "结束", PortType::Void),
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
            notes: vec![
                "接在它下游的整段子图会**按元素逐个跑一遍**，每次「元素」输出就是当前那个。".into(),
                "「最大遍历数目」是 0 时跑完整个列表；大于 0 时最多跑这么多个（列表更短就跑列表那个数）。".into(),
                "「结束」在所有元素跑完之后给出一个空信号 —— 相当于 `finally`，用来接收尾的节点。\
                 它是**可选**的：不接也没关系，但一直在、删不掉。"
                    .into(),
                "元素类型是 `ANY`（编译期不知道列表里装的是什么，运行期再核对）。".into(),
            ],
        },
        run,
    )
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
