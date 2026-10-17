//! 显式 Exec 入口。无输入，out 触发后续节点，不传数据。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, PortDef};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "start";

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "起点".into(),
            category: "来源".into(),
            description: "工作流的起点标记。没有输入，发出一个空信号，\
                          用来接后续节点的空输入并排先后。"
                .into(),
            // 不接任何输入（入口节点因此也不补「空输入」）。
            inputs: Vec::new(),
            // 一个空信号：接给别的节点的 `-` 空输入，只表示执行先后。
            outputs: vec![PortDef::exec("out", "开始")],
            params: Vec::new(),
            notes: vec!["显式 Exec 入口。无输入，out 触发后续节点，不传数据。".into()],
        },
        run,
    )
    // 新建工作流时默认放置的就是它。
    .entry()
}

fn run(_args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let mut outputs = ValueMap::new();
    outputs.insert("out".to_string(), Value::Void);
    Ok(outputs)
}
