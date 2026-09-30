//! 节点类型的「元数据」描述。
//!
//! 每个工具（节点类型）都把自己长什么样交给这里描述：有哪些输入端口、哪些
//! 输出端口、哪些参数。前端不认识任何具体工具，它只渲染这份元数据 ——
//! 所以加一个新工具只需要在 `nodes/` 里加一个文件。

use serde::Serialize;

use super::params::Params;
use super::port_type::PortType;

/// 一个端口的声明。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortDef {
    pub id: String,
    pub label: String,
    pub ty: PortType,
    /// 必填端口在运行前必须连上线。
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl PortDef {
    pub fn new(id: &str, label: &str, ty: PortType) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            ty,
            required: false,
            hint: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.to_string());
        self
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl SelectOption {
    pub fn new(value: &str, label: &str) -> Self {
        Self {
            value: value.to_string(),
            label: label.to_string(),
            hint: None,
        }
    }

    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.to_string());
        self
    }
}

/// 参数控件。前端按 `control` 字段决定渲染成什么。
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "control",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ParamSpec {
    Number {
        default: f64,
        min: f64,
        max: f64,
        step: f64,
        integer: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    /// 和 `Number` 取值完全一样，只是用一个滑杆来拖 ——
    /// 像「允许的损耗率」这种 0—100 的连续量，拖着比输数字顺手。
    Slider {
        default: f64,
        min: f64,
        max: f64,
        step: f64,
        integer: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    Text {
        default: String,
        multiline: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    Select {
        default: String,
        options: Vec<SelectOption>,
    },
    Bool {
        default: bool,
    },
    /// 一个本地文件路径，配合「选择文件」按钮使用。
    File {
        default: String,
        dialog_title: String,
        extensions: Vec<String>,
        /// 为真时对话框选的是一个目录（「保存到目录」节点用）。
        #[serde(default, skip_serializing_if = "is_false")]
        directory: bool,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// 参数在什么条件下才显示。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleWhen {
    pub param: String,
    pub any_of: Vec<String>,
    /// 还要**同时**满足的条件。空表示只看上面那一条。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub all_of: Vec<VisibleWhen>,
}

impl VisibleWhen {
    fn new(param: &str, any_of: &[&str]) -> Self {
        Self {
            param: param.to_string(),
            any_of: any_of.iter().map(|value| value.to_string()).collect(),
            all_of: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamDef {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<VisibleWhen>,
    #[serde(flatten)]
    pub spec: ParamSpec,
}

impl ParamDef {
    pub fn new(id: &str, label: &str, spec: ParamSpec) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            description: None,
            visible_when: None,
            spec,
        }
    }

    pub fn described(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    pub fn visible_when(mut self, param: &str, any_of: &[&str]) -> Self {
        self.visible_when = Some(VisibleWhen::new(param, any_of));
        self
    }

    /// 在上一条之外再加一条必须同时满足的条件（「而且」）。
    pub fn and_visible_when(mut self, param: &str, any_of: &[&str]) -> Self {
        match &mut self.visible_when {
            Some(when) => when.all_of.push(VisibleWhen::new(param, any_of)),
            None => self.visible_when = Some(VisibleWhen::new(param, any_of)),
        }
        self
    }
}

/// 一个工具（节点类型）的完整说明。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeKind {
    /// 稳定标识，落盘的工作流里引用它。
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    /// 起点节点没有输入端口，工作流从它开始。
    pub is_source: bool,
    pub inputs: Vec<PortDef>,
    pub outputs: Vec<PortDef>,
    pub params: Vec<ParamDef>,
    /// 展开节点卡片时列在「注意事项」里的那几句话。
    pub notes: Vec<String>,
}

/// 输出端口的解析函数。
///
/// 绝大多数节点的端口类型是写死的，直接返回 [`NodeKind::outputs`]；输入节点
/// 是例外：它输出的图像格式取决于用户选了哪个文件，所以要现算。
pub type ResolveOutputsFn = fn(&Params) -> Vec<PortDef>;

impl NodeKind {
    /// 从参数声明里推出默认参数值。新建节点时直接用这份，
    /// 免得在前端再抄一遍默认值（抄歪了就很难查）。
    pub fn default_params(&self) -> Params {
        let mut params = Params::new();
        for def in &self.params {
            let value = match &def.spec {
                ParamSpec::Number { default, .. } | ParamSpec::Slider { default, .. } => {
                    serde_json::json!(default)
                }
                ParamSpec::Text { default, .. } => serde_json::json!(default),
                ParamSpec::Select { default, .. } => serde_json::json!(default),
                ParamSpec::Bool { default } => serde_json::json!(default),
                ParamSpec::File { default, .. } => serde_json::json!(default),
            };
            params.insert(def.id.clone(), value);
        }
        params
    }
}

/// 工具清单里的一项：节点元数据 + 默认参数。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeKindInfo {
    #[serde(flatten)]
    pub kind: NodeKind,
    pub defaults: Params,
}
