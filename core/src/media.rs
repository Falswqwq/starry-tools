//! 通用媒体产物。
//!
//! 图像有专门的 [`crate::image_io::ImageValue`]（带解码缓存、能出预览）。
//! 视频、以及以后可能的音频这类产物，只需要「类型 + 编码后的字节 + 一点元信息」，
//! 就用这里的 [`MediaValue`]。它不假设自己能被解码，一切以字节为准。
//!
//! 类型用登记表里的具体类型 id 表示（见 [`crate::types`]），写盘用的扩展名也从
//! 登记表里取 —— 因此加一种新格式不需要改这里。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{AppError, NodeError};
use crate::model::value::human_size;
use crate::types::{types, PortType};

/// 展示用的元信息：键值对（时长、分辨率、编码器……）。
pub type Meta = Vec<(String, String)>;

#[derive(Clone)]
pub struct MediaValue {
    type_id: &'static str,
    origin: Option<PathBuf>,
    bytes: Arc<Vec<u8>>,
    meta: Meta,
}

impl MediaValue {
    pub fn from_bytes(type_id: &'static str, bytes: Vec<u8>) -> Self {
        Self {
            type_id,
            origin: None,
            bytes: Arc::new(bytes),
            meta: Vec::new(),
        }
    }

    /// 从磁盘读入，类型由调用方给定（调用方才知道它是什么产物）。
    pub fn open(type_id: &'static str, path: &Path) -> Result<Self, NodeError> {
        let bytes = std::fs::read(path)
            .map_err(|err| NodeError::new(format!("无法读取文件 {}：{err}", path.display())))?;
        Ok(Self {
            type_id,
            origin: Some(path.to_path_buf()),
            bytes: Arc::new(bytes),
            meta: Vec::new(),
        })
    }

    /// 只包住一段已经在内存里的字节，不复制。
    pub fn with_bytes(type_id: &'static str, bytes: Arc<Vec<u8>>) -> Self {
        Self {
            type_id,
            origin: None,
            bytes,
            meta: Vec::new(),
        }
    }

    pub fn type_id(&self) -> &'static str {
        self.type_id
    }

    /// 这个值在类型系统里的类型（一个具体类型）。
    pub fn port_type(&self) -> PortType {
        PortType::concrete(self.type_id)
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    pub fn origin(&self) -> Option<&Path> {
        self.origin.as_deref()
    }

    pub fn meta(&self) -> &[(String, String)] {
        &self.meta
    }

    pub fn with_origin(mut self, origin: Option<PathBuf>) -> Self {
        self.origin = origin;
        self
    }

    pub fn with_meta(mut self, meta: Meta) -> Self {
        self.meta = meta;
        self
    }

    /// 把另一个产物的「出处」搬过来（原始文件），下游写盘时好起名。
    pub fn inherit_provenance(mut self, source: &MediaValue) -> Self {
        self.origin = source.origin.clone();
        self
    }

    /// 写盘用的主扩展名 —— 来自类型登记表。
    pub fn extension(&self) -> &'static str {
        types()
            .get(self.type_id)
            .and_then(|info| info.extension)
            .unwrap_or("bin")
    }

    pub fn save_to(&self, path: &Path) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.bytes.as_slice())?;
        Ok(())
    }

    /// 界面上的一行摘要。
    pub fn describe(&self) -> String {
        let label = types()
            .get(self.type_id)
            .map_or(self.type_id, |info| info.label);
        let mut text = format!("{label} · {}", human_size(self.byte_len()));
        for (key, value) in &self.meta {
            text.push_str(&format!(" · {key} {value}"));
        }
        text
    }
}

impl std::fmt::Debug for MediaValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaValue")
            .field("type_id", &self.type_id)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
