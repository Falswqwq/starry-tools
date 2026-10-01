//! StarryTools 的核心。
//!
//! 四个部分：
//!
//! * [`model`] —— 数据形状与类型系统（端口类型、参数、工作流存档格式）；
//! * [`nodes`] —— 每个工具就是一个文件，自报输入 / 输出 / 参数；
//! * [`engine`] —— 静态检查（`resolve`）与真正执行（`run`）；
//! * [`png_opt`] / [`png_quant`] —— PNG 的无损与有损优化。
//!
//! 这一层**不依赖任何 GUI 框架**。界面只是在它之上画；
//! 所以它的测试能脱离窗口独立跑。

pub mod engine;
pub mod error;
pub mod image_io;
pub mod model;
pub mod nodes;
pub mod png_opt;
pub mod png_quant;
pub mod registry;
pub mod storage;

pub use error::AppError;
