//! 注册式类型系统。构建期允许拥有字符串的扩展，冻结后只读。
//! 类型 ID 和展示文本在启动期统一去重驻留，使 `PortType` 保持 `Copy` 和零分配查询。

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::error::NodeError;
use crate::model::port_type::ImageFormat;
use crate::model::value::Value;
use crate::registry::RegistrationError;

/// 一次隐式类型转换。只接收已经确认属于 `from` 的值。
pub type ConvertFn = Arc<dyn Fn(&Value) -> Result<Value, NodeError> + Send + Sync>;

/// GUI 无关的 RGB 值；扩展可以直接贡献新颜色，无需增加枚举分支。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TypeColor(pub [u8; 3]);

impl TypeColor {
    pub const UNKNOWN: Self = Self::rgb(0x94, 0xa3, 0xb8);
    pub const TEXT: Self = Self::rgb(0x33, 0x41, 0x55);
    pub const NUMBER: Self = Self::rgb(0x47, 0x55, 0x69);
    pub const BOOL: Self = Self::rgb(0x64, 0x74, 0x8b);
    pub const IMAGE: Self = Self::rgb(0x3b, 0x82, 0xf6);
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b])
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub badge: &'static str,
    pub color: TypeColor,
    pub traits: Vec<String>,
    pub is_trait: bool,
    pub extension: Option<&'static str>,
    pub list_badge: &'static str,
    pub list_label: &'static str,
}

/// 仅构建器调用：每个不同字符串在进程内只驻留一次；查询/每帧绘制不泄漏。
/// 接受动态字符串后调用方可立即释放原值。冻结表和 `PortType` 共用稳定文本。
fn intern(text: &str) -> &'static str {
    static TEXTS: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Default::default);
    let mut texts = TEXTS.lock().expect("type metadata interner poisoned");
    if let Some(existing) = texts.get(text) {
        return existing;
    }
    let text = Box::leak(text.to_owned().into_boxed_str());
    texts.insert(text);
    text
}

#[derive(Default)]
pub struct RegistryBuilder {
    entries: BTreeMap<&'static str, TypeInfo>,
    conversions: BTreeMap<(&'static str, &'static str), ConvertFn>,
}

impl RegistryBuilder {
    pub fn register_trait(
        &mut self,
        id: &str,
        label: &str,
        badge: &str,
        color: TypeColor,
        supertraits: &[&str],
    ) -> Result<PortType, RegistrationError> {
        self.register(id, label, badge, color, supertraits, true, None)
    }

    pub fn register_type(
        &mut self,
        id: &str,
        label: &str,
        badge: &str,
        color: TypeColor,
        traits: &[&str],
        extension: Option<&str>,
    ) -> Result<PortType, RegistrationError> {
        self.register(id, label, badge, color, traits, false, extension)
    }

    #[allow(clippy::too_many_arguments)]
    fn register(
        &mut self,
        id: &str,
        label: &str,
        badge: &str,
        color: TypeColor,
        traits: &[&str],
        is_trait: bool,
        extension: Option<&str>,
    ) -> Result<PortType, RegistrationError> {
        if self.entries.contains_key(id) {
            return Err(RegistrationError::Duplicate {
                kind: "type",
                id: id.to_owned(),
            });
        }
        let id = intern(id);
        self.entries.insert(
            id,
            TypeInfo {
                id,
                label: intern(label),
                badge: intern(badge),
                color,
                traits: traits.iter().map(|id| (*id).to_owned()).collect(),
                is_trait,
                extension: extension.map(intern),
                list_badge: intern(&format!("[{badge}]")),
                list_label: intern(&format!("列表 · {label}")),
            },
        );
        Ok(PortType {
            id,
            is_trait,
            list: false,
        })
    }

    /// 声明 `from` 可以隐式转换为 `to`。
    ///
    /// 只支持标量具体类型；列表和接口不参与隐式转换。转换失败会阻止该节点运行。
    pub fn register_conversion(
        &mut self,
        from: PortType,
        to: PortType,
        convert: impl Fn(&Value) -> Result<Value, NodeError> + Send + Sync + 'static,
    ) -> Result<(), RegistrationError> {
        if from.list
            || to.list
            || from.is_trait
            || to.is_trait
            || from.id == to.id
            || !self.entries.contains_key(from.id)
            || !self.entries.contains_key(to.id)
        {
            return Err(RegistrationError::InvalidConversion {
                from: from.id.to_owned(),
                to: to.id.to_owned(),
            });
        }
        let key = (from.id, to.id);
        if self.conversions.contains_key(&key) {
            return Err(RegistrationError::Duplicate {
                kind: "conversion",
                id: format!("{} -> {}", from.id, to.id),
            });
        }
        self.conversions.insert(key, Arc::new(convert));
        Ok(())
    }

    pub fn freeze(self) -> Result<Registry, RegistrationError> {
        for info in self.entries.values() {
            for parent in &info.traits {
                if !self
                    .entries
                    .get(parent.as_str())
                    .is_some_and(|entry| entry.is_trait)
                {
                    return Err(RegistrationError::Unknown {
                        kind: "type interface",
                        id: parent.clone(),
                    });
                }
            }
        }
        let registry = Registry {
            entries: self.entries,
            conversions: self.conversions,
        };
        // 继承图必须无环，否则“最具体的共同接口”没有明确含义。
        for info in registry.entries.values() {
            for parent in &info.traits {
                if registry.implements_id(parent, info.id) {
                    return Err(RegistrationError::TypeCycle(info.id.into()));
                }
            }
        }
        Ok(registry)
    }
}

pub struct Registry {
    entries: BTreeMap<&'static str, TypeInfo>,
    conversions: BTreeMap<(&'static str, &'static str), ConvertFn>,
}

impl Registry {
    pub fn get(&self, id: &str) -> Option<&TypeInfo> {
        self.entries.get(id)
    }
    /// 按稳定的 ID 顺序枚举。
    pub fn all(&self) -> impl Iterator<Item = &TypeInfo> {
        self.entries.values()
    }
    pub fn port_type(&self, id: &str) -> Option<PortType> {
        let info = self.get(id)?;
        Some(PortType {
            id: info.id,
            is_trait: info.is_trait,
            list: false,
        })
    }

    fn implements_id(&self, id: &str, target: &str) -> bool {
        let mut pending = vec![id];
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let Some(info) = self.get(id) else {
                continue;
            };
            if info.is_trait && info.id == target {
                return true;
            }
            pending.extend(info.traits.iter().map(String::as_str));
        }
        false
    }

    pub fn implements(&self, ty: PortType, trait_id: &str) -> bool {
        trait_id == "any" || (!ty.list && self.implements_id(ty.id, trait_id))
    }

    /// 是否存在一条 `from -> to` 的隐式转换。只认直接声明的规则。
    pub fn can_convert(&self, from: PortType, to: PortType) -> bool {
        !from.list
            && !to.list
            && !from.is_trait
            && !to.is_trait
            && self.conversions.contains_key(&(from.id, to.id))
    }

    /// 尝试把 `value` 从 `from` 转成 `to`。
    ///
    /// `Ok(None)` 表示没有这条规则；`Err` 表示规则存在但值无法转换。
    pub fn convert_value(
        &self,
        from: PortType,
        to: PortType,
        value: &Value,
    ) -> Result<Option<Value>, NodeError> {
        if !self.can_convert(from, to) {
            return Ok(None);
        }
        self.conversions[&(from.id, to.id)](value).map(Some)
    }

    /// 同型保留具体类型；否则选共同接口中不再有更具体候选的那个。
    /// 不可比较的多个最具体接口用 ID 字典序打破平局，避免 HashMap 顺序影响结果。
    pub fn common_type(&self, types: &[PortType]) -> PortType {
        let Some(&first) = types.first() else {
            return PortType::Any;
        };
        // 端口当前只支持一层列表，嵌套列表不能伪装成其标量元素。
        if types.iter().any(|ty| ty.is_list()) {
            return PortType::Any;
        }
        if types.iter().all(|ty| *ty == first) {
            return first;
        }
        let candidates: Vec<_> = self
            .entries
            .values()
            .filter(|info| info.is_trait && types.iter().all(|ty| self.implements(*ty, info.id)))
            .collect();
        candidates
            .iter()
            .find(|candidate| {
                !candidates.iter().any(|other| {
                    other.id != candidate.id
                        && (candidate.id == "any" || self.implements_id(other.id, candidate.id))
                })
            })
            .map_or(PortType::Any, |info| PortType::trait_(info.id))
    }
}

/// 与节点表使用同一个冻结快照，扩展必须在首次查询前安装。
pub fn types() -> &'static Registry {
    crate::registry::registry().types()
}

/// 两个端口类型之间是否存在隐式转换。
pub fn can_convert(from: PortType, to: PortType) -> bool {
    types().can_convert(from, to)
}

/// 按已注册规则转换一个值。`Ok(None)` 表示没有可用规则。
pub fn convert_value(
    from: PortType,
    to: PortType,
    value: &Value,
) -> Result<Option<Value>, NodeError> {
    types().convert_value(from, to, value)
}

pub(crate) fn register_builtin(registry: &mut RegistryBuilder) -> Result<(), RegistrationError> {
    registry.register_trait("any", "任意值", "ANY", TypeColor::UNKNOWN, &[])?;
    registry.register_trait("file", "文件", "FILE", TypeColor::UNKNOWN, &[])?;
    registry.register_trait("img", "图像", "IMG", TypeColor::IMAGE, &["file"])?;
    let text = registry.register_type("text", "文本", "TXT", TypeColor::TEXT, &[], None)?;
    let number = registry.register_type("number", "数字", "NUM", TypeColor::NUMBER, &[], None)?;
    registry.register_type("bool", "布尔", "BOOL", TypeColor::BOOL, &[], None)?;
    registry.register_type("void", "空", "-", TypeColor::UNKNOWN, &[], None)?;

    registry.register_conversion(number, text, |value| match value.inner() {
        Value::Number(number) => Ok(Value::text(number.to_string())),
        other => Err(NodeError::new(format!(
            "数字转文本失败：拿到的是{}",
            other.port_type().label()
        ))),
    })?;
    registry.register_conversion(text, number, |value| match value.inner() {
        Value::Text(text) => text
            .trim()
            .parse::<f64>()
            .map(Value::Number)
            .map_err(|_| NodeError::new(format!("「{text}」不是一个数字"))),
        other => Err(NodeError::new(format!(
            "文本转数字失败：拿到的是{}",
            other.port_type().label()
        ))),
    })?;
    for format in ImageFormat::CONCRETE {
        registry.register_type(
            format.name(),
            format.label(),
            format.badge(),
            TypeColor::IMAGE,
            &["img", "file"],
            Some(format.extension()),
        )?;
    }
    Ok(())
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
        types().implements(self.element(), trait_id)
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
        self.info().map_or(TypeColor::UNKNOWN, |info| info.color)
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

impl<'de> Deserialize<'de> for PortType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        let (list, id) = name
            .strip_prefix("list:")
            .map_or((false, name.as_str()), |id| (true, id));
        let mut ty = types()
            .port_type(id)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown type: {id}")))?;
        ty.list = list;
        Ok(ty)
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
    fn owned_types_support_transitive_interfaces_and_deterministic_common_types() {
        let mut builder = RegistryBuilder::default();
        register_builtin(&mut builder).unwrap();
        let family = format!("custom/{}", "audio");
        builder
            .register_trait(&family, "声音", "AUD", TypeColor::rgb(2, 4, 6), &["file"])
            .unwrap();
        builder
            .register_trait(
                "custom/lossless",
                "无损音频",
                "LA",
                TypeColor::UNKNOWN,
                &[&family],
            )
            .unwrap();
        let a = builder
            .register_type(
                "custom/a",
                "A",
                "A",
                TypeColor::UNKNOWN,
                &["custom/lossless"],
                Some("a"),
            )
            .unwrap();
        let b = builder
            .register_type(
                "custom/b",
                "B",
                "B",
                TypeColor::UNKNOWN,
                &["custom/lossless"],
                None,
            )
            .unwrap();
        assert!(matches!(
            builder.register_type("custom/a", "覆盖", "X", TypeColor::UNKNOWN, &[], None),
            Err(RegistrationError::Duplicate { kind: "type", .. })
        ));
        drop(family);
        let registry = builder.freeze().unwrap();
        assert!(registry.implements(a, "file"));
        assert_eq!(registry.common_type(&[a, b]).id(), "custom/lossless");
        assert_eq!(registry.common_type(&[a, a]), a);
        assert_eq!(
            registry.common_type(&[a, PortType::Image(ImageFormat::Png)]),
            PortType::File
        );
        assert_eq!(registry.common_type(&[a, PortType::Text]), PortType::Any);
        assert_eq!(registry.get("custom/audio").unwrap().color.0, [2, 4, 6]);
        assert_eq!(registry.port_type("custom/a"), Some(a));
        assert_eq!(
            serde_json::to_string(&PortType::list_of(a)).unwrap(),
            "\"list:custom/a\""
        );
    }

    #[test]
    fn incomparable_common_interfaces_use_id_order() {
        let mut builder = RegistryBuilder::default();
        builder
            .register_trait("z", "Z", "Z", TypeColor::UNKNOWN, &[])
            .unwrap();
        builder
            .register_trait("a", "A", "A", TypeColor::UNKNOWN, &[])
            .unwrap();
        let x = builder
            .register_type("x", "X", "X", TypeColor::UNKNOWN, &["z", "a"], None)
            .unwrap();
        let y = builder
            .register_type("y", "Y", "Y", TypeColor::UNKNOWN, &["a", "z"], None)
            .unwrap();
        let registry = builder.freeze().unwrap();
        assert_eq!(registry.common_type(&[x, y]).id(), "a");
        assert_eq!(registry.common_type(&[y, x]).id(), "a");
    }

    #[test]
    fn invalid_interface_graphs_fail_at_freeze() {
        let mut missing = RegistryBuilder::default();
        missing
            .register_trait("a", "A", "A", TypeColor::UNKNOWN, &["missing"])
            .unwrap();
        assert!(matches!(
            missing.freeze(),
            Err(RegistrationError::Unknown {
                kind: "type interface",
                ..
            })
        ));
        let mut cyclic = RegistryBuilder::default();
        cyclic
            .register_trait("a", "A", "A", TypeColor::UNKNOWN, &["b"])
            .unwrap();
        cyclic
            .register_trait("b", "B", "B", TypeColor::UNKNOWN, &["a"])
            .unwrap();
        assert!(matches!(
            cyclic.freeze(),
            Err(RegistrationError::TypeCycle(_))
        ));
    }

    #[test]
    fn type_ids_are_interned_only_once() {
        let mut first = RegistryBuilder::default();
        let mut second = RegistryBuilder::default();
        let a = first
            .register_type(
                &String::from("owned/id"),
                "A",
                "A",
                TypeColor::UNKNOWN,
                &[],
                None,
            )
            .unwrap();
        let b = second
            .register_type(
                &String::from("owned/id"),
                "B",
                "B",
                TypeColor::UNKNOWN,
                &[],
                None,
            )
            .unwrap();
        assert!(std::ptr::eq(a.id(), b.id()));
    }

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

    #[test]
    fn builtin_number_text_conversions_are_declared_and_fallible() {
        let mut builder = RegistryBuilder::default();
        register_builtin(&mut builder).unwrap();
        let registry = builder.freeze().unwrap();

        assert!(registry.can_convert(PortType::Number, PortType::Text));
        assert!(registry.can_convert(PortType::Text, PortType::Number));
        assert!(!registry.can_convert(PortType::Bool, PortType::Text));
        assert!(!registry.can_convert(PortType::Text, PortType::Bool));

        let text = registry
            .convert_value(PortType::Number, PortType::Text, &Value::Number(42.0))
            .unwrap()
            .expect("有转换规则");
        assert_eq!(text.as_text().unwrap(), "42");

        let number = registry
            .convert_value(PortType::Text, PortType::Number, &Value::text(" 3.5 "))
            .unwrap()
            .expect("有转换规则");
        assert_eq!(number.as_number().unwrap(), 3.5);

        assert!(registry
            .convert_value(PortType::Text, PortType::Number, &Value::text("不是数字"))
            .is_err());
    }

    #[test]
    fn custom_types_can_register_conversions() {
        let mut builder = RegistryBuilder::default();
        register_builtin(&mut builder).unwrap();
        let source = builder
            .register_type("custom/source", "源", "SRC", TypeColor::UNKNOWN, &[], None)
            .unwrap();
        let target = builder
            .register_type(
                "custom/target",
                "目标",
                "DST",
                TypeColor::UNKNOWN,
                &[],
                None,
            )
            .unwrap();
        builder
            .register_conversion(source, target, |value| {
                Ok(Value::text(format!("target:{}", value.as_text()?)))
            })
            .unwrap();
        let registry = builder.freeze().unwrap();

        assert!(registry.can_convert(source, target));
        let converted = registry
            .convert_value(source, target, &Value::text("x"))
            .unwrap()
            .unwrap();
        assert_eq!(converted.as_text().unwrap(), "target:x");
        assert!(!registry.can_convert(target, source), "方向必须显式登记");
    }

    #[test]
    fn conversions_reject_traits_lists_and_duplicates() {
        let mut builder = RegistryBuilder::default();
        register_builtin(&mut builder).unwrap();
        assert!(matches!(
            builder.register_conversion(PortType::Any, PortType::Text, |_| Ok(Value::Void)),
            Err(RegistrationError::InvalidConversion { .. })
        ));
        let list = PortType::list_of(PortType::Text);
        assert!(matches!(
            builder.register_conversion(list, PortType::Text, |_| Ok(Value::Void)),
            Err(RegistrationError::InvalidConversion { .. })
        ));
        assert_eq!(
            builder.register_conversion(PortType::Bool, PortType::Text, |_| Ok(Value::Void)),
            Ok(())
        );
        assert!(matches!(
            builder.register_conversion(PortType::Bool, PortType::Text, |_| Ok(Value::Void)),
            Err(RegistrationError::Duplicate {
                kind: "conversion",
                ..
            })
        ));
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
