//! 静态检查与运行，以及这两件事在画布上留下的痕迹。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use starrytools_core::engine::{self, NodeStatus, ResolvedWorkflow, RunReport, Severity};
use starrytools_core::model::workflow::Workflow;

/// 画布上要额外画出来的东西：哪些连线有问题、哪些节点跑过、各花了多久。
#[derive(Default)]
pub struct Marks {
    /// 有问题的连线（按连线 id）。
    pub invalid_edges: HashSet<String>,
    /// 跑失败 / 跑成功的节点（按节点 id）。
    pub failed_nodes: HashSet<String>,
    pub ok_nodes: HashSet<String>,
    /// 跑过的节点用了多少毫秒（卡片右上角那句「完成 12ms」）。
    pub node_ms: std::collections::HashMap<String, u64>,
}

impl Marks {
    pub fn build(resolved: &ResolvedWorkflow, report: Option<&RunReport>) -> Self {
        let mut marks = Marks::default();

        for issue in &resolved.issues {
            if !matches!(issue.severity, Severity::Error) {
                continue;
            }
            if let Some(edge) = &issue.edge_id {
                marks.invalid_edges.insert(edge.clone());
            }
        }

        if let Some(report) = report {
            for node in &report.nodes {
                marks.node_ms.insert(node.node_id.clone(), node.elapsed_ms);
                match node.status {
                    NodeStatus::Ok => {
                        marks.ok_nodes.insert(node.node_id.clone());
                    }
                    NodeStatus::Failed => {
                        marks.failed_nodes.insert(node.node_id.clone());
                    }
                    NodeStatus::Skipped => {}
                }
            }
        }

        marks
    }
}

/// 静态检查的结果，按画布版本号缓存。
///
/// **不能每帧跑**：`resolve` 会去读输入节点选中的文件（要读文件头才知道是什么格式），
/// 那是真的磁盘 IO。
pub struct Check {
    revision: u64,
    resolved: ResolvedWorkflow,
}

impl Default for Check {
    fn default() -> Self {
        Self {
            // 故意给一个不可能的版本号，第一次取就一定重算。
            revision: u64::MAX,
            resolved: ResolvedWorkflow {
                nodes: Vec::new(),
                issues: Vec::new(),
                runnable: false,
            },
        }
    }
}

impl Check {
    pub fn get(&mut self, revision: u64, workflow: &Workflow) -> &ResolvedWorkflow {
        if self.revision != revision {
            self.resolved = engine::resolve(workflow);
            self.revision = revision;
        }
        &self.resolved
    }

    pub fn errors(&self) -> usize {
        self.resolved
            .issues
            .iter()
            .filter(|issue| matches!(issue.severity, Severity::Error))
            .count()
    }
}

/// 跑一次工作流。
///
/// `run` 是 CPU 活（无损 PNG 优化会挨个试颜色类型 / 位深 / filter / zopfli），
/// 放在界面线程上会把窗口卡死，所以丢到后台线程，每帧来收一次结果。
#[derive(Default)]
pub struct Runner {
    job: Option<Job>,
    report: Option<RunReport>,
    error: Option<String>,
}

struct Job {
    rx: Receiver<Result<RunReport, starrytools_core::error::AppError>>,
}

impl Runner {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn report(&self) -> Option<&RunReport> {
        self.report.as_ref()
    }

    /// 清掉上一次的运行结果（运行记录上的「清空」）。
    pub fn clear(&mut self) {
        self.report = None;
        self.error = None;
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 开跑。已经在跑就直接忽略 —— 上一次的结果还没回来。
    pub fn start(&mut self, workflow: Workflow, only_node: Option<String>, output_root: PathBuf) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = engine::run(&workflow, only_node.as_deref(), &output_root);
            // 收端已经走掉了就算了，不需要特别处理。
            let _ = tx.send(result);
        });

        self.report = None;
        self.error = None;
        self.job = Some(Job { rx });
    }

    /// 来收一次结果，不阻塞。
    pub fn poll(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        match job.rx.try_recv() {
            Ok(Ok(report)) => {
                self.report = Some(report);
                self.job = None;
            }
            Ok(Err(err)) => {
                self.error = Some(err.to_string());
                self.job = None;
            }
            Err(TryRecvError::Empty) => {}
            // 线程没了却没送结果 —— 只可能是它 panic 了。
            Err(TryRecvError::Disconnected) => {
                self.error = Some("运行线程异常退出".to_string());
                self.job = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starrytools_core::engine::Issue;

    #[test]
    fn only_errors_mark_edges_as_invalid() {
        let resolved = ResolvedWorkflow {
            nodes: Vec::new(),
            runnable: false,
            issues: vec![
                Issue {
                    severity: Severity::Error,
                    message: "类型对不上".into(),
                    node_id: None,
                    port_id: None,
                    edge_id: Some("e1".into()),
                },
                Issue {
                    severity: Severity::Warning,
                    message: "只是提醒".into(),
                    node_id: None,
                    port_id: None,
                    edge_id: Some("e2".into()),
                },
            ],
        };

        let marks = Marks::build(&resolved, None);
        assert!(marks.invalid_edges.contains("e1"));
        assert!(!marks.invalid_edges.contains("e2"), "警告不该把连线画红");
    }
}
