//! 工作流的磁盘存储：一个工作流一个 JSON 文件。

use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::model::workflow::{now_millis, Workflow, WorkflowSummary};

pub struct Storage {
    root: PathBuf,
}

impl Clone for Storage {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
        }
    }
}

impl Storage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure(&self) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// 按最近修改排序列出全部工作流。
    pub fn list(&self) -> Result<Vec<WorkflowSummary>, AppError> {
        self.ensure()?;
        let mut summaries = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            // 单个文件坏掉不该让整个列表打不开。
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(workflow) = serde_json::from_str::<Workflow>(&text) {
                    summaries.push(WorkflowSummary::from(&workflow));
                }
            }
        }
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.updated_at.unwrap_or(0)));
        Ok(summaries)
    }

    pub fn load(&self, id: &str) -> Result<Workflow, AppError> {
        let path = self.path_for(id)?;
        if !path.exists() {
            return Err(AppError::WorkflowNotFound(id.to_string()));
        }
        let text = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&text)?)
    }

    /// 写入。会补齐 id、时间戳，并返回最终落盘的内容。
    pub fn save(&self, workflow: &Workflow) -> Result<Workflow, AppError> {
        self.ensure()?;
        let mut workflow = workflow.clone();
        if workflow.id.trim().is_empty() {
            workflow.id = uuid::Uuid::new_v4().to_string();
        }
        if workflow.name.trim().is_empty() {
            workflow.name = "未命名工作流".to_string();
        }
        let now = now_millis();
        if workflow.created_at.is_none() {
            workflow.created_at = Some(now);
        }
        workflow.updated_at = Some(now);

        let path = self.path_for(&workflow.id)?;
        // 先写临时文件再改名，避免写到一半崩溃留下半个 JSON。
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_string_pretty(&workflow)?)?;
        std::fs::rename(&temp, &path)?;
        Ok(workflow)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let path = self.path_for(id)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// 防止 id 里混进路径分隔符跑到目录外面去。
    fn path_for(&self, id: &str) -> Result<PathBuf, AppError> {
        let safe: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if safe.is_empty() {
            return Err(AppError::msg("工作流 id 不合法"));
        }
        Ok(self.root.join(format!("{safe}.json")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::workflow::NodeInstance;

    fn temp_storage(name: &str) -> Storage {
        let root =
            std::env::temp_dir().join(format!("starrytools-test-{name}-{}", uuid::Uuid::new_v4()));
        Storage::new(root)
    }

    #[test]
    fn save_list_load_delete() {
        let storage = temp_storage("crud");

        let mut workflow = Workflow::new("给像素画放大");
        workflow.nodes.push(NodeInstance {
            id: "n1".into(),
            kind: "input".into(),
            position: Default::default(),
            params: Default::default(),
        });
        let saved = storage.save(&workflow).unwrap();
        assert_eq!(saved.name, "给像素画放大");

        let listed = storage.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].node_count, 1);

        let loaded = storage.load(&saved.id).unwrap();
        assert_eq!(loaded.nodes.len(), 1);

        storage.delete(&saved.id).unwrap();
        assert!(storage.list().unwrap().is_empty());
    }

    #[test]
    fn rejects_ids_that_escape_the_directory() {
        let storage = temp_storage("escape");
        assert!(storage.load("../../etc/passwd").is_err());
        assert!(storage.load("").is_err());
    }
}
