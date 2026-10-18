//! 同步 Sequence：按数字输出序号执行，每个完整分支返回后才开始下一分支。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params::{self, Params};
use crate::model::value::{NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "sequence";

/// 最多允许几个输出 —— 手滑点多了也不至于撑爆版面。
pub const MAX_OUTPUTS: usize = 32;

/// 那个「动态输出」参数在参数表里的 id。
const PARAM_OUTS: &str = "outs";

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "顺序执行".into(),
            category: "逻辑".into(),
            description: "把接到各个输出上的分支按从上到下的顺序依次执行。\
                          给本来互不相干的分支（比如同时往不同目录写文件）定个先后。"
                .into(),
            // 没有数据输入 —— 会自动补一个「空输入」，可挂到别的逻辑输出上。
            inputs: vec![],
            // 声明里给一个泛化的样子（节点库卡片用）；真正的个数由 `output_ports` 现算。
            outputs: vec![PortDef::exec("out1", "1")],
            params: vec![ParamDef::new(
                PARAM_OUTS,
                "输出",
                ParamSpec::Ports {
                    default: 1.0,
                    min: 1.0,
                    max: MAX_OUTPUTS as f64,
                    label_prefix: "branch".into(),
                    default_label: "分支".into(),
                },
            )
            .described("接在这个输出上的分支会先跑。左侧文本只是分支注释，不参与运行。")],
            notes: vec![
                "同步 Sequence：按数字输出序号执行，每个完整分支返回后才开始下一分支。".into(),
            ],
        },
        output_ports,
        run,
    )
    .control_flow(crate::registry::ControlFlow::Sequence {
        output_prefix: "out".into(),
    })
    // 逻辑类：边线走暗蓝 —— 「这类节点长这样」。
    .accent(super::LOGIC_ACCENT)
}

/// 按参数里的个数现算这一排输出端口：`out1`（必须）+ `out2` .. `outN`（可选）。
fn output_ports(params: &Params) -> Vec<PortDef> {
    let count = params::integer(params, PARAM_OUTS, 1).clamp(1, MAX_OUTPUTS as i64) as usize;
    (1..=count)
        .map(|k| {
            // 输出画在参数栏的「输出」控件那一行上（0 起）—— 节点顶部因此没有输出。
            let def = PortDef::exec(&format!("out{k}"), &k.to_string()).on_panel_row(k - 1);
            // 第一个是「必须输出」，删不掉；其余是可选的。
            if k == 1 {
                def.required()
            } else {
                def
            }
        })
        .collect()
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    let count = params::integer(args.params, PARAM_OUTS, 1).clamp(1, MAX_OUTPUTS as i64) as usize;
    let mut outputs = ValueMap::new();
    for k in 1..=count {
        outputs.insert(format!("out{k}"), Value::Void);
    }
    Ok(outputs)
}
