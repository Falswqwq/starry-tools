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
/// 按参数现算**输入**端口。给「运算」这类输入类型随模式变的节点用。
pub type ResolveInputsFn = Arc<dyn Fn(&Params) -> Vec<PortDef> + Send + Sync>;

/// 节点本地变量的端口协议。变量状态随一次运行存在，不进工作流存档。
///
/// Get 在消费时读取当前值；Set 是写入端口，可接多条线，按上游实际
/// 产生值的顺序以后写到的为准。Set 不参与 Get 的求值依赖，因此不会成环。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablePorts {
    pub value_param: String,
    pub get: String,
    pub set: String,
}

impl VariablePorts {
    pub fn new(value_param: &str, get: &str, set: &str) -> Self {
        Self {
            value_param: value_param.into(),
            get: get.into(),
            set: set.into(),
        }
    }
}

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
    },
    Repeat {
        count_param: String,
        index_output: String,
        body_output: String,
        finally_output: String,
    },
    Loop {
        list_input: String,
        item_output: String,
        body_output: String,
        finally_output: String,
        max_param: String,
    },
}

pub struct NodeSpec {
    /// Opt-in: callbacks must have no observable side effects.
    pub pure: bool,
    /// 节点本地变量的 Get/Set 端口协议；普通节点为 `None`。
    pub variable: Option<VariablePorts>,
    pub kind: NodeKind,
    pub resolve_outputs: Option<ResolveOutputsFn>,
    /// 输入端口的现算函数。`None` 表示认 `kind.inputs` 里写的声明。
    ///
    /// 返回的是**声明输入**；参数端口与空输入回退仍由 `NodeKind::inputs_with` 补上。
    pub resolve_inputs: Option<ResolveInputsFn>,
    pub interactive: Option<InteractiveFn>,
    /// 这个节点属于哪类**专色**（颜色册里的 id，见 [`RegistryBuilder::register_accent`]）。
    ///
    /// `None` 表示没有专属外观，走正常主题边线。拦住运行的紫色不在这里声明 ——
    /// 那是随参数变的，由 [`NodeSpec::interactive`] 推导。
    pub accent: Option<String>,
    pub model_param: Option<String>,
    pub requires_tool: Option<String>,
    pub control_flow: Option<ControlFlow>,
    /// 这是不是新建工作流时默认放的**入口节点**（目前只有「起点」）。
    pub entry: bool,
    /// 输出是否自动写进工作流的产物目录。
    ///
    /// 默认 `true`；「读取」这类只是把外部文件读进来的节点关掉它，免得复制一份。
    pub auto_artifact: bool,
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
            resolve_inputs: None,
            interactive: None,
            accent: None,
            model_param: None,
            requires_tool: None,
            control_flow: None,
            pure: false,
            variable: None,
            entry: false,
            auto_artifact: true,
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

    /// 让输入端口也随参数现算（如「运算」在数值 / 文本之间切换）。
    ///
    /// 函数返回**声明输入**；参数端口与空输入回退会自动补上。
    pub fn resolve_inputs(
        mut self,
        resolve_inputs: impl Fn(&Params) -> Vec<PortDef> + Send + Sync + 'static,
    ) -> Self {
        self.resolve_inputs = Some(Arc::new(resolve_inputs));
        self
    }

    /// 这个节点在给定参数下的输入端口（含推导出的参数端口与空输入回退）。
    ///
    /// 入口节点（「起点」）不接任何输入，所以不补空输入 —— 它只发出开始信号。
    pub fn inputs_for(&self, params: &Params) -> Vec<PortDef> {
        let declared = match &self.resolve_inputs {
            Some(resolve) => resolve(params),
            None => self.kind.inputs.clone(),
        };
        let mut ports = self.kind.inputs_with_options(declared, params, false);
        if !self.pure && !self.entry && !ports.iter().any(|p| p.exec) {
            ports.insert(0, PortDef::exec("exec", "执行"));
        }
        ports
    }

    pub fn pure(mut self) -> Self {
        self.pure = true;
        self.auto_artifact = false;
        self
    }

    /// 声明这个节点是本地变量：`value_param` 是初始值参数，`get` / `set` 是端口 id。
    pub fn variable(mut self, ports: VariablePorts) -> Self {
        self.variable = Some(ports);
        self
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

    /// 声明这个节点属于哪类专色（颜色册里的 id）。
    pub fn accent(mut self, id: impl Into<String>) -> Self {
        self.accent = Some(id.into());
        self
    }

    pub fn control_flow(mut self, control_flow: ControlFlow) -> Self {
        self.control_flow = Some(control_flow);
        self
    }

    /// 标记为新建工作流时默认放置的入口节点。
    pub fn entry(mut self) -> Self {
        self.entry = true;
        self
    }

    /// 这个节点的输出不作为自动产物落盘（「读取」读进来的就是外部文件本身）。
    pub fn without_auto_artifact(mut self) -> Self {
        self.auto_artifact = false;
        self
    }

    pub fn is_interactive(&self, params: &Params) -> bool {
        self.interactive.as_ref().is_some_and(|is| is(params))
    }

    pub fn outputs_for(&self, params: &Params) -> Vec<PortDef> {
        let mut ports = match &self.resolve_outputs {
            Some(resolve) => resolve(params),
            None => self.kind.outputs.clone(),
        };
        if !self.pure && !ports.iter().any(|p| p.exec) {
            ports.push(PortDef::exec("then", "继续"));
        }
        ports
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
    pub id: String,
    pub label: String,
}

impl CategoryInfo {
    /// 分类只负责把节点**分组**，不带颜色 —— 颜色属于节点类专色。
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// 一类节点的**专色**。
///
/// 专色说的是「这类节点长这样」（逻辑节点暗蓝、会拦住运行的节点紫色），
/// 与节点库分类**无关** —— 分类只把节点分组，不给节点上色。
/// 一个节点属于哪一类，由它自己在 [`NodeSpec::accent`] 里声明颜色册里的 id。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeAccent {
    pub id: String,
    pub color: TypeColor,
}

impl NodeAccent {
    pub fn new(id: impl Into<String>, color: TypeColor) -> Self {
        Self {
            id: id.into(),
            color,
        }
    }
}

/// 保留的专色 id：**当前会拦住运行**的节点（见 [`NodeSpec::interactive`]）。
///
/// 它不是静态声明 —— 同一个节点（如「图像裁切」）换个参数才变成紫色，
/// 所以由 `interactive` 回调推导，界面按这个 id 取色。
pub const INTERACTIVE_ACCENT: &str = "interactive";

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
    #[error("invalid conversion: {from} -> {to}")]
    InvalidConversion { from: String, to: String },
    #[error("the global registry is already initialized")]
    AlreadyInitialized,
}

/// 只有构建器能登记；`freeze` 消耗它，得到没有写入 API 的注册表。
#[derive(Default)]
pub struct RegistryBuilder {
    types: types::RegistryBuilder,
    specs: Vec<NodeSpec>,
    categories: Vec<CategoryInfo>,
    accents: BTreeMap<String, NodeAccent>,
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
        // 框架级专色：拦住运行的节点是紫色的。它随参数变，不能由节点静态声明。
        builder.register_accent(INTERACTIVE_ACCENT, TypeColor::rgb(0x7c, 0x3a, 0xed))?;
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

    /// 注册一类节点的**专色**。节点用 [`NodeSpec::accent`] 声明自己属于哪一类。
    ///
    /// 专色与分类无关：分类只把节点分组，颜色说的是「这类节点长这样」。
    pub fn register_accent(
        &mut self,
        id: impl Into<String>,
        color: TypeColor,
    ) -> Result<(), RegistrationError> {
        let id = id.into();
        if self.accents.contains_key(&id) {
            return Err(RegistrationError::Duplicate {
                kind: "node accent",
                id,
            });
        }
        self.accents.insert(id.clone(), NodeAccent { id, color });
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
        for spec in &self.specs {
            if let Some(accent) = &spec.accent {
                if !self.accents.contains_key(accent) {
                    return Err(RegistrationError::Unknown {
                        kind: "node accent",
                        id: accent.clone(),
                    });
                }
            }
        }
        Ok(Registry {
            types: self.types.freeze()?,
            specs: self.specs,
            categories: self.categories,
            accents: self.accents,
            tools: self.tools,
            aliases: self.aliases,
        })
    }
}

pub struct Registry {
    types: types::Registry,
    specs: Vec<NodeSpec>,
    categories: Vec<CategoryInfo>,
    accents: BTreeMap<String, NodeAccent>,
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
    /// 某类节点的专色，界面按它给节点边线上色。
    pub fn accent(&self, id: &str) -> Option<&NodeAccent> {
        self.accents.get(id)
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
                    kind: NodeKind {
                        inputs: spec.inputs_for(&defaults),
                        outputs: spec.outputs_for(&defaults),
                        ..spec.kind.clone()
                    },
                    pure: spec.pure,
                    variable: spec.variable.clone(),
                    interactive: spec.is_interactive(&defaults),
                    entry: spec.entry,
                    accent: spec.accent.clone(),
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
            description: String::from("拥有扩展节点"),
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
            .register_category(CategoryInfo::new(category.clone(), "扩展"))
            .unwrap();
        // 专色颜色册：扩展可以自报一类节点色，节点用 id 声明归属。
        builder
            .register_accent(String::from("extension-accent"), TypeColor::rgb(3, 5, 7))
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
        .accent(String::from("extension-accent"))
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
            registry.accent("extension-accent").unwrap().color,
            TypeColor::rgb(3, 5, 7)
        );
        assert_eq!(
            registry
                .kinds()
                .iter()
                .find(|info| info.kind.id == "extension/node")
                .and_then(|info| info.accent.clone()),
            Some(String::from("extension-accent"))
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
            .register_category(CategoryInfo::new("extension", "扩展"))
            .unwrap();
        assert!(matches!(
            builder.register_category(CategoryInfo::new("extension", "替换")),
            Err(RegistrationError::Duplicate {
                kind: "category",
                ..
            })
        ));
        builder
            .register_accent("accent", TypeColor::rgb(1, 2, 3))
            .unwrap();
        assert!(matches!(
            builder.register_accent("accent", TypeColor::rgb(4, 5, 6)),
            Err(RegistrationError::Duplicate {
                kind: "node accent",
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
        // 节点声明了颜色册里没有的专色 id：冻结时拒绝，避免执行期找不到颜色。
        let mut builder = RegistryBuilder::new();
        builder
            .register_category(CategoryInfo::new("extension", "扩展"))
            .unwrap();
        builder
            .register_node(NodeSpec::fixed(kind("node"), |_| Ok(ValueMap::new())).accent("missing"))
            .unwrap();
        assert!(matches!(
            builder.freeze(),
            Err(RegistrationError::Unknown {
                kind: "node accent",
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
        // 专色颜色册：框架的「拦住运行」紫 + 内置逻辑类暗蓝。
        assert_eq!(
            registry.accent(INTERACTIVE_ACCENT).unwrap().color,
            TypeColor::rgb(0x7c, 0x3a, 0xed)
        );
        assert_eq!(
            registry.accent(crate::nodes::LOGIC_ACCENT).unwrap().color,
            TypeColor::rgb(0x1e, 0x40, 0xaf)
        );
        assert_eq!(
            registry.accent(crate::nodes::DATA_ACCENT).unwrap().color,
            TypeColor::rgb(0x16, 0x65, 0x34)
        );
        // 声明归属：只有逻辑节点带专色，其余节点走正常主题边线。
        for id in ["sequence", "iterate", "repeat"] {
            assert_eq!(
                registry.get(id).unwrap().accent.as_deref(),
                Some(crate::nodes::LOGIC_ACCENT),
                "{id} 应该属于逻辑类专色"
            );
        }
        for id in ["convert_image", "read", "save_output"] {
            assert_eq!(registry.get(id).unwrap().accent, None, "{id} 不该带专色");
        }
        for spec in registry.all() {
            assert!(registry.category(&spec.kind.category).is_some());
        }
        assert_eq!(
            registry.get("sequence").unwrap().control_flow,
            Some(ControlFlow::Sequence {
                output_prefix: "out".into(),
            })
        );
        assert_eq!(
            registry.get("repeat").unwrap().control_flow,
            Some(ControlFlow::Repeat {
                count_param: "times".into(),
                index_output: "index".into(),
                body_output: "body".into(),
                finally_output: "finally".into(),
            })
        );
        assert_eq!(
            registry.get("iterate").unwrap().control_flow,
            Some(ControlFlow::Loop {
                list_input: "list".into(),
                item_output: "item".into(),
                body_output: "body".into(),
                finally_output: "finally".into(),
                max_param: "max".into(),
            })
        );
        assert!(!registry.tool_available("not-a-registered-tool"));
    }

    #[test]
    fn input_ports_obey_the_column_then_parameter_order() {
        // 契约：端口列输入（声明输入 + 自动空输入）必须排在参数端口之前。
        // 画布按「第几个非参数端口」定位圆点与徽标（`canvas/placement.rs::in_port`）。
        let registry = RegistryBuilder::with_builtins().unwrap().freeze().unwrap();
        for spec in registry.all() {
            let params = spec.kind.default_params();
            let ports = spec.inputs_for(&params);

            let last_column = ports
                .iter()
                .rposition(|port| !port.is_param() && port.panel_input_row.is_none());
            let first_param = ports.iter().position(PortDef::is_param);
            if let (Some(last), Some(first)) = (last_column, first_param) {
                assert!(
                    last < first,
                    "{} 的端口列输入必须排在参数端口之前：{ports:?}",
                    spec.kind.id
                );
            }

            // 参数端口的类型必须来自参数声明，不能由节点另写一份。
            for port in ports.iter().filter(|port| port.is_param()) {
                let id = port.param.as_deref().expect("参数端口必须带参数 id");
                let def = spec
                    .kind
                    .params
                    .iter()
                    .find(|def| def.id == id)
                    .unwrap_or_else(|| panic!("{}：未知参数 {id}", spec.kind.id));
                assert_eq!(
                    port.ty,
                    def.spec.input_type().expect("参数端口意味着可喂值"),
                    "{}：{id} 的端口类型必须来自参数声明",
                    spec.kind.id
                );
            }

            // 入口节点不接输入；其余没有声明输入、但有输出的节点恰好一个空输入。
            if spec.entry {
                assert!(ports.is_empty(), "{} 是入口节点，不该有输入", spec.kind.id);
            } else if !spec.pure {
                let voids = ports.iter().filter(|port| port.exec).count();
                assert_eq!(
                    voids, 1,
                    "{} 该恰好有一个自动空输入：{ports:?}",
                    spec.kind.id
                );
            }
        }
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
