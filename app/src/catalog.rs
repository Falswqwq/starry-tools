//! 节点清单 —— 直接读 core 的注册表。
//!
//! 这里**不认识任何具体工具**：节点的名字、分类、描述、注意事项、端口类型、
//! 参数控件、默认值全部来自 `starrytools_core::registry()`。所以加一个新工具
//! 只需要在 `core/src/nodes/` 里加一个文件并在 `registry.rs` 登记一行，
//! 这个界面会自动长出对应的卡片、节点和参数控件。

use serde_json::Value;
use starrytools_core::bg_model;
use starrytools_core::model::node_kind::{ParamDef, ParamSpec, PortDef};
use starrytools_core::model::params::Params;
use starrytools_core::model::port_type::PortType;
use starrytools_core::registry::registry;

/// 一个端口在界面上需要知道的东西。
#[derive(Clone)]
pub struct Port {
    /// 端口 id，落盘时用（存档里的 `sourcePort` / `targetPort` 认的就是它）。
    pub id: String,
    pub label: String,
    /// 短标签，卡片上的 `IMG → PNG` 用它。
    pub badge: String,
    /// 真实类型。**连线合不合法就靠它**（`PortType::accepts`）；
    /// 徽标的颜色也只看它（见 [`crate::theme::badge_color`]）。
    pub ty: PortType,
    pub required: bool,
    /// 这是某个**参数**的可选输入时，记下参数的 id。
    /// 界面据此把它画在参数那一行（浅蓝的小圆点），而不是端口列里。
    pub param: Option<String>,
}

impl Port {
    /// 是不是某个参数的可选输入。
    pub fn is_param(&self) -> bool {
        self.param.is_some()
    }
}

/// 下拉框里的一个选项。
#[derive(Clone)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub hint: Option<String>,
}

/// 参数控件。
///
/// 照着 core 的 [`ParamSpec`] 镜像一份 —— 那边是落盘与传输的形状，
/// 这边是「画成什么控件」的形状。多这一层是为了让界面代码不直接依赖
/// 后端的序列化细节。
#[derive(Clone)]
pub enum Control {
    /// 拖数字。
    Number {
        min: f64,
        max: f64,
        #[allow(dead_code)]
        step: f64,
        integer: bool,
        unit: Option<String>,
    },
    /// 滑杆。取值和 `Number` 完全一样，只是拖起来更顺手。
    Slider {
        min: f64,
        max: f64,
        #[allow(dead_code)]
        step: f64,
        integer: bool,
        unit: Option<String>,
    },
    Text {
        multiline: bool,
        placeholder: Option<String>,
    },
    Select {
        options: Vec<Choice>,
    },
    /// 通用取色器：收起时是一条色条，点开选颜色。
    Color,
    Bool,
    File {
        dialog_title: String,
        extensions: Vec<String>,
        directory: bool,
    },
    /// 一大块输入区（拖文件 / 粘贴 / 点一下打字）。
    DropZone,
}

/// 参数在什么条件下才显示。
#[derive(Clone)]
pub struct VisibleWhen {
    pub param: String,
    pub any_of: Vec<String>,
    /// 还要**同时**满足的条件。
    pub all_of: Vec<VisibleWhen>,
}

impl VisibleWhen {
    /// 这一条成立吗。
    ///
    /// 参数值一律按字符串比 —— core 那边也是这么写的（开关就是 `"true"` / `"false"`）。
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

/// 把一个参数值取成字符串。
fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// 一个参数在界面上需要知道的东西。
#[derive(Clone)]
pub struct Param {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub control: Control,
    pub visible_when: Option<VisibleWhen>,
}

impl Param {
    /// 现在该不该显示这个参数。
    pub fn visible(&self, params: &Params) -> bool {
        self.visible_when
            .as_ref()
            .is_none_or(|when| when.matches(params))
    }

    /// 节点库卡片展开时那一行说明。那边不画控件，只描述一下。
    pub fn summary(&self) -> String {
        match &self.control {
            Control::Number {
                min,
                max,
                unit,
                integer,
                ..
            }
            | Control::Slider {
                min,
                max,
                unit,
                integer,
                ..
            } => {
                let kind = if *integer { "整数" } else { "数值" };
                format!("{kind} {min}–{max}{}", unit.as_deref().unwrap_or(""))
            }
            Control::Text { multiline, .. } => {
                if *multiline {
                    "多行文本".to_string()
                } else {
                    "文本".to_string()
                }
            }
            Control::Select { options } => format!(
                "下拉：{}",
                options
                    .iter()
                    .map(|choice| choice.label.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            ),
            Control::Color => "颜色".to_string(),
            Control::Bool => "开关".to_string(),
            Control::DropZone => "拖入文件 / 粘贴 / 打字".to_string(),
            Control::File {
                directory,
                extensions,
                ..
            } => {
                if *directory {
                    "选择目录".to_string()
                } else if extensions.is_empty() {
                    "选择文件".to_string()
                } else {
                    format!("选择文件（{}）", extensions.join(" / "))
                }
            }
        }
    }
}

/// 一个工具在界面上需要知道的东西。
#[derive(Clone)]
pub struct Kind {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    /// 展开卡片时列在「注意事项」里的那几句话。
    pub notes: Vec<String>,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub params: Vec<Param>,
    /// 用默认参数时会不会拦住运行（紫色节点）。节点库卡片据此标注。
    pub interactive: bool,
    /// 这个节点要一个**得下载的模型**才能跑时，那个「模型」参数的 id（见 [`Kind::model`]）。
    pub model_param: Option<String>,
    /// 新建节点时用的初始参数，免得在界面里再抄一遍默认值。
    pub defaults: Params,
}

impl Kind {
    /// 这个节点当前选中的模型。没有「模型」参数、或参数指向一个不认识的 id 时返回 `None`。
    pub fn model(&self, params: &Params) -> Option<&'static bg_model::Model> {
        let param = self.model_param.as_deref()?;
        let id = params
            .get(param)
            .and_then(Value::as_str)
            .unwrap_or_else(|| bg_model::default_model().id);
        bg_model::find(id)
    }

    /// 本地还没有这个节点要的模型吗 —— 界面据此把整个节点禁用、挂一个下载面板。
    /// 需要模型但参数指向一个不认识的 id 时也算「缺」——那样才拦得住运行。
    pub fn model_missing(&self, params: &Params) -> bool {
        self.model_param.is_some()
            && self
                .model(params)
                .is_none_or(|model| !bg_model::is_downloaded(model.id))
    }
}

/// 全部内置工具，顺序就是 `registry.rs` 里的登记顺序。
pub fn all() -> Vec<Kind> {
    registry()
        .kinds()
        .into_iter()
        .map(|info| {
            let kind = info.kind;
            Kind {
                id: kind.id.clone(),
                name: kind.name.clone(),
                category: kind.category.clone(),
                description: kind.description.clone(),
                notes: kind.notes.clone(),
                inputs: ports(&kind.inputs),
                outputs: ports(&kind.outputs),
                params: kind.params.iter().map(param).collect(),
                interactive: info.interactive,
                model_param: info.model_param,
                defaults: info.defaults,
            }
        })
        .collect()
}

/// 某个节点在**当前参数**下的输入输出端口。
///
/// 端口类型是可以跟着参数走的：「图像格式转换」选了 JPEG，输出就从 `Image(Png)`
/// 变成 `Image(Jpeg)`；「图像压缩」换了模式也一样；「输入」节点则要真读了文件头
/// 才知道。所以**不能在建节点的时候抄一份就完事** —— 参数一改就得重算。
///
/// 类型 id 不认得时返回空端口（老存档里的未知节点）。
pub fn ports_for(kind_id: &str, params: &Params) -> (Vec<Port>, Vec<Port>) {
    match registry().get(kind_id) {
        Some(spec) => (
            // 含**参数端口**（每个能被上游喂的参数各一个）。界面会把它们
            // 从端口列里挑出来，画到各自的参数行上。
            ports(&spec.kind.inputs_for(params)),
            ports(&spec.outputs_for(params)),
        ),
        None => (Vec::new(), Vec::new()),
    }
}

/// 某个节点实例在**当前参数**下会不会拦住运行（紫色节点）。
///
/// 和端口一样，这件事跟着参数走 —— 「图像裁切」切到形状模式才变紫。
pub fn is_interactive(kind_id: &str, params: &Params) -> bool {
    registry().is_interactive(kind_id, params)
}

/// 这个节点有没有「输入区」（`DropZone`）控件。有的话它自己就把值摆出来了 ——
/// 跑完之后不用再在卡片底下摆一条结果图。
pub fn has_drop_zone(kind_id: &str) -> bool {
    registry().get(kind_id).is_some_and(|spec| {
        spec.kind
            .params
            .iter()
            .any(|param| matches!(param.spec, ParamSpec::DropZone))
    })
}

/// 分类列表：按出现顺序排，前面加一个「全部」。不写死分类名 ——
/// core 里新加一个分类，这里自动就有了。
pub fn categories(kinds: &[Kind]) -> Vec<String> {
    let mut out = vec!["全部".to_string()];
    for kind in kinds {
        if !out.contains(&kind.category) {
            out.push(kind.category.clone());
        }
    }
    out
}

fn ports(defs: &[PortDef]) -> Vec<Port> {
    defs.iter()
        .map(|def| Port {
            id: def.id.clone(),
            label: def.label.clone(),
            badge: def.ty.badge().to_string(),
            ty: def.ty,
            required: def.required,
            param: def.param.clone(),
        })
        .collect()
}

fn param(def: &ParamDef) -> Param {
    let control = match &def.spec {
        ParamSpec::Number {
            min,
            max,
            step,
            integer,
            unit,
            ..
        } => Control::Number {
            min: *min,
            max: *max,
            step: *step,
            integer: *integer,
            unit: unit.clone(),
        },
        ParamSpec::Slider {
            min,
            max,
            step,
            integer,
            unit,
            ..
        } => Control::Slider {
            min: *min,
            max: *max,
            step: *step,
            integer: *integer,
            unit: unit.clone(),
        },
        ParamSpec::Text {
            multiline,
            placeholder,
            ..
        } => Control::Text {
            multiline: *multiline,
            placeholder: placeholder.clone(),
        },
        ParamSpec::Select { options, .. } => Control::Select {
            options: options.iter().map(choice).collect(),
        },
        ParamSpec::Color { .. } => Control::Color,
        ParamSpec::Bool { .. } => Control::Bool,
        ParamSpec::DropZone => Control::DropZone,
        ParamSpec::File {
            dialog_title,
            extensions,
            directory,
            ..
        } => Control::File {
            dialog_title: dialog_title.clone(),
            extensions: extensions.clone(),
            directory: *directory,
        },
    };

    Param {
        id: def.id.clone(),
        label: def.label.clone(),
        description: def.description.clone(),
        control,
        visible_when: def.visible_when.as_ref().map(visible_when),
    }
}

fn choice(option: &starrytools_core::model::node_kind::SelectOption) -> Choice {
    Choice {
        value: option.value.clone(),
        label: option.label.clone(),
        hint: option.hint.clone(),
    }
}

fn visible_when(when: &starrytools_core::model::node_kind::VisibleWhen) -> VisibleWhen {
    VisibleWhen {
        param: when.param.clone(),
        any_of: when.any_of.clone(),
        all_of: when.all_of.iter().map(visible_when).collect(),
    }
}
