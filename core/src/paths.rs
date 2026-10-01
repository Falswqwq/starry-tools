//! 数据目录。
//!
//! 整个应用的数据都放在同一个根下：
//!
//! ```text
//! <数据目录>/com.falsw.starrytools/
//!   workflows/   工作流（一个一份 JSON）
//!   outputs/     运行产物
//!   models/      下载的 ONNX 模型
//!   settings.json
//! ```
//!
//! 应用标识符是长期稳定的，**不要改** —— 改了会换一个目录，用户之前保存的东西会看起来
//! 凭空丢了。

use std::path::PathBuf;

/// 应用标识。它决定了数据目录。
pub const IDENTIFIER: &str = "com.falsw.starrytools";

/// 数据根目录。
pub fn data_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(IDENTIFIER)
}

/// 下载的模型放这儿。
pub fn models_dir() -> PathBuf {
    data_root().join("models")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 数据目录**不能改**。
    #[test]
    fn the_data_directory_is_stable() {
        assert!(data_root().ends_with(IDENTIFIER));
        assert_eq!(IDENTIFIER, "com.falsw.starrytools");
        assert!(models_dir().starts_with(data_root()));
    }
}
