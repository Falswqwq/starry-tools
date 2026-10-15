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

pub fn register(registry: &mut RegistryBuilder) -> Result<(), RegistrationError> {
    for (category, color) in [
        ("来源", TypeColor::rgb(0x03, 0x69, 0xa1)),
        ("图像", TypeColor::rgb(0x15, 0x80, 0x3d)),
        ("逻辑", TypeColor::rgb(0x1e, 0x40, 0xaf)),
        ("通用", TypeColor::rgb(0x6d, 0x28, 0xd9)),
        ("产出", TypeColor::rgb(0xb4, 0x53, 0x09)),
    ] {
        registry.register_category(CategoryInfo::new(category, category, Some(color)))?;
    }
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
