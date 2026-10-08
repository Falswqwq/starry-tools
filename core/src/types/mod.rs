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
/// 只存一个 id 加一个「是不是接口」的标志 —— 具体含义（徽标、名字、颜色、
/// 实现的接口）全部去 [`types`] 里查。因此新增类型不需要改这个类型本身。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortType {
    id: &'static str,
    is_trait: bool,
}

#[allow(non_upper_case_globals)] // `Any` / `Text` / …… 故意用类型名，读起来才像类型。
impl PortType {
    /// 通配接口：什么都能接。
    pub const Any: PortType = PortType {
        id: "any",
        is_trait: true,
    };
    /// 能写盘的产物接口。
    pub const File: PortType = PortType {
        id: "file",
        is_trait: true,
    };
    /// 文本。
    pub const Text: PortType = PortType {
        id: "text",
        is_trait: false,
    };
    /// 数字。
    pub const Number: PortType = PortType {
        id: "number",
        is_trait: false,
    };
    /// 布尔。
    pub const Bool: PortType = PortType {
        id: "bool",
        is_trait: false,
    };

    /// 指向一个接口（trait）。
    pub fn trait_(id: &'static str) -> Self {
        Self { id, is_trait: true }
    }

    /// 指向一个具体类型。
    pub fn concrete(id: &'static str) -> Self {
        Self {
            id,
            is_trait: false,
        }
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

    fn info(self) -> Option<&'static TypeInfo> {
        types().get(self.id)
    }

    /// 运行期真的拿到它时，这个类型算不算「说清楚了」。
    /// 接口不是具体类型 —— 通配、格式未知的图像都不是。
    pub fn is_concrete(self) -> bool {
        !self.is_trait
    }

    /// 这个端口（或值）算不算图像：是不是「图像」接口，或实现了它。
    pub fn is_image(self) -> bool {
        self.implements("img")
    }

    /// 是图像时，把格式取出来。接口「图像」返回 [`ImageFormat::Any`]。
    pub fn image_format(self) -> Option<ImageFormat> {
        if self.id == "img" {
            return Some(ImageFormat::Any);
        }
        if !self.is_trait && self.implements("img") {
            return ImageFormat::from_name(self.id);
        }
        None
    }

    /// 这个类型实现了接口 `trait_id` 吗。`ANY` 是一切的接口。
    pub fn implements(self, trait_id: &str) -> bool {
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

    /// 端口徽标上的短标签。
    pub fn badge(self) -> &'static str {
        self.info().map_or("?", |info| info.badge)
    }

    /// 界面上完整一点的名字。
    pub fn label(self) -> &'static str {
        self.info().map_or(self.id, |info| info.label)
    }

    /// 徽标的语义色。
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
    pub fn accepts(self, source: PortType) -> bool {
        if self.id == "any" || source.id == "any" {
            return true;
        }
        // 来源是个接口：格式不定，先让它接上，运行期再较真。
        if source.is_trait {
            return true;
        }
        if self.is_trait {
            return source.implements(self.id);
        }
        // 目标是具体类型：只认它自己。
        self.id == source.id
    }

    /// 运行期检查：端口声明 `self`，实际拿到的值是 `source`，是否严格匹配。
    pub fn strictly_accepts(self, source: PortType) -> bool {
        // 运行期的来源永远是一个具体实例。
        if !source.is_concrete() {
            return false;
        }
        if self.is_trait {
            return source.implements(self.id);
        }
        self.id == source.id
    }
}

impl Serialize for PortType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id)
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
}
