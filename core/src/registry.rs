//! 启动期注册、随后冻结的节点与扩展元数据。
//!
//! 扩展先向 `RegistryBuilder` 贡献类型、类别、工具与节点，再 `freeze` / `install`。
//! 执行回调可以捕获拥有的状态；这里不加载或执行任何脚本。

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use serde::Serialize;

use crate::error::NodeError;
use crate::model::node_kind::{NodeKind, NodeKindInfo, PortDef, ResolveOutputsFn};
use crate::model::params::Params;
use crate::model::value::{NodeArgs, ValueMap};
use crate::types::{self, TypeColor};

pub type RunFn = Arc<dyn Fn(&mut NodeArgs<'_>) -> Result<ValueMap, NodeError> + Send + Sync>;
pub type InteractiveFn = Arc<dyn Fn(&Params) -> bool + Send + Sync>;

/// 引擎消费的控制流协议，不依赖任何内置节点 id。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ControlFlow {
    Sequence {
        output_prefix: String,
        blocking_prefix: String,
    },
    Loop {
        list_input: String,
        item_output: String,
        finally_output: String,
        max_param: String,
    },
}

pub struct NodeSpec {
    pub kind: NodeKind,
    pub resolve_outputs: Option<ResolveOutputsFn>,
    pub interactive: Option<InteractiveFn>,
    pub model_param: Option<String>,
    pub requires_tool: Option<String>,
    pub control_flow: Option<ControlFlow>,
    pub run: RunFn,
}

impl NodeSpec {
    pub fn fixed(
        kind: NodeKind,
        run: impl Fn(&mut NodeArgs<'_>) -> Result<ValueMap, NodeError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            resolve_outputs: None,
            interactive: None,
            model_param: None,
            requires_tool: None,
            control_flow: None,
            run: Arc::new(run),
        }
    }

    pub fn dynamic(
        kind: NodeKind,
        resolve_outputs: impl Fn(&Params) -> Vec<PortDef> + Send + Sync + 'static,
        run: impl Fn(&mut NodeArgs<'_>) -> Result<ValueMap, NodeError> + Send + Sync + 'static,
    ) -> Self {
        let mut spec = Self::fixed(kind, run);
        spec.resolve_outputs = Some(Arc::new(resolve_outputs));
        spec
    }

    pub fn needs_model(mut self, param: impl Into<String>) -> Self {
        self.model_param = Some(param.into());
        self
    }

    pub fn needs_tool(mut self, tool: impl Into<String>) -> Self {
        self.requires_tool = Some(tool.into());
        self
    }

    pub fn interactive(
        mut self,
        is_interactive: impl Fn(&Params) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.interactive = Some(Arc::new(is_interactive));
        self
    }

    pub fn control_flow(mut self, control_flow: ControlFlow) -> Self {
        self.control_flow = Some(control_flow);
        self
    }

    pub fn is_interactive(&self, params: &Params) -> bool {
        self.interactive.as_ref().is_some_and(|is| is(params))
    }

    pub fn outputs_for(&self, params: &Params) -> Vec<PortDef> {
        match &self.resolve_outputs {
            Some(resolve) => resolve(params),
            None => self.kind.outputs.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
    pub id: String,
    pub label: String,
    /// 类别边线色。没有专属颜色时由 UI 使用状态色。
    pub color: Option<TypeColor>,
}

impl CategoryInfo {
    pub fn new(id: impl Into<String>, label: impl Into<String>, color: Option<TypeColor>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            color,
        }
    }
}

pub struct ToolSpec {
    pub id: String,
    pub probe: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl ToolSpec {
    pub fn new(id: impl Into<String>, probe: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            id: id.into(),
            probe: Arc::new(probe),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistrationError {
    #[error("duplicate {kind} id: {id}")]
    Duplicate { kind: &'static str, id: String },
    #[error("unknown {kind} id: {id}")]
    Unknown { kind: &'static str, id: String },
    #[error("cyclic type interface: {0}")]
    TypeCycle(String),
    #[error("the global registry is already initialized")]
    AlreadyInitialized,
}

/// 只有构建器能登记；`freeze` 消耗它，得到没有写入 API 的注册表。
#[derive(Default)]
pub struct RegistryBuilder {
    types: types::RegistryBuilder,
    specs: Vec<NodeSpec>,
    categories: Vec<CategoryInfo>,
    tools: BTreeMap<String, ToolSpec>,
    aliases: BTreeMap<String, String>,
}

impl RegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins() -> Result<Self, RegistrationError> {
        let mut builder = Self::new();
        types::register_builtin(builder.types_mut())?;
        crate::nodes::register(&mut builder)?;
        Ok(builder)
    }

    pub fn types_mut(&mut self) -> &mut types::RegistryBuilder {
        &mut self.types
    }

    pub fn register_node(&mut self, spec: NodeSpec) -> Result<(), RegistrationError> {
        let id = &spec.kind.id;
        if self.specs.iter().any(|entry| entry.kind.id == *id) || self.aliases.contains_key(id) {
            return Err(RegistrationError::Duplicate {
                kind: "node",
                id: id.clone(),
            });
        }
        self.specs.push(spec);
        Ok(())
    }

    pub fn register_category(&mut self, category: CategoryInfo) -> Result<(), RegistrationError> {
        if self.categories.iter().any(|entry| entry.id == category.id) {
            return Err(RegistrationError::Duplicate {
                kind: "category",
                id: category.id,
            });
        }
        self.categories.push(category);
        Ok(())
    }

    pub fn register_tool(&mut self, tool: ToolSpec) -> Result<(), RegistrationError> {
        if self.tools.contains_key(&tool.id) {
            return Err(RegistrationError::Duplicate {
                kind: "tool",
                id: tool.id,
            });
        }
        self.tools.insert(tool.id.clone(), tool);
        Ok(())
    }

    /// 别名直接指向正式节点 id，不允许别名链。
    pub fn register_alias(
        &mut self,
        alias: impl Into<String>,
        target: impl Into<String>,
    ) -> Result<(), RegistrationError> {
        let alias = alias.into();
        if self.aliases.contains_key(&alias)
            || self.specs.iter().any(|entry| entry.kind.id == alias)
        {
            return Err(RegistrationError::Duplicate {
                kind: "node alias",
                id: alias,
            });
        }
        self.aliases.insert(alias, target.into());
        Ok(())
    }

    pub fn freeze(self) -> Result<Registry, RegistrationError> {
        for spec in &self.specs {
            if !self
                .categories
                .iter()
                .any(|entry| entry.id == spec.kind.category)
            {
                return Err(RegistrationError::Unknown {
                    kind: "category",
                    id: spec.kind.category.clone(),
                });
            }
        }
        for target in self.aliases.values() {
            if !self.specs.iter().any(|entry| entry.kind.id == *target) {
                return Err(RegistrationError::Unknown {
                    kind: "node alias target",
                    id: target.clone(),
                });
            }
        }
        Ok(Registry {
            types: self.types.freeze()?,
            specs: self.specs,
            categories: self.categories,
            tools: self.tools,
            aliases: self.aliases,
        })
    }
}

pub struct Registry {
    types: types::Registry,
    specs: Vec<NodeSpec>,
    categories: Vec<CategoryInfo>,
    tools: BTreeMap<String, ToolSpec>,
    aliases: BTreeMap<String, String>,
}

impl Registry {
    pub fn types(&self) -> &types::Registry {
        &self.types
    }
    pub fn categories(&self) -> &[CategoryInfo] {
        &self.categories
    }
    pub fn category(&self, id: &str) -> Option<&CategoryInfo> {
        self.categories.iter().find(|category| category.id == id)
    }
    pub fn tool(&self, id: &str) -> Option<&ToolSpec> {
        self.tools.get(id)
    }
    /// 未登记的工具不是可用工具。
    pub fn tool_available(&self, id: &str) -> bool {
        self.tool(id).is_some_and(|tool| (tool.probe)())
    }
    pub fn canonical_kind_id<'a>(&'a self, id: &'a str) -> &'a str {
        self.aliases.get(id).map(String::as_str).unwrap_or(id)
    }
    pub fn get(&self, kind_id: &str) -> Option<&NodeSpec> {
        let canonical = self.canonical_kind_id(kind_id);
        self.specs.iter().find(|spec| spec.kind.id == canonical)
    }
    pub fn all(&self) -> &[NodeSpec] {
        &self.specs
    }
    pub fn is_interactive(&self, kind_id: &str, params: &Params) -> bool {
        self.get(kind_id)
            .is_some_and(|spec| spec.is_interactive(params))
    }
    pub fn kinds(&self) -> Vec<NodeKindInfo> {
        self.specs
            .iter()
            .map(|spec| {
                let defaults = spec.kind.default_params();
                NodeKindInfo {
                    kind: spec.kind.clone(),
                    interactive: spec.is_interactive(&defaults),
                    model_param: spec.model_param.clone(),
                    requires_tool: spec.requires_tool.clone(),
                    control_flow: spec.control_flow.clone(),
                    defaults,
                }
            })
            .collect()
    }
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// 必须在任何全局元数据查询之前调用；冻结后不支持热注册。
pub fn install(registry: Registry) -> Result<(), RegistrationError> {
    REGISTRY
        .set(registry)
        .map_err(|_| RegistrationError::AlreadyInitialized)
}

pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| {
        RegistryBuilder::with_builtins()
            .and_then(RegistryBuilder::freeze)
            .expect("invalid builtin registry")
    })
}

pub fn canonical_kind_id(kind_id: &str) -> &str {
    registry().canonical_kind_id(kind_id)
}
pub fn tool_available(name: &str) -> bool {
    registry().tool_available(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::port_type::PortType;
    use crate::model::value::{one_output, Value};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn kind(id: &str) -> NodeKind {
        NodeKind {
            id: id.into(),
            name: format!("动态 {id}"),
            category: "extension".into(),
            description: String::from("拥有字符串的扩展节点"),
            is_source: true,
            inputs: vec![],
            outputs: vec![PortDef::new("out", "值", PortType::Text)],
            params: vec![],
            notes: vec![format!("{id} 的动态注释")],
        }
    }

    #[test]
    fn extensions_keep_owned_metadata_and_captured_callbacks() {
        let mut builder = RegistryBuilder::with_builtins().unwrap();
        let category = String::from("extension");
        builder
            .register_category(CategoryInfo::new(
                category.clone(),
                "扩展",
                Some(TypeColor::rgb(3, 5, 7)),
            ))
            .unwrap();
        let available = Arc::new(AtomicBool::new(false));
        let probe_state = available.clone();
        builder
            .register_tool(ToolSpec::new(String::from("extension-tool"), move || {
                probe_state.load(Ordering::SeqCst)
            }))
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = calls.clone();
        let output_label = String::from("动态输出");
        let captured_value = String::from("captured");
        let interactive_key = String::from("pause");
        let spec = NodeSpec::dynamic(
            kind("extension/node"),
            move |_| vec![PortDef::new("out", &output_label, PortType::Text)],
            move |_| {
                callback_calls.fetch_add(1, Ordering::SeqCst);
                Ok(one_output("out", Value::text(captured_value.clone())))
            },
        )
        .interactive(move |params| params.contains_key(&interactive_key))
        .needs_tool(String::from("extension-tool"));
        builder.register_node(spec).unwrap();
        builder
            .register_alias(
                String::from("old-extension"),
                String::from("extension/node"),
            )
            .unwrap();
        drop(category);
        let registry = builder.freeze().unwrap();
        let spec = registry.get("old-extension").unwrap();
        assert_eq!(spec.kind.notes, ["extension/node 的动态注释"]);
        assert_eq!(registry.category("extension").unwrap().label, "扩展");
        assert_eq!(
            serde_json::to_value(registry.category("extension").unwrap()).unwrap()["color"],
            serde_json::json!([3, 5, 7])
        );
        assert!(!registry.tool_available("extension-tool"));
        available.store(true, Ordering::SeqCst);
        assert!(registry.tool_available("extension-tool"));
        assert!(!registry.tool_available("unknown"));
        let mut params = Params::new();
        params.insert("pause".into(), serde_json::json!(true));
        assert!(spec.is_interactive(&params));
        assert_eq!(spec.outputs_for(&params)[0].label, "动态输出");
        let inputs = ValueMap::new();
        let mut warnings = vec![];
        let mut args = NodeArgs {
            node_id: "test",
            node_name: "测试",
            params: &params,
            inputs: &inputs,
            warnings: &mut warnings,
            interaction: None,
            progress: None,
        };
        assert_eq!(
            (spec.run)(&mut args).unwrap()["out"].as_text().unwrap(),
            "captured"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(registry
            .kinds()
            .iter()
            .any(|info| info.kind.id == "extension/node"));
    }

    #[test]
    fn every_registration_namespace_rejects_duplicates() {
        let mut builder = RegistryBuilder::new();
        builder
            .register_category(CategoryInfo::new("extension", "扩展", None))
            .unwrap();
        assert!(matches!(
            builder.register_category(CategoryInfo::new("extension", "替换", None)),
            Err(RegistrationError::Duplicate {
                kind: "category",
                ..
            })
        ));
        builder
            .register_tool(ToolSpec::new("tool", || false))
            .unwrap();
        assert!(matches!(
            builder.register_tool(ToolSpec::new("tool", || true)),
            Err(RegistrationError::Duplicate { kind: "tool", .. })
        ));
        builder
            .register_node(NodeSpec::fixed(kind("node"), |_| Ok(ValueMap::new())))
            .unwrap();
        assert!(matches!(
            builder.register_node(NodeSpec::fixed(kind("node"), |_| Ok(ValueMap::new()))),
            Err(RegistrationError::Duplicate { kind: "node", .. })
        ));
        builder.register_alias("old", "node").unwrap();
        assert!(builder.register_alias("old", "node").is_err());
        assert!(builder.register_alias("node", "node").is_err());
        assert!(builder
            .register_node(NodeSpec::fixed(kind("old"), |_| Ok(ValueMap::new())))
            .is_err());
        let registry = builder.freeze().unwrap();
        assert!(
            !registry.tool_available("tool"),
            "重复注册没有覆盖原有探测器"
        );
        assert_eq!(registry.category("extension").unwrap().label, "扩展");
    }

    #[test]
    fn freeze_rejects_missing_category_and_alias_target() {
        let mut builder = RegistryBuilder::new();
        builder
            .register_node(NodeSpec::fixed(kind("node"), |_| Ok(ValueMap::new())))
            .unwrap();
        assert!(matches!(
            builder.freeze(),
            Err(RegistrationError::Unknown {
                kind: "category",
                ..
            })
        ));
        let mut builder = RegistryBuilder::new();
        builder.register_alias("old", "missing").unwrap();
        assert!(matches!(
            builder.freeze(),
            Err(RegistrationError::Unknown {
                kind: "node alias target",
                ..
            })
        ));
    }

    #[test]
    fn builtin_metadata_and_legacy_ids_are_preserved() {
        let registry = RegistryBuilder::with_builtins().unwrap().freeze().unwrap();
        assert_eq!(
            registry.get("input").unwrap().kind.id,
            crate::nodes::read::KIND
        );
        assert_eq!(
            registry.get("convert_to_png").unwrap().kind.id,
            crate::nodes::convert::KIND
        );
        assert_eq!(
            registry.category("逻辑").unwrap().color,
            Some(TypeColor::rgb(0x1e, 0x40, 0xaf))
        );
        for spec in registry.all() {
            assert!(registry.category(&spec.kind.category).is_some());
        }
        assert_eq!(
            registry.get("sequence").unwrap().control_flow,
            Some(ControlFlow::Sequence {
                output_prefix: "out".into(),
                blocking_prefix: "block".into(),
            })
        );
        assert_eq!(
            registry.get("iterate").unwrap().control_flow,
            Some(ControlFlow::Loop {
                list_input: "list".into(),
                item_output: "item".into(),
                finally_output: "finally".into(),
                max_param: "max".into(),
            })
        );
        assert!(!registry.tool_available("not-a-registered-tool"));
    }

    #[test]
    fn optional_features_contribute_only_when_enabled() {
        let registry = RegistryBuilder::with_builtins().unwrap().freeze().unwrap();
        assert_eq!(
            registry.get("video_compress").is_some(),
            cfg!(feature = "video")
        );
        assert_eq!(registry.category("视频").is_some(), cfg!(feature = "video"));
        assert_eq!(registry.tool("ffmpeg").is_some(), cfg!(feature = "video"));
        for id in ["vid", "mp4", "mkv", "webm", "mov", "avi"] {
            assert_eq!(
                registry.types().get(id).is_some(),
                cfg!(feature = "video"),
                "{id}"
            );
        }
        assert_eq!(
            registry.get("background_removal").is_some(),
            cfg!(feature = "onnx")
        );
        let read = &registry.get("read").unwrap().kind;
        assert_eq!(
            read.params.iter().any(|param| param.id == "videoPath"),
            cfg!(feature = "video")
        );
    }
}
