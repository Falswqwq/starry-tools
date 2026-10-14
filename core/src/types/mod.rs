//! 类型系统 —— 注册式，与应用本体解耦。
//!
//! 这里只有两个概念：
//!
//! * **接口（trait）** —— 一族类型的共同身份，像 `IMG`（图像）、`VID`（视频）、
//!   `FILE`（能写盘的产物）、`ANY`（通配）。接口**本身不是一个类型**，它就是
//!   「哪些类型算一类」这件事的名字。接口之间也没有继承 —— `IMG` 想说自己也能当
//!   `FILE` 用，只是把自己的 `traits` 里写上 `"file"` 而已。
//! * **具体类型** —— `PNG` / `JPEG` / `TEXT` / `MP4` …… 每个具体类型声明它实现哪些接口。
//!   具体类型之间没有任何派生关系，全是并列的。
//!
//! 端口的类型（[`PortType`]）要么指向一个**具体类型**（只收它自己），要么指向一个
//! **接口**（收下所有实现它的类型）。
//!
//! 内置类型由本模块登记；其它 feature（视频、音频……）在启动时用
//! [`Registry::register_trait`] / [`Registry::register_type`] 补上自己的接口与类型，
//! 界面侧完全不用改 —— 它只认登记表。
//!
//! 序列化时，一个端口类型就是一个字符串 id（`"png"` / `"img"` / `"video/mp4"` 之类）。

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::ser::{Serialize, Serializer};

use crate::model::port_type::ImageFormat;

/// 徽标的语义色。
///
/// **只回答「这是什么类型」一个问题** —— 与节点的状态色（选中 / 运行 / 报错）无关。
/// 界面把它映射成具体颜色，因此同一类型在任何地方都同色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeColor {
    /// 通配、以及「说不清是什么」的类型。
    Unknown,
    Text,
    Number,
    Bool,
    Image,
    Video,
}

/// 一条类型登记记录 —— 接口和具体类型共用这一份结构。
pub struct TypeInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub badge: &'static str,
    pub color: TypeColor,
    /// 具体类型：它实现的接口；接口：它能当作哪些上级接口用。
    pub traits: &'static [&'static str],
    /// 是接口（trait）还是具体类型。
    pub is_trait: bool,
    /// 写盘用的主扩展名（只有具体类型有）。
    pub extension: Option<&'static str>,
    /// 这个类型的**列表形式** `List[它]` 的徽标与名字 —— 登记时一次性算好、泄漏成
    /// `&'static str`（字符串就那几个）。于是 `PortType::badge()` 对列表也能零分配返回。
    pub list_badge: &'static str,
    pub list_label: &'static str,
}

/// 拼出某个类型的列表徽标 / 名字。只在登记时调几次，泄漏可忽略。
fn list_forms(label: &str, badge: &str) -> (&'static str, &'static str) {
    let list_badge = Box::leak(format!("[{badge}]").into_boxed_str());
    let list_label = Box::leak(format!("列表 · {label}").into_boxed_str());
    (list_badge, list_label)
}

/// 全部已登记的类型。
#[derive(Default)]
pub struct Registry {
    entries: HashMap<&'static str, TypeInfo>,
}

impl Registry {
    /// 登记一个接口。
    pub fn register_trait(
        &mut self,
        id: &'static str,
        label: &'static str,
        badge: &'static str,
        color: TypeColor,
        supertraits: &'static [&'static str],
    ) {
        let (list_badge, list_label) = list_forms(label, badge);
        self.entries.insert(
            id,
            TypeInfo {
                id,
                label,
                badge,
                color,
                traits: supertraits,
                is_trait: true,
                extension: None,
                list_badge,
                list_label,
            },
        );
    }

    /// 登记一个具体类型。
    pub fn register_type(
        &mut self,
        id: &'static str,
        label: &'static str,
        badge: &'static str,
        color: TypeColor,
        traits: &'static [&'static str],
        extension: Option<&'static str>,
    ) {
        let (list_badge, list_label) = list_forms(label, badge);
        self.entries.insert(
            id,
            TypeInfo {
                id,
                label,
                badge,
                color,
                traits,
                is_trait: false,
                extension,
                list_badge,
                list_label,
            },
        );
    }

    pub fn get(&self, id: &str) -> Option<&TypeInfo> {
        self.entries.get(id)
    }
}

/// 全局类型表。第一次用时建好：内置类型 + 各 feature 补上的类型。
pub fn types() -> &'static Registry {
    static REGISTRY: LazyLock<Registry> = LazyLock::new(build);
    &REGISTRY
}

fn build() -> Registry {
    let mut registry = Registry::default();
    builtin(&mut registry);
    #[cfg(feature = "video")]
    crate::features::video::register(&mut registry);
    registry
}

/// 内置的接口：通配、文件、图像。
fn builtin(registry: &mut Registry) {
    registry.register_trait("any", "任意值", "ANY", TypeColor::Unknown, &[]);
    // 能写盘的东西：图像、视频、以后可能还有音频，都实现它。
    registry.register_trait("file", "文件", "FILE", TypeColor::Unknown, &[]);
    registry.register_trait("img", "图像", "IMG", TypeColor::Image, &["file"]);

    registry.register_type("text", "文本", "TXT", TypeColor::Text, &[], None);
    registry.register_type("number", "数字", "NUM", TypeColor::Number, &[], None);
    registry.register_type("bool", "布尔", "BOOL", TypeColor::Bool, &[], None);
    // 空信号：逻辑节点（顺序执行 / 遍历的「结束」）用它当输出；任何不需要数据的节点
    // 都可以接一个，**纯粹用来定执行先后**（见 `NodeKind::inputs_for`）。
    registry.register_type("void", "空", "-", TypeColor::Unknown, &[], None);

    // 图像格式：id 就是 `ImageFormat::name()`，徽标 / 名字 / 扩展名都取自它 ——
    // 保持「图像格式表」只有一个来源。
    for format in ImageFormat::CONCRETE {
        registry.register_type(
            format.name(),
            format.label(),
            format.badge(),
            TypeColor::Image,
            &["img", "file"],
            Some(format.extension()),
        );
    }
}

/// 一个端口的类型。
///
/// 只存一个 id、一个「是不是接口」的标志，加一个「是不是列表」的标志 —— 具体含义
/// （徽标、名字、颜色、实现的接口）全部去 [`types`] 里查。因此新增类型不需要改这个
/// 类型本身。
///
/// **列表**：`list` 为真时表示 `List[元素]`，元素就是 `id` / `is_trait` 描述的那个
/// 类型。列表只存一层（元素本身不能再是列表）—— 目前的场景用不上嵌套。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortType {
    id: &'static str,
    is_trait: bool,
    list: bool,
}

#[allow(non_upper_case_globals)] // `Any` / `Text` / …… 故意用类型名，读起来才像类型。
impl PortType {
    /// 通配接口：什么都能接。
    pub const Any: PortType = PortType {
        id: "any",
        is_trait: true,
        list: false,
    };
    /// 能写盘的产物接口。
    pub const File: PortType = PortType {
        id: "file",
        is_trait: true,
        list: false,
    };
    /// 文本。
    pub const Text: PortType = PortType {
        id: "text",
        is_trait: false,
        list: false,
    };
    /// 数字。
    pub const Number: PortType = PortType {
        id: "number",
        is_trait: false,
        list: false,
    };
    /// 布尔。
    pub const Bool: PortType = PortType {
        id: "bool",
        is_trait: false,
        list: false,
    };
    /// 空信号 —— 逻辑节点用它定执行先后，本身不携带数据。
    pub const Void: PortType = PortType {
        id: "void",
        is_trait: false,
        list: false,
    };

    /// 指向一个接口（trait）。
    pub fn trait_(id: &'static str) -> Self {
        Self {
            id,
            is_trait: true,
            list: false,
        }
    }

    /// 指向一个具体类型。
    pub fn concrete(id: &'static str) -> Self {
        Self {
            id,
            is_trait: false,
            list: false,
        }
    }

    /// `List[inner]` —— 和 `inner` 同样的元素，只是包成了列表。
    pub fn list_of(inner: PortType) -> Self {
        Self {
            id: inner.id,
            is_trait: inner.is_trait,
            list: true,
        }
    }

    /// `List[Any]` —— 任意列表。
    pub fn any_list() -> Self {
        Self::list_of(Self::Any)
    }

    /// 图像类型：格式确定就是那个具体格式，格式未知就是「图像」这个接口。
    ///
    /// 名字保持大写，是为了让调用点读起来仍像原来的 `PortType::Image(Png)`。
    #[allow(non_snake_case)]
    pub fn Image(format: ImageFormat) -> Self {
        match format {
            ImageFormat::Any => Self::trait_("img"),
            concrete => Self::concrete(concrete.name()),
        }
    }

    /// 视频类型：与图像同理。
    #[cfg(feature = "video")]
    #[allow(non_snake_case)]
    pub fn Video(format: crate::features::video::VideoFormat) -> Self {
        use crate::features::video::VideoFormat;
        match format {
            VideoFormat::Any => Self::trait_("vid"),
            concrete => Self::concrete(concrete.name()),
        }
    }

    pub fn id(self) -> &'static str {
        self.id
    }

    pub fn is_trait(self) -> bool {
        self.is_trait
    }

    /// 这是不是一个列表类型 `List[元素]`。
    pub fn is_list(self) -> bool {
        self.list
    }

    /// 列表的元素类型；不是列表就返回自己。
    pub fn element(self) -> PortType {
        PortType {
            list: false,
            ..self
        }
    }

    fn info(self) -> Option<&'static TypeInfo> {
        types().get(self.id)
    }

    /// 运行期真的拿到它时，这个类型算不算「说清楚了」。
    /// 接口不是具体类型 —— 通配、格式未知的图像都不是。
    pub fn is_concrete(self) -> bool {
        !self.is_trait
    }

    /// 这个端口（或值）算不算图像：是不是「图像」接口，或实现了它。列表不算图像。
    pub fn is_image(self) -> bool {
        !self.list && self.implements_trait("img")
    }

    /// 是图像时，把格式取出来。接口「图像」返回 [`ImageFormat::Any`]。列表返回 `None`。
    pub fn image_format(self) -> Option<ImageFormat> {
        if self.list {
            return None;
        }
        if self.id == "img" {
            return Some(ImageFormat::Any);
        }
        if !self.is_trait && self.implements_trait("img") {
            return ImageFormat::from_name(self.id);
        }
        None
    }

    /// 这个类型实现了接口 `trait_id` 吗。`ANY` 是一切的接口；**列表只认 `ANY`**
    /// （一个 `List[Img]` 不是「图像」，也不是「文件」）。
    pub fn implements(self, trait_id: &str) -> bool {
        if trait_id == "any" {
            return true;
        }
        if self.list {
            return false;
        }
        self.implements_trait(trait_id)
    }

    /// 只看元素类型 / 本身实现的接口，**不管是不是列表**。内部用。
    fn implements_trait(self, trait_id: &str) -> bool {
        if trait_id == "any" {
            return true;
        }
        let Some(info) = self.info() else {
            return false;
        };
        if info.is_trait && self.id == trait_id {
            return true;
        }
        info.traits.contains(&trait_id)
    }

    /// 端口徽标上的短标签。列表是 `[元素徽标]`。
    pub fn badge(self) -> &'static str {
        match self.info() {
            Some(info) if self.list => info.list_badge,
            Some(info) => info.badge,
            None => "?",
        }
    }

    /// 界面上完整一点的名字。`List[元素]` 读作「列表 · 元素」。
    pub fn label(self) -> &'static str {
        match self.info() {
            Some(info) if self.list => info.list_label,
            Some(info) => info.label,
            None => self.id,
        }
    }

    /// 徽标的语义色。列表沿用元素的颜色（同一个色系，一眼能看出元素是什么）。
    pub fn color(self) -> TypeColor {
        self.info().map_or(TypeColor::Unknown, |info| info.color)
    }

    /// 写盘时该用的主扩展名（具体类型才有）。
    pub fn extension(self) -> Option<&'static str> {
        self.info().and_then(|info| info.extension)
    }

    /// 编辑期检查：`self` 作为目标端口，能否接受 `source` 端口的数据。
    ///
    /// 比 [`PortType::strictly_accepts`] 宽松的地方：通配一头出现就放行；
    /// 来源是接口（格式还没定）时也先放行 —— 都等运行期再确认。
    /// 列表只能接列表（元素按同样的规则）。
    pub fn accepts(self, source: PortType) -> bool {
        // 通配（非列表）出现一头就放行。
        if (self.id == "any" && !self.list) || (source.id == "any" && !source.list) {
            return true;
        }
        // 列表 / 非列表必须一致。
        if self.list != source.list {
            return false;
        }
        // 来源是个接口：格式不定，先让它接上，运行期再较真。
        if source.is_trait {
            return true;
        }
        if self.is_trait {
            return source.implements_trait(self.id);
        }
        // 目标是具体类型：只认它自己。
        self.id == source.id
    }

    /// 运行期检查：端口声明 `self`，实际拿到的值是 `source`，是否严格匹配。
    ///
    /// 列表与非列表必须一致；标量还要求来源是一个**具体实例**（格式未知的图像不行）；
    /// 列表则在元素层面比 —— 于是「元素是接口」的列表（比如混合格式目录产出的
    /// `List[Img]`）能接进 `List[Any]` / `List[Img]`。
    pub fn strictly_accepts(self, source: PortType) -> bool {
        if self.list != source.list {
            return false;
        }
        // 标量：运行期的来源永远是一个具体实例。
        if !self.list && !source.is_concrete() {
            return false;
        }
        let target = self.element();
        let source = source.element();
        if target.id == "any" {
            return true;
        }
        if source.id == "any" {
            return false;
        }
        if target.is_trait {
            return source.implements_trait(target.id);
        }
        target.id == source.id
    }
}

impl Serialize for PortType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.list {
            serializer.serialize_str(&format!("list:{}", self.id))
        } else {
            serializer.serialize_str(self.id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_is_a_plain_id() {
        assert_eq!(serde_json::to_string(&PortType::Text).unwrap(), "\"text\"");
        assert_eq!(serde_json::to_string(&PortType::Any).unwrap(), "\"any\"");
        assert_eq!(
            serde_json::to_string(&PortType::Image(ImageFormat::Png)).unwrap(),
            "\"png\""
        );
        assert_eq!(serde_json::to_string(&PortType::File).unwrap(), "\"file\"");
    }

    /// 具体图像只接同一种；「图像」接口收下所有实现它的格式。
    #[test]
    fn image_formats_only_flow_into_their_own_or_the_interface() {
        let png = PortType::Image(ImageFormat::Png);
        let jpg = PortType::Image(ImageFormat::Jpeg);
        let any = PortType::Image(ImageFormat::Any); // 图像接口

        assert!(any.accepts(png));
        assert!(any.accepts(jpg));
        assert!(png.accepts(png));
        assert!(!png.accepts(jpg));
        assert!(!PortType::Text.accepts(png));
        assert!(!png.accepts(PortType::Text));

        // 编辑期放行「格式未知」，运行期不放行。
        assert!(png.accepts(any));
        assert!(!png.strictly_accepts(any));
        assert!(any.strictly_accepts(jpg));
        assert!(!any.strictly_accepts(any));
        assert!(png.strictly_accepts(png));
        assert!(!png.strictly_accepts(jpg));
    }

    /// 通配什么都能接；运行期要的是一个说得清的实例。
    #[test]
    fn wildcard_port_takes_anything() {
        let any = PortType::Any;
        let sources = [
            PortType::Text,
            PortType::Number,
            PortType::Bool,
            PortType::Image(ImageFormat::Png),
            PortType::Image(ImageFormat::Any),
        ];
        for source in sources {
            assert!(any.accepts(source), "编辑期通配什么都能接：{source:?}");
            assert!(
                source.accepts(any),
                "通配送出来的东西，编辑期也能接到任何端点上"
            );
        }

        assert!(any.strictly_accepts(PortType::Text));
        assert!(any.strictly_accepts(PortType::Image(ImageFormat::Png)));
        assert!(!any.strictly_accepts(PortType::Any));
        assert!(!any.strictly_accepts(PortType::Image(ImageFormat::Any)));
    }

    /// `FILE` 接口收下所有实现它的具体类型（图像、以后还有视频……），
    /// 但收不下一个纯标量。
    #[test]
    fn file_interface_takes_saveable_products() {
        let png = PortType::Image(ImageFormat::Png);
        assert!(PortType::File.accepts(png));
        assert!(PortType::File.strictly_accepts(png));
        assert!(!PortType::File.accepts(PortType::Text));
        assert!(!PortType::File.strictly_accepts(PortType::Text));
    }

    /// 每一类类型都带了徽标、名字、颜色 —— 登记表里没有漏项。
    #[test]
    fn every_registered_type_is_described() {
        for info in types().entries.values() {
            assert!(!info.badge.is_empty(), "{} 没有徽标", info.id);
            assert!(!info.label.is_empty(), "{} 没有名字", info.id);
        }
        assert!(PortType::Image(ImageFormat::Png).extension().is_some());
        assert!(PortType::Text.extension().is_none());
    }

    /// 列表在其元素上是协变的；列表与非列表不能互接。
    #[test]
    fn lists_are_covariant_in_their_element() {
        let list_png = PortType::list_of(PortType::Image(ImageFormat::Png));
        let list_jpg = PortType::list_of(PortType::Image(ImageFormat::Jpeg));
        // `List[Img]`：元素是「图像」接口。
        let list_img = PortType::list_of(PortType::Image(ImageFormat::Any));
        let list_any = PortType::any_list();

        assert!(list_any.accepts(list_png));
        assert!(list_img.accepts(list_png));
        assert!(!list_png.accepts(list_jpg), "具体元素不兼容");
        assert!(
            list_png.accepts(list_any),
            "编辑期：来源是接口（格式未定）先放行"
        );

        // 列表和单个元素不能互接。
        let png = PortType::Image(ImageFormat::Png);
        assert!(!png.accepts(list_png));
        assert!(!list_png.accepts(png));

        // 运行时：`List[Img]` 两种图像都收；`List[Png]` 只收 PNG。
        assert!(list_img.strictly_accepts(list_png));
        assert!(list_img.strictly_accepts(list_jpg));
        assert!(list_png.strictly_accepts(list_png));
        assert!(!list_png.strictly_accepts(list_jpg));
        // `List[Any]` 收得下任何列表（包括元素是接口的那些）。
        assert!(list_any.strictly_accepts(list_png));
        assert!(list_any.strictly_accepts(list_any));

        // 列表不是图像、也不是文件；只有通配收得下它。
        assert!(!list_png.is_image());
        assert!(!list_png.implements("file"));
        assert!(list_png.implements("any"));
        assert!(PortType::Any.accepts(list_png));

        // 徽标带方括号，一眼看出是列表。
        assert_eq!(list_png.badge(), "[PNG]");
        assert_eq!(list_any.badge(), "[ANY]");
    }

    /// 空信号只能接空信号（通配除外）。
    #[test]
    fn void_only_connects_to_void() {
        assert!(PortType::Void.accepts(PortType::Void));
        assert!(PortType::Void.strictly_accepts(PortType::Void));
        assert!(!PortType::Void.accepts(PortType::Text));
        assert!(!PortType::Text.accepts(PortType::Void));
        assert!(PortType::Any.accepts(PortType::Void));
        assert_eq!(PortType::Void.badge(), "-");
    }
}
