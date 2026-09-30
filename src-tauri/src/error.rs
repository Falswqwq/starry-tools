//! 错误类型。
//!
//! 分成两层：命令层用 [`AppError`]（要序列化成字符串交给前端），节点层用
//! [`NodeError`]（要按节点归类，写进运行报告里）。

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("文件读写失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("图像处理失败：{0}")]
    Image(#[from] image::ImageError),
    #[error("数据格式错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("找不到工作流「{0}」")]
    WorkflowNotFound(String),
}

impl AppError {
    pub fn msg(message: impl Into<String>) -> Self {
        AppError::Message(message.into())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// 单个节点执行失败的原因。
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct NodeError(pub String);

impl NodeError {
    pub fn new(message: impl Into<String>) -> Self {
        NodeError(message.into())
    }

    /// 给底层错误加一层上下文，例如「解码图像：unexpected EOF」。
    pub fn context(self, context: &str) -> Self {
        NodeError(format!("{context}：{}", self.0))
    }
}

impl From<std::io::Error> for NodeError {
    fn from(err: std::io::Error) -> Self {
        NodeError(err.to_string())
    }
}

impl From<image::ImageError> for NodeError {
    fn from(err: image::ImageError) -> Self {
        NodeError(err.to_string())
    }
}
