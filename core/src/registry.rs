//! 内置工具的注册表。
//!
//! 加一个新工具只需要：写一个 `nodes/xxx.rs`，在这里塞进 [`builtin_specs`]。
//! 前端会自动长出对应的节点，不需要改一行前端代码。

use std::sync::LazyLock;

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, NodeKindInfo, PortDef, ResolveOutputsFn};
use crate::model::params::Params;
use crate::model::value::{NodeArgs, ValueMap};
use crate::nodes;

pub type RunFn = fn(&mut NodeArgs<'_>) -> Result<ValueMap, NodeError>;

/// 这个节点在给定参数下会不会「拦住」运行、等用户操作（紫色节点）。
///
/// 大多数节点恒为 `false`；「图像裁切」只有切到形状模式才会变成紫色。
pub type InteractiveFn = fn(&Params) -> bool;

pub struct NodeSpec {
    pub kind: NodeKind,
    /// 输出端口的现算函数。`None` 表示端口类型写死在 `kind.outputs` 里。
    pub resolve_outputs: Option<ResolveOutputsFn>,
    /// 会不会拦住运行等用户操作。`None` 表示永远不会。
    pub interactive: Option<InteractiveFn>,
    /// 需要某个**得下载的模型**才能跑时，那个「模型」参数的 id。
    /// 界面据此在模型缺失时把节点整个禁用、挂一个下载按钮。
    pub model_param: Option<&'static str>,
    pub run: RunFn,
}

impl NodeSpec {
    pub fn fixed(kind: NodeKind, run: RunFn) -> Self {
        Self {
            kind,
            resolve_outputs: None,
            interactive: None,
            model_param: None,
            run,
        }
    }

    pub fn dynamic(kind: NodeKind, resolve_outputs: ResolveOutputsFn, run: RunFn) -> Self {
        Self {
            kind,
            resolve_outputs: Some(resolve_outputs),
            interactive: None,
            model_param: None,
            run,
        }
    }

    /// 声明这个节点需要一个要下载的模型；`param` 是那个「模型」参数的 id。
    pub fn needs_model(mut self, param: &'static str) -> Self {
        self.model_param = Some(param);
        self
    }

    /// 声明这个节点在某些参数下会拦住运行，等用户操作。
    pub fn interactive(mut self, is_interactive: InteractiveFn) -> Self {
        self.interactive = Some(is_interactive);
        self
    }

    /// 现在这个参数下，它会不会拦住运行。
    pub fn is_interactive(&self, params: &Params) -> bool {
        self.interactive.is_some_and(|is| is(params))
    }

    /// 这个节点在给定参数下的输出端口。
    pub fn outputs_for(&self, params: &Params) -> Vec<PortDef> {
        match self.resolve_outputs {
            Some(resolve) => resolve(params),
            None => self.kind.outputs.clone(),
        }
    }
}

/// 改过 id 的节点。存档里写的是老 id，解析时映射到新 id ——
/// 缺的参数由默认值补上，所以老工作流还能照常打开和运行。
const LEGACY_KIND_IDS: &[(&str, &str)] = &[
    ("convert_to_png", nodes::convert::KIND),
    // 老的「输入」是个按文件读的起点节点，现在归到「读取」上。
    ("input", nodes::read::KIND),
];

/// 把老 id 归一成当前的 id。
pub fn canonical_kind_id(kind_id: &str) -> &str {
    LEGACY_KIND_IDS
        .iter()
        .find(|(old, _)| *old == kind_id)
        .map(|(_, current)| *current)
        .unwrap_or(kind_id)
}

pub struct Registry {
    specs: Vec<NodeSpec>,
}

impl Registry {
    pub fn get(&self, kind_id: &str) -> Option<&NodeSpec> {
        let canonical = canonical_kind_id(kind_id);
        self.specs.iter().find(|spec| spec.kind.id == canonical)
    }

    pub fn all(&self) -> &[NodeSpec] {
        &self.specs
    }

    /// 某个节点实例在给定参数下会不会拦住运行（画布据此决定边线画不画成紫色）。
    pub fn is_interactive(&self, kind_id: &str, params: &Params) -> bool {
        self.get(kind_id)
            .is_some_and(|spec| spec.is_interactive(params))
    }

    /// 交给前端的工具清单。
    ///
    /// 端口发的是**声明里的样子**（静态的）—— 节点库卡片是个**模板**：动态输出的节点
    /// （输入 / 图像格式转换 / 图像压缩）在卡片上显示泛化的类型（`ANY` / `IMG`），
    /// 具体类型要等拖出来、定下参数才知道。
    pub fn kinds(&self) -> Vec<NodeKindInfo> {
        self.specs
            .iter()
            .map(|spec| {
                let defaults = spec.kind.default_params();
                NodeKindInfo {
                    kind: spec.kind.clone(),
                    interactive: spec.is_interactive(&defaults),
                    model_param: spec.model_param.map(str::to_string),
                    defaults,
                }
            })
            .collect()
    }
}

fn builtin_specs() -> Vec<NodeSpec> {
    vec![
        nodes::read::spec(),
        nodes::literal::text_spec(),
        nodes::literal::number_spec(),
        nodes::literal::bool_spec(),
        nodes::convert::spec(),
        nodes::compress::spec(),
        nodes::border::spec(),
        nodes::remove_color::spec(),
        nodes::background_removal::spec(),
        nodes::palette::spec(),
        nodes::crop::spec(),
        nodes::transform::spec(),
        nodes::upscale::spec(),
        nodes::rename::spec(),
        nodes::save::spec(),
    ]
}

pub fn registry() -> &'static Registry {
    static REGISTRY: LazyLock<Registry> = LazyLock::new(|| Registry {
        specs: builtin_specs(),
    });
    &REGISTRY
}
