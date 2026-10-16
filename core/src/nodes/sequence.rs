//! 「顺序执行」节点 —— 给本来彼此独立的分支排一个先后。
//!
//! 它自己不需要任何数据输入（因此自动带一个「空输入」，可以挂到别的逻辑输出上）。
//! 输出是一排**空信号**：接到哪个分支上，那个分支就归到这个序号下；引擎保证序号小的
//! 分支先跑（见 `engine::topological_order` 里加的排序约束）。
//!
//! 输出的个数是动态的 —— 由 `ParamSpec::Ports` 这个参数声明（界面上是一列带序号的
//! 输出 + 每行的删除按钮 + 底部的加号），节点不再手写这一排端口。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params::{self, Params};
use crate::model::port_type::PortType;
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
            is_source: false,
            // 没有数据输入 —— 会自动补一个「空输入」，可挂到别的逻辑输出上。
            inputs: vec![],
            // 声明里给一个泛化的样子（节点库卡片用）；真正的个数由 `output_ports` 现算。
            outputs: vec![PortDef::new("out1", "1", PortType::Void)],
            params: vec![ParamDef::new(
                PARAM_OUTS,
                "输出",
                ParamSpec::Ports {
                    default: 1.0,
                    min: 1.0,
                    max: MAX_OUTPUTS as f64,
                    blocking: true,
                    blocking_prefix: "block".into(),
                },
            )
            .described("接在这个输出上的分支会先跑。按加号增加一个输出，从上到下依次执行。")],
            notes: vec![
                "每个输出都是一个**空信号** —— 不传数据，只用来排先后。".into(),
                "把「保存到目录」「读取」这类**不需要数据输入**的节点接到对应的空输入上，\
                 它们的执行顺序就由这里的序号说了算。"
                    .into(),
                "序号小的先跑；挂在同一个输出上的几个分支之间不再分先后。".into(),
                "每一行左边有一个**阻塞开关**：开着时这一段跑完后面的才轮到；关掉就不施加\
                 任何先后约束（可以拿它让某几段「一起跑」）。"
                    .into(),
            ],
        },
        output_ports,
        run,
    )
    .control_flow(crate::registry::ControlFlow::Sequence {
        output_prefix: "out".into(),
        blocking_prefix: "block".into(),
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
            let def = PortDef::new(&format!("out{k}"), &k.to_string(), PortType::Void)
                .on_panel_row(k - 1);
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
