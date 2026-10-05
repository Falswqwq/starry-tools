//! 工作流的存取：数据目录、当前打开的是哪一份、有没有未保存的改动。
//!
//! 数据目录沿用应用一直以来的标识符 —— 换了实现也不该让用户已保存的工作流
//! 看起来凭空丢了。

use std::path::PathBuf;

use starrytools_core::model::workflow::{NodeInstance, Position, Workflow, WorkflowSummary};
use starrytools_core::paths;
use starrytools_core::storage::Storage;

use crate::catalog::Kind;
use crate::canvas::graph::Graph;

pub struct Workspace {
    storage: Storage,
    /// 运行产物写到哪（跑工作流时自动落盘的那条路子）。
    pub output_root: PathBuf,
    /// 当前工作流的元信息：id / 名字 / 描述 / 创建时间。
    /// 节点和连线在画布上，保存时由画布填进来。
    current: Workflow,
    /// 上次保存时的画布版本号，用来判断有没有未保存的改动。
    saved_revision: u64,
    /// 最近一条出错提示。
    pub notice: Option<String>,
}

impl Workspace {
    /// 打开工作区，并起一份带「输入」节点的新工作流。
    pub fn open(kinds: &[Kind]) -> Self {
        let root = data_root();
        let output_root = root.join("outputs");
        let _ = std::fs::create_dir_all(&output_root);

        Self {
            storage: Storage::new(root.join("workflows")),
            output_root,
            current: fresh(kinds),
            saved_revision: 0,
            notice: None,
        }
    }

    pub fn workflow(&self) -> &Workflow {
        &self.current
    }

    pub fn workflow_dir(&self) -> String {
        self.storage.root().to_string_lossy().to_string()
    }

    #[allow(dead_code)]
    pub fn output_dir(&self) -> String {
        self.output_root.to_string_lossy().to_string()
    }

    pub fn id(&self) -> &str {
        &self.current.id
    }

    /// 取走一条待显示的错误提示（保存 / 加载 / 删除失败时留下的）。
    /// 拿出来就清掉 —— 界面拿它弹一个提示。
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    pub fn name_mut(&mut self) -> &mut String {
        &mut self.current.name
    }

    pub fn description_mut(&mut self) -> &mut String {
        &mut self.current.description
    }

    /// 有没有未保存的改动。只看画布内容的版本号 —— 平移缩放不算。
    pub fn is_dirty(&self, graph: &Graph) -> bool {
        graph.revision != self.saved_revision
    }

    /// 起一份新的空工作流（照旧自带一个「输入」节点）。
    pub fn new_workflow(&mut self, kinds: &[Kind]) {
        self.current = fresh(kinds);
        self.notice = None;
    }

    pub fn list(&self) -> Vec<WorkflowSummary> {
        self.storage.list().unwrap_or_default()
    }

    /// 保存。成功后 `current` 会被换成落盘后的版本（补齐时间戳等）。
    pub fn save(&mut self, graph: &Graph) -> bool {
        let workflow = graph.to_workflow(&self.current);
        match self.storage.save(&workflow) {
            Ok(saved) => {
                self.current = saved;
                self.saved_revision = graph.revision;
                self.notice = None;
                true
            }
            Err(err) => {
                self.notice = Some(format!("保存失败：{err}"));
                false
            }
        }
    }

    /// 打开一份已保存的工作流。调用方拿到之后要把画布换掉。
    pub fn load(&mut self, id: &str) -> Option<Workflow> {
        match self.storage.load(id) {
            Ok(workflow) => {
                self.current = workflow.clone();
                self.notice = None;
                Some(workflow)
            }
            Err(err) => {
                self.notice = Some(format!("打开失败：{err}"));
                None
            }
        }
    }

    /// 画布换好之后调一次，把「已保存」的基准对上。
    pub fn mark_saved(&mut self, graph: &Graph) {
        self.saved_revision = graph.revision;
    }

    /// 删掉一份存档。删的正好是当前打开的那份时，画布保持原样，不自动新建。
    pub fn remove(&mut self, id: &str) -> bool {
        match self.storage.delete(id) {
            Ok(()) => {
                self.notice = None;
                true
            }
            Err(err) => {
                self.notice = Some(format!("删除失败：{err}"));
                false
            }
        }
    }
}

/// 数据根目录。**唯一的一处定义在 `starrytools_core::paths`** —— 工作流、产物、
/// 模型、设置都放在同一个根下。这里只是给它起个短名字方便调用。
pub fn data_root() -> PathBuf {
    paths::data_root()
}

/// 一份新的空工作流：只有一个「起点」节点（所有工作流都从它开始）。
fn fresh(kinds: &[Kind]) -> Workflow {
    let mut workflow = Workflow::new("未命名工作流");
    if let Some(read) = kinds.iter().find(|kind| kind.is_source) {
        workflow.nodes.push(NodeInstance {
            id: uuid::Uuid::new_v4().to_string(),
            kind: read.id.clone(),
            position: Position { x: 120.0, y: 180.0 },
            params: read.defaults.clone(),
        });
    }
    workflow
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 数据目录**不能改**。
    ///
    /// 应用标识符是长期稳定的，改了它会换一个数据目录，
    /// 用户之前保存的工作流会看起来凭空丢了。
    #[test]
    fn the_data_directory_is_stable() {
        assert!(data_root().ends_with(paths::IDENTIFIER));
        assert_eq!(paths::IDENTIFIER, "com.falsw.starrytools");
    }

    /// 新建的工作流自带一个「读取」节点 —— 所有工作流都从它开始。
    #[test]
    fn a_fresh_workflow_starts_with_an_input() {
        let kinds = crate::catalog::all();
        let workflow = fresh(&kinds);
        assert_eq!(workflow.nodes.len(), 1);
        assert_eq!(workflow.nodes[0].kind, "read");
        // 默认参数要从元数据里取，不能在这儿再拄一份。
        assert_eq!(
            workflow.nodes[0].params,
            kinds.iter().find(|k| k.id == "read").unwrap().defaults
        );
    }
}
