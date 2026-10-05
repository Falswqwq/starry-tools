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
    /// 这是某个**参数**的可选输入时，记下参数的 id。
    ///
    /// 界面据此把它画在参数那一行（浅蓝的小圆点），而不是端口列里；
    /// 引擎据此把接上来的值覆盖到那个参数上。见 [`NodeKind::inputs_for`]。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
}

impl PortDef {
    pub fn new(id: &str, label: &str, ty: PortType) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            ty,
            required: false,
            hint: None,
            param: None,
        }
    }

    /// 某个参数的可选输入端口。端口 id 带前缀，和节点声明的输入不会撞名。
    pub fn for_param(param_id: &str, label: &str, ty: PortType) -> Self {
        Self {
            id: param_port_id(param_id),
            label: label.to_string(),
            ty,
            required: false,
            hint: None,
            param: Some(param_id.to_string()),
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

    /// 这是不是某个参数的可选输入（而不是节点自己的输入端口）。
    pub fn is_param(&self) -> bool {
        self.param.is_some()
    }
}

/// 参数端口的 id 前缀。`param:foo` 认的就是参数 `foo`。
pub const PARAM_PORT_PREFIX: &str = "param:";

/// 某个参数的可选输入端口的 id。
pub fn param_port_id(param_id: &str) -> String {
    format!("{PARAM_PORT_PREFIX}{param_id}")
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
    /// 颜色：一个**通用取色器**（前端画成一条色条，点开是取色区 + 色相条 + R/G/B + Hex）。
    ///
    /// 值是一个字符串 —— `#rrggbb` / `#rrggbbaa`；`transparent` 表示全透明（向后兼容）。
    /// 以后任何节点想要个颜色，声明一个 `Color` 参数就行。
    Color {
        default: String,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl ParamSpec {
    /// 这个参数能被哪种**基本类型**的值从上游喂进来。
    ///
    /// 只有「值就是单个标量」的控件才给：数字 / 文本 / 开关 / 路径（路径也是字符串）。
    /// 下拉框**不给** —— 它的合法值是一张固定的表，随便接一段文字进来只会静默地出错。
    ///
    /// 这件事直接决定了节点会不会多出一个可选输入端口，所以写成推导而不是
    /// 逐个节点去声明 —— 加一个新工具时不用再想着它。
    pub fn input_type(&self) -> Option<PortType> {
        match self {
            ParamSpec::Number { .. } | ParamSpec::Slider { .. } => Some(PortType::Number),
            ParamSpec::Text { .. } => Some(PortType::Text),
            ParamSpec::Bool { .. } => Some(PortType::Bool),
            ParamSpec::File { .. } => Some(PortType::Text),
            // 颜色可以接一段**色板文本**（hex，每行一个）过来 —— 接上就取第一种颜色。
            ParamSpec::Color { .. } => Some(PortType::Text),
            // 下拉框的合法值是一张固定的表，随便接一段文字进来只会静默出错。
            ParamSpec::Select { .. } => None,
        }
    }
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

    /// 这一条成立吗。
    ///
    /// 参数值一律按字符串比 —— 界面那边也是这么做的（开关就是 `"true"` / `"false"`）。
    pub fn matches(&self, params: &Params) -> bool {
        let one = |when: &VisibleWhen| {
            params
                .get(&when.param)
                .map(as_text)
                .is_some_and(|value| when.any_of.contains(&value))
        };
        one(self) && self.all_of.iter().all(one)
    }
}

fn as_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
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

    /// 现在该不该显示这个参数。
    pub fn visible(&self, params: &Params) -> bool {
        self.visible_when
            .as_ref()
            .is_none_or(|when| when.matches(params))
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
/// 绝大多数节点的端口类型是写死的，直接返回 [`NodeKind::outputs`]；像「输入」
/// （按所选文件的格式）、「图像格式转换」（按目标格式）这种才要现算。
pub type ResolveOutputsFn = fn(&Params) -> Vec<PortDef>;

impl NodeKind {
    /// 这个节点在给定参数下的**全部**输入端口。
    ///
    /// = 声明里写的输入 + 每个「能被上游喂」的可见参数各一个可选输入。
    ///
    /// 参数端口是**推导**出来的，不是逐个节点写的：节点只要正常声明参数，
    /// 它就自动有了（见 [`ParamSpec::input_type`]）。界面把参数端口画在参数那一行，
    /// 引擎把接上来的值覆盖到那个参数上 —— 两边都不认识具体工具。
    ///
    /// 起点节点不生成：它的参数就是内容本身，没有上游可接。
    pub fn inputs_for(&self, params: &Params) -> Vec<PortDef> {
        let mut ports = self.inputs.clone();
        if self.is_source {
            return ports;
        }
        for def in &self.params {
            let Some(ty) = def.spec.input_type() else {
                continue;
            };
            if def.visible(params) {
                ports.push(PortDef::for_param(&def.id, &def.label, ty));
            }
        }
        ports
    }

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
                ParamSpec::Color { default, .. } => serde_json::json!(default),
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
    /// 用默认参数时会不会拦住运行（紫色节点）。节点库卡片据此标注。
    pub interactive: bool,
    /// 这个节点要一个**得下载的模型**才能跑时，给出那个「模型」参数的 id。
    ///
    /// 界面据此：模型不在本地就把整个节点禁用，并在卡片上挂一个下载按钮。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_param: Option<String>,
    pub defaults: Params,
}
