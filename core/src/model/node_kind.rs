//! 节点类型的「元数据」描述。
//!
//! 每个工具（节点类型）都把自己长什么样交给这里描述：有哪些输入端口、哪些
//! 输出端口、哪些参数。前端不认识任何具体工具，它只渲染这份元数据 ——
//! 所以加一个新工具只需要在 `nodes/` 里加一个文件。

use serde::Serialize;
use std::sync::Arc;

use super::params::Params;
use super::port_type::PortType;

/// 一个端口的声明。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortDef {
    /// Exec is a control channel, never a runtime Value.
    #[serde(default)]
    pub exec: bool,
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
    /// 这个**输出**端口是不是画在参数栏的某一行上（而不是顶部端口列）。
    ///
    /// 值是它在参数栏里的**行号**（0 起）。「顺序执行」「遍历」的输出就是挂在
    /// 那个「动态输出」参数的每一行上 —— 于是节点顶部可以一个输出也没有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_row: Option<usize>,
    /// 这个**输入**端口是不是画在参数区**下方**的独立一行上（而不是顶部端口列）。
    ///
    /// 值是行号（0 起）。「数据」节点的「设定值」用它 —— 它不影响上面那个值输入框。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_input_row: Option<usize>,
    /// 同一个端口是否允许接多条线。变量节点的 Set 口用它 ——
    /// 多个上游都能写入，按连线顺序以后写到的为准。
    #[serde(default, skip_serializing_if = "is_false")]
    pub multi: bool,
}

impl PortDef {
    pub fn new(id: &str, label: &str, ty: PortType) -> Self {
        Self {
            exec: false,
            id: id.to_string(),
            label: label.to_string(),
            ty,
            required: false,
            hint: None,
            param: None,
            panel_row: None,
            panel_input_row: None,
            multi: false,
        }
    }

    pub fn exec(id: &str, label: &str) -> Self {
        Self {
            exec: true,
            ..Self::new(id, label, PortType::Void)
        }
    }

    /// 某个参数的可选输入端口。端口 id 带前缀，和节点声明的输入不会撞名。
    pub fn for_param(param_id: &str, label: &str, ty: PortType) -> Self {
        Self {
            exec: false,
            id: param_port_id(param_id),
            label: label.to_string(),
            ty,
            required: false,
            hint: None,
            param: Some(param_id.to_string()),
            panel_row: None,
            panel_input_row: None,
            multi: false,
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

    /// 允许同一个输入端口接多条线。
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// 把这个输出端口画在参数栏的第 `row` 行（0 起）上，而不是顶部端口列。
    pub fn on_panel_row(mut self, row: usize) -> Self {
        self.panel_row = Some(row);
        self
    }

    /// 把这个**输入**端口画在参数区下方的第 `row` 行（0 起），不禁用任何控件。
    pub fn below_params(mut self, row: usize) -> Self {
        self.panel_input_row = Some(row);
        self
    }

    /// 短徽标直接取自类型登记表，不在 UI 镜像一份字段。
    pub fn badge(&self) -> &'static str {
        self.ty.badge()
    }

    /// 这是不是某个参数的可选输入（而不是节点自己的输入端口）。
    pub fn is_param(&self) -> bool {
        self.param.is_some()
    }
}

/// 参数端口的 id 前缀。`param:foo` 认的就是参数 `foo`。
pub const PARAM_PORT_PREFIX: &str = "param:";

/// 不需要任何输入的节点自动获得的那个「空输入」端口 id。
pub const VOID_INPUT_PORT: &str = "seq";

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
    /// 一排**动态端口**的参数：存的是**个数**，控件画成一列端口 + 每行一个删除按钮
    /// + 底部一个加号。节点的 `outputs_for` 读它，把 `{prefix}{序号}` 一个个列出来。
    ///
    /// 节点**不再手写**这一排输出，而是用这个参数声明 —— 于是加/删输出的交互只写一次，
    /// 以后任何「输出个数可变」的节点都能用（「顺序执行」当前就在用它）。
    Ports {
        default: f64,
        /// 最少几个、且前几个**不能删**（比如「顺序执行」必须留一个）。
        min: f64,
        /// 最多几个 —— 手滑点多了不把版面撑爆。
        max: f64,
        /// 行首注释文本的参数前缀；为空时不显示可编辑名称。
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label_prefix: String,
        /// 新增行没有保存过名称时显示的默认前缀，后面会补行号。
        #[serde(default, skip_serializing_if = "String::is_empty")]
        default_label: String,
    },
}

/// 动态端口行没有保存过名称时的默认注释，例如「分支 1」。
pub fn ports_row_default_label(default_label: &str, index: usize) -> String {
    if default_label.is_empty() {
        index.to_string()
    } else {
        format!("{default_label} {index}")
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_true(value: &bool) -> bool {
    *value
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
            // 动态输出个数是节点自己管的，不从上游喂。
            ParamSpec::Ports { .. } => None,
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
    /// 这个参数能不能被上游喂值（默认能）。
    ///
    /// 「数据」节点的「值」关掉它：值框始终可编辑，来自上游的写入走单独的
    /// 「设定值」端口，而不是把控件禁用。
    #[serde(skip_serializing_if = "is_true")]
    pub upstream: bool,
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
            upstream: true,
            spec,
        }
    }

    /// 这个参数不接受上游喂值：它只由用户在本节点上编辑。
    pub fn local_only(mut self) -> Self {
        self.upstream = false;
        self
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
pub type ResolveOutputsFn = Arc<dyn Fn(&Params) -> Vec<PortDef> + Send + Sync>;

impl NodeKind {
    /// 这个节点在给定参数下的**全部**输入端口。
    ///
    /// = 声明里写的输入 + 每个「能被上游喂」的可见参数各一个可选输入。
    ///
    /// 参数端口是**推导**出来的，不是逐个节点写的：节点只要正常声明参数，
    /// 它就自动有了（见 [`ParamSpec::input_type`]）。界面把参数端口画在参数那一行，
    /// 引擎把接上来的值覆盖到那个参数上 —— 两边都不认识具体工具。
    pub fn inputs_for(&self, params: &Params) -> Vec<PortDef> {
        self.inputs_with(self.inputs.clone(), params)
    }

    /// 以 `ports` 作为**声明输入**，补上推导出的参数端口与「空输入」回退。
    ///
    /// 动态输入（`NodeSpec::resolve_inputs`）复用这一套推导，不另起一份规则。
    ///
    /// **顺序契约**：端口列里的输入（声明输入 + 自动空输入）必须排在参数端口之前 ——
    /// 画布按「第几个端口列输入」定位圆点与徽标（见 `canvas/placement.rs::in_port`）。
    pub fn inputs_with(&self, ports: Vec<PortDef>, params: &Params) -> Vec<PortDef> {
        self.inputs_with_options(ports, params, true)
    }

    /// 同 [`NodeKind::inputs_with`]，但可以关掉「空输入」回退。
    ///
    /// 「起点」用它：它没有任何输入，只负责发出一个开始信号。
    pub fn inputs_with_options(
        &self,
        mut ports: Vec<PortDef>,
        params: &Params,
        add_void_input: bool,
    ) -> Vec<PortDef> {
        let _ = add_void_input;
        for def in &self.params {
            if !def.upstream {
                continue;
            }
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
                ParamSpec::Ports { default, .. } => serde_json::json!(default),
            };
            params.insert(def.id.clone(), value);

            if let ParamSpec::Ports {
                default,
                label_prefix,
                default_label,
                ..
            } = &def.spec
            {
                if !label_prefix.is_empty() {
                    for index in 1..=(default.round().max(0.0) as usize) {
                        params.insert(
                            format!("{label_prefix}{index}"),
                            serde_json::json!(ports_row_default_label(default_label, index)),
                        );
                    }
                }
            }
        }
        params
    }
}

/// 工具清单里的一项：节点元数据 + 默认参数。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeKindInfo {
    pub pure: bool,
    /// 变量节点声明；app 据此把运行后的状态写回对应参数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<crate::registry::VariablePorts>,
    #[serde(flatten)]
    pub kind: NodeKind,
    /// 用默认参数时会不会拦住运行（紫色节点）。节点库卡片据此标注。
    pub interactive: bool,
    /// 这是不是新建工作流时默认放的**入口节点**（目前只有「起点」）。
    #[serde(default)]
    pub entry: bool,
    /// 这个节点声明的**节点类专色** id（`None` = 走正常主题边线）。
    ///
    /// 颜色本身在注册表的颜色册里（见 `RegistryBuilder::register_accent`）；
    /// 「拦住运行」的紫色不在这里，它随参数变，由 `interactive` 推导。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// 这个节点要一个**得下载的模型**才能跑时，给出那个「模型」参数的 id。
    ///
    /// 界面据此：模型不在本地就把整个节点禁用，并在卡片上挂一个下载按钮。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_param: Option<String>,
    /// 这个节点要一个**外部程序**（如 `ffmpeg`）才能跑时，给出它的名字。
    ///
    /// 界面据此：程序不在本地就把整个节点禁用，并在卡片上挂一句提示。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_flow: Option<crate::registry::ControlFlow>,
    pub defaults: Params,
}
