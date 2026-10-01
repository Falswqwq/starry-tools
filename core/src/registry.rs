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

pub struct NodeSpec {
    pub kind: NodeKind,
    /// 输出端口的现算函数。`None` 表示端口类型写死在 `kind.outputs` 里。
    pub resolve_outputs: Option<ResolveOutputsFn>,
    pub run: RunFn,
}

impl NodeSpec {
    pub fn fixed(kind: NodeKind, run: RunFn) -> Self {
        Self {
            kind,
            resolve_outputs: None,
            run,
        }
    }

    pub fn dynamic(kind: NodeKind, resolve_outputs: ResolveOutputsFn, run: RunFn) -> Self {
        Self {
            kind,
            resolve_outputs: Some(resolve_outputs),
            run,
        }
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
    // 「输入」拆成了「读取」和「输入框」两个节点。老的「输入」按文件读，
    // 归到「读取」上最接近。
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

    /// 交给前端的工具清单。
    ///
    /// 端口发的是**声明里的样子**（静态的）—— 节点库卡片是个**模板**：动态输出的节点
    /// （输入 / 图像格式转换 / 图像压缩）在卡片上显示泛化的类型（`ANY` / `IMG`），
    /// 具体类型要等拖出来、定下参数才知道。
    pub fn kinds(&self) -> Vec<NodeKindInfo> {
        self.specs
            .iter()
            .map(|spec| NodeKindInfo {
                kind: spec.kind.clone(),
                defaults: spec.kind.default_params(),
            })
            .collect()
    }
}

fn builtin_specs() -> Vec<NodeSpec> {
    vec![
        nodes::read::spec(),
        nodes::input_box::spec(),
        nodes::literal::text_spec(),
        nodes::literal::number_spec(),
        nodes::literal::bool_spec(),
        nodes::convert::spec(),
        nodes::compress::spec(),
        nodes::crop::spec(),
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
