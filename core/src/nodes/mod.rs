//! 内置工具。一个文件一个工具；此模块贡献注册项。

#[cfg(feature = "onnx")]
pub mod background_removal;
pub mod border;
pub mod compress;
pub mod convert;
pub mod crop;
pub mod iterate;
pub mod literal;
pub mod palette;
pub mod read;
pub mod read_dir;
pub mod remove_color;
pub mod rename;
pub mod save;
pub mod sequence;
pub mod split;
pub mod transform;
pub mod upscale;

use crate::registry::{CategoryInfo, RegistrationError, RegistryBuilder};
use crate::types::TypeColor;

/// 「逻辑节点」专色 id —— 顺序执行 / 遍历用暗蓝，一眼和普通节点区分开。
pub const LOGIC_ACCENT: &str = "logic";

pub fn register(registry: &mut RegistryBuilder) -> Result<(), RegistrationError> {
    // 分类只用来把节点分组，不带颜色 —— 节点外观是另一套（见下面的专色）。
    for category in ["来源", "图像", "逻辑", "通用", "产出"] {
        registry.register_category(CategoryInfo::new(category, category))?;
    }
    // 逻辑类专色：声明了 `LOGIC_ACCENT` 的节点（顺序执行 / 遍历）边线走它。
    registry.register_accent(LOGIC_ACCENT, TypeColor::rgb(0x1e, 0x40, 0xaf))?;
    for spec in [
        read::spec(),
        read_dir::spec(),
        literal::text_spec(),
        literal::number_spec(),
        literal::bool_spec(),
        convert::spec(),
        compress::spec(),
        border::spec(),
        remove_color::spec(),
    ] {
        registry.register_node(spec)?;
    }
    #[cfg(feature = "onnx")]
    registry.register_node(background_removal::spec())?;
    for spec in [
        palette::spec(),
        crop::spec(),
        transform::spec(),
        upscale::spec(),
        split::spec(),
        sequence::spec(),
        iterate::spec(),
    ] {
        registry.register_node(spec)?;
    }
    // 保持节点库顺序：feature 媒体节点位于通用/产出节点之前。
    #[cfg(feature = "video")]
    crate::features::video::register(registry)?;
    registry.register_node(rename::spec())?;
    registry.register_node(save::spec())?;
    registry.register_alias("convert_to_png", convert::KIND)?;
    registry.register_alias("input", read::KIND)?;
    Ok(())
}
