//! 自动产物的所有权清单。只回收清单中的直接文件，不猜测历史文件名。
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{AppError, NodeError};
use crate::model::workflow::Workflow;
use serde::{Deserialize, Serialize};

const MANIFEST: &str = ".starrytools-products.json";
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_PRODUCTS: usize = 10_000;

/// 对用户输入与 manifest 使用同一跨平台边界，即使在 Linux 也拒绝 Windows 特殊名。
pub(crate) fn validate_file_name(name: &str) -> Result<(), NodeError> {
    let base = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches([' ', '.'])
        .to_uppercase();
    let device = matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(*prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if name.is_empty()
        || name.len() > 240
        || name.ends_with([' ', '.'])
        || device
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        || Path::new(name).file_name().and_then(|part| part.to_str()) != Some(name)
    {
        return Err(NodeError::new(format!(
            "无效文件名「{name}」：必须是安全的单个文件名，不能包含路径、设备名或结尾空格/句点"
        )));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Product {
    size: u64,
    checksum: u32,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    workflow_id: String,
    products: BTreeMap<String, Product>,
}

pub(super) struct Artifacts {
    dir: PathBuf,
    manifest: Manifest,
}

impl Artifacts {
    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(super) fn prepare(root: &Path, workflow: &Workflow, clear: bool) -> Result<Self, AppError> {
        // 完整编码 id，避免同名工作流或净化后同名的 id 互相清理；限制单个路径分量。
        if workflow.id.is_empty() || workflow.id.len() > 64 {
            return Err(AppError::msg("工作流 id 必须为 1–64 字节"));
        }
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let id: String = workflow
            .id
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let dir = root.join(format!("{}--{id}", super::file_slug(&workflow.name)));
        match fs::symlink_metadata(&dir) {
            Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
                return Err(AppError::msg("产物目录不能是链接或普通文件"))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&dir)?,
            Err(error) => return Err(error.into()),
        }
        let dir = dir.canonicalize()?;
        if dir.parent() != Some(root.as_path()) {
            return Err(AppError::msg("产物目录越过了输出根目录"));
        }
        let path = dir.join(MANIFEST);
        let manifest = match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if !meta.is_file()
                    || meta.file_type().is_symlink()
                    || meta.len() > MAX_MANIFEST_BYTES as u64
                {
                    return Err(AppError::msg("产物清单必须是有界的普通文件"));
                }
                let mut bytes = Vec::new();
                fs::File::open(&path)?
                    .take(MAX_MANIFEST_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > MAX_MANIFEST_BYTES {
                    return Err(AppError::msg("产物清单过大"));
                }
                let manifest: Manifest = serde_json::from_slice(&bytes)?;
                if manifest.version != 1
                    || manifest.workflow_id != workflow.id
                    || manifest.products.len() > MAX_PRODUCTS
                {
                    return Err(AppError::msg("产物清单版本、工作流归属或数量无效"));
                }
                for name in manifest.products.keys() {
                    validate_product_name(name)?;
                }
                manifest
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Manifest {
                version: 1,
                workflow_id: workflow.id.clone(),
                products: BTreeMap::new(),
            },
            Err(error) => return Err(error.into()),
        };
        let mut artifacts = Self { dir, manifest };
        if clear {
            artifacts.clear_previous_outputs()?;
        }
        Ok(artifacts)
    }

    fn persist(&self) -> Result<(), AppError> {
        if self.manifest.products.len() > MAX_PRODUCTS {
            return Err(AppError::msg("自动产物数量超过上限"));
        }
        let bytes = serde_json::to_vec(&self.manifest)?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(AppError::msg("产物清单过大"));
        }
        let target = self.dir.join(MANIFEST);
        match fs::symlink_metadata(&target) {
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
                return Err(AppError::msg("产物清单不能是链接或目录"))
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let temp = self.dir.join(format!(
            ".starrytools-manifest-{}.tmp",
            uuid::Uuid::new_v4()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| -> Result<(), AppError> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, &target)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }

    fn clear_previous_outputs(&mut self) -> Result<(), AppError> {
        for (name, product) in &self.manifest.products {
            let path = self.dir.join(name);
            // 用户替换、编辑过的文件，或者链接/目录都不是可回收产物。
            if product_matches(&path, product)? {
                fs::remove_file(path)?;
            }
        }
        self.manifest.products.clear();
        self.persist()
    }

    pub(super) fn write_product(&mut self, name: &str, bytes: &[u8]) -> Result<PathBuf, AppError> {
        validate_product_name(name)?;
        if self.manifest.products.len() >= MAX_PRODUCTS {
            return Err(AppError::msg("自动产物数量超过上限"));
        }
        // 从不覆盖已有文件（包括未登记的旧引擎产物及符号链接）。
        let mut candidate = name.to_string();
        let mut created = None;
        for index in 0..MAX_PRODUCTS {
            if index > 0 {
                let path = Path::new(name);
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
                let suffix = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|ext| format!(".{ext}"))
                    .unwrap_or_default();
                candidate = format!("{stem} ({}){suffix}", index + 1);
                validate_product_name(&candidate)?;
            }
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.dir.join(&candidate))
            {
                Ok(file) => {
                    created = Some(file);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let mut file = created.ok_or_else(|| AppError::msg("找不到可用的产物文件名"))?;
        let path = self.dir.join(&candidate);
        if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error.into());
        }
        drop(file);
        self.manifest.products.insert(
            candidate.clone(),
            Product {
                size: bytes.len() as u64,
                checksum: crc32fast::hash(bytes),
            },
        );
        if let Err(error) = self.persist() {
            self.manifest.products.remove(&candidate);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(path)
    }
}

fn validate_product_name(name: &str) -> Result<(), AppError> {
    validate_file_name(name).map_err(|error| AppError::msg(error.0))?;
    if name.to_ascii_lowercase().starts_with(".starrytools-") {
        return Err(AppError::msg("产物不能使用清单保留名称"));
    }
    Ok(())
}

fn product_matches(path: &Path, product: &Product) -> Result<bool, AppError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != product.size {
        return Ok(false);
    }
    let mut reader = fs::File::open(path)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize() == product.checksum)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("starry-manifest-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn validates_cross_platform_names() {
        for name in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "C:x",
            "\\\\server\\x",
            "NUL.txt",
            "COM1",
            "lpt9.png",
            "CON .txt",
            "COM¹",
            "x.",
            "x ",
            "x\0y",
        ] {
            assert!(validate_file_name(name).is_err(), "{name:?}");
        }
        for name in ["角色.png", "hello (2).png", "COM10.txt", ".hidden"] {
            assert!(validate_file_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn manifest_cleans_three_digit_and_nested_names_but_not_user_files() {
        let root = root();
        let workflow = Workflow::new("same name");
        let mut artifacts = Artifacts::prepare(&root, &workflow, true).unwrap();
        let dir = artifacts.dir.clone();
        fs::write(dir.join("01-user.png"), b"user").unwrap();
        let names = [
            super::super::output_file_name(100, "node", "out", false, "png", &[]),
            super::super::output_file_name(100, "node", "out", false, "png", &[100, 2]),
        ];
        for name in &names {
            artifacts.write_product(name, b"generated").unwrap();
        }
        drop(artifacts);
        Artifacts::prepare(&root, &workflow, true).unwrap();
        for name in &names {
            assert!(!dir.join(name).exists());
        }
        assert!(dir.join("01-user.png").exists());
    }

    #[test]
    fn collisions_and_user_edits_are_preserved() {
        let root = root();
        let workflow = Workflow::new("collisions");
        let mut artifacts = Artifacts::prepare(&root, &workflow, true).unwrap();
        let original = artifacts.dir.join("01-node.png");
        fs::write(&original, b"user").unwrap();
        let generated = artifacts.write_product("01-node.png", b"image").unwrap();
        assert_ne!(original, generated);
        fs::write(&generated, b"edited by user").unwrap();
        Artifacts::prepare(&root, &workflow, true).unwrap();
        assert_eq!(fs::read(original).unwrap(), b"user");
        assert_eq!(fs::read(generated).unwrap(), b"edited by user");
    }

    #[test]
    fn invalid_manifest_cannot_escape_directory() {
        let root = root();
        let workflow = Workflow::new("traversal");
        let artifacts = Artifacts::prepare(&root, &workflow, true).unwrap();
        let outside = root.join("outside.txt");
        fs::write(&outside, b"keep").unwrap();
        let malicious = serde_json::json!({"version": 1, "workflow_id": workflow.id, "products": {"../outside.txt": {"size": 4, "checksum": crc32fast::hash(b"keep")}}});
        fs::write(
            artifacts.dir.join(MANIFEST),
            serde_json::to_vec(&malicious).unwrap(),
        )
        .unwrap();
        assert!(Artifacts::prepare(&root, &workflow, true).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"keep");
    }

    #[test]
    fn partial_runs_keep_ownership_and_repeated_full_runs_are_stable() {
        let root = root();
        let workflow = Workflow::new("partial");
        let mut first = Artifacts::prepare(&root, &workflow, true).unwrap();
        let old = first.write_product("100-node.png", b"old").unwrap();
        drop(first);
        let mut partial = Artifacts::prepare(&root, &workflow, false).unwrap();
        let second = partial.write_product("100-node.png", b"new").unwrap();
        assert_ne!(old, second);
        assert_eq!(fs::read(&old).unwrap(), b"old");
        drop(partial);
        for _ in 0..2 {
            let mut next = Artifacts::prepare(&root, &workflow, true).unwrap();
            assert!(!old.exists());
            assert!(!second.exists());
            assert_eq!(next.write_product("100-node.png", b"old").unwrap(), old);
        }
    }

    #[test]
    fn manifest_size_and_workflow_path_boundaries_are_enforced() {
        let root = root();
        let mut workflow = Workflow::new("../../CON ");
        workflow.id = "../outside".into();
        let artifacts = Artifacts::prepare(&root, &workflow, true).unwrap();
        assert_eq!(
            artifacts.dir.parent(),
            Some(root.canonicalize().unwrap().as_path())
        );
        let manifest = fs::File::create(artifacts.dir.join(MANIFEST)).unwrap();
        manifest.set_len(MAX_MANIFEST_BYTES as u64 + 1).unwrap();
        drop(manifest);
        assert!(Artifacts::prepare(&root, &workflow, true).is_err());
    }

    #[test]
    fn failed_manifest_write_rolls_back_the_new_product() {
        let root = root();
        let workflow = Workflow::new("rollback");
        let mut artifacts = Artifacts::prepare(&root, &workflow, false).unwrap();
        fs::create_dir(artifacts.dir.join(MANIFEST)).unwrap();
        assert!(artifacts.write_product("01-node.png", b"image").is_err());
        assert!(!artifacts.dir.join("01-node.png").exists());
        assert!(artifacts.manifest.products.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn linked_artifacts_and_output_directories_are_not_followed() {
        use std::os::unix::fs::symlink;
        let root = root();
        let workflow = Workflow::new("symlink");
        let mut artifacts = Artifacts::prepare(&root, &workflow, true).unwrap();
        let file = artifacts.write_product("01-node.png", b"same").unwrap();
        let outside = root.join("outside.png");
        fs::write(&outside, b"same").unwrap();
        fs::remove_file(&file).unwrap();
        symlink(&outside, &file).unwrap();
        Artifacts::prepare(&root, &workflow, true).unwrap();
        assert_eq!(fs::read(outside).unwrap(), b"same");
        let directory = artifacts.dir.clone();
        fs::rename(&directory, root.join("moved")).unwrap();
        symlink(root.join("moved"), &directory).unwrap();
        assert!(Artifacts::prepare(&root, &workflow, true).is_err());
    }
}
