//! 「重命名」节点 —— 给流过的值定个名字，让下游的输出节点照它写盘。
//!
//! 它是**通用**的：流过来的可以是图像，也可以是文本、数字、布尔 —— 都原样透传，
//! 一个字节都不动、类型也不变。它做的唯一一件事，是把名字挂在值上带下去。
//!
//! 「自检测后缀名」决定写盘时要不要按值**实际**的格式补扩展名 ——
//! 下游把图像换成了 JPEG，开着这个选项就会写成 `.jpg`。

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params;
use crate::model::port_type::PortType;
use crate::model::value::{NodeArgs, OutputName, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "rename";

const PARAM_NAME: &str = "name";
const PARAM_AUTO_EXTENSION: &str = "autoExtension";

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "重命名".into(),
            category: "通用".into(),
            description: "给流过的值起个名字，接在它后面的输出节点就用这个名字写盘。\
                          不挑类型：图像、文本、数字都原样透传。"
                .into(),
            is_source: false,
            inputs: vec![PortDef::new("in", "值", PortType::Any)
                .required()
                .hint("任何类型的值")],
            outputs: vec![PortDef::new("out", "值", PortType::Any)],
            params: vec![
                ParamDef::new(
                    PARAM_NAME,
                    "文件名",
                    ParamSpec::Text {
                        default: String::new(),
                        multiline: false,
                        placeholder: Some("例如：hero-idle".into()),
                    },
                )
                .described("不用自己写扩展名，下面那个开关会补。"),
                ParamDef::new(
                    PARAM_AUTO_EXTENSION,
                    "自检测后缀名",
                    ParamSpec::Bool { default: true },
                )
                .described("按值实际的格式自动补上扩展名，例如 .png。"),
            ],
            notes: vec![
                "这一步原样透传：类型、内容、格式都不变，只是给值挂了个名字。".into(),
                "它是通用节点 —— 图像、文本、数字、布尔都能接，将来有别的类型的工具也一样能用。"
                    .into(),
                "开着「自检测后缀名」时，扩展名按实际格式补 —— 下游换成 JPEG 了，\
                 写出来就是 .jpg。"
                    .into(),
                "关掉它，文件名就原样使用：想自己带扩展名，或者干脆不要扩展名，都可以。".into(),
                "文件名留空时下游退回原来的起名方式。".into(),
            ],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // 克隆一个值很便宜（图像只复制一个 Arc），后面还要拿 `args` 记提示。
    let value = args.input("in")?.clone();
    let name = params::string(args.params, PARAM_NAME, "");
    let auto_extension = params::boolean(args.params, PARAM_AUTO_EXTENSION, true);

    let value = match name.trim() {
        "" => {
            args.warn("文件名是空的，下游会沿用原来的名字");
            value.with_name_hint(None)
        }
        stem => value.with_name_hint(Some(OutputName {
            stem: stem.to_string(),
            auto_extension,
        })),
    };

    let mut outputs = ValueMap::new();
    outputs.insert("out".to_string(), value);
    Ok(outputs)
}
