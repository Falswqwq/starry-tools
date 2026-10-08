//! 静态检查与运行，以及这两件事在画布上留下的痕迹。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use starrytools_core::engine::{
    self, NodeRunResult, NodeStatus, ResolvedWorkflow, RunReport, Severity,
};
use starrytools_core::interaction::{Interaction, InteractionRequest, InteractionResponse};
use starrytools_core::model::workflow::Workflow;
use starrytools_core::progress::{NodeStep, Progress, ProgressEvent};

/// 画布上要额外画出来的东西：哪些连线有问题、哪些节点跑过、各花了多久。
#[derive(Default)]
pub struct Marks {
    /// 有问题的连线（按连线 id）。
    pub invalid_edges: HashSet<String>,
    /// 跑失败 / 跑成功的节点（按节点 id）。
    pub failed_nodes: HashSet<String>,
    pub ok_nodes: HashSet<String>,
    /// 跑过的节点用了多少毫秒（卡片右上角那句「完成 12ms」）。
    pub node_ms: HashMap<String, u64>,
    /// 正在等用户操作的那个紫色节点（画布把它高亮）。
    pub waiting: Option<String>,
    /// **正在跑**的那个节点（跑完就挪到下一个）—— 实时高亮用。
    pub running: Option<String>,
    /// 正在跑的那个节点报上来的进度（进度条 / 帧计数用）：`(节点 id, 进度)`。
    pub step: Option<(String, NodeStep)>,
}

impl Marks {
    /// 点过空白处之后，把「跑过了」的高亮收起来 —— 节点蓝色边框、蓝色连线都靠它。
    /// （失败的红色激光留着：那是出错提示，不是高亮。）
    pub fn dismiss_ok_highlight(&mut self) {
        self.ok_nodes.clear();
    }

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

/// 一次运行**进行中**的实时进度：跑到哪儿了、哪些已经跑完。
///
/// 报告要等整张图跑完才回来；在那之前，界面靠它把「当前节点」和**每个节点的结果**
/// （状态 / 耗时 / 缩略图 / 色板 / 产物）画出来 —— 卡片一张张亮起来。
#[derive(Default)]
pub struct LiveRun {
    /// 当前正在跑的节点 id。
    pub running: Option<String>,
    /// 已经跑完的节点 → 它的结果。
    pub done: HashMap<String, NodeRunResult>,
    /// 正在跑的那个节点报上来的实时进度：`(节点 id, 进度)`。
    pub step: Option<(String, NodeStep)>,
}

/// 跑一次工作流。
///
/// `run` 是 CPU 活（无损 PNG 优化会挨个试颜色类型 / 位深 / filter / zopfli），
/// 放在界面线程上会把窗口卡死，所以丢到后台线程，每帧来收一次结果。
///
/// 紫色节点会从这个后台线程发一条请求过来，并阻塞等着；界面每帧 `poll` 一下
/// 把它捞出来显示，用户动完手再 `respond` 把答案送回去。
#[derive(Default)]
pub struct Runner {
    job: Option<Job>,
    report: Option<RunReport>,
    error: Option<String>,
    /// 紫色节点发过来的请求通道（界面这端收）。
    requests: Option<Receiver<InteractionRequest>>,
    /// 把用户的答案送回后台线程。
    responses: Option<Sender<InteractionResponse>>,
    /// 当前等着用户回答的那条请求。
    pending: Option<InteractionRequest>,
    /// 引擎推过来的进度（界面这端收）。
    progress: Option<Receiver<ProgressEvent>>,
    /// 已经收到的进度：跑到哪儿了、哪些跑完了。`None` = 没在跑。
    live: Option<LiveRun>,
    /// 用户把运行记录清空了：画布该把上一次运行的痕迹也抹掉。
    reset_requested: bool,
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

    /// 正在等用户操作的那条请求（没有就是 `None`）。
    pub fn waiting(&self) -> Option<&InteractionRequest> {
        self.pending.as_ref()
    }

    /// 运行进行中的实时进度（没在跑就是 `None`）。
    pub fn live(&self) -> Option<&LiveRun> {
        self.live.as_ref()
    }

    /// 用户做完了，把答案送回后台线程。
    pub fn respond(&mut self, response: InteractionResponse) {
        if let Some(sender) = &self.responses {
            let _ = sender.send(response);
        }
        self.pending = None;
    }

    /// 清掉上一次的运行结果（运行记录上的「清空」）。
    pub fn clear(&mut self) {
        self.report = None;
        self.error = None;
        self.reset_requested = true;
    }

    /// 外壳读一下「该重置画布痕迹了」—— 读到就清零。
    pub fn take_reset(&mut self) -> bool {
        std::mem::take(&mut self.reset_requested)
    }

    /// 一次运行彻底结束（或开始新的一次）：把通道收干净。
    ///
    /// 进度也一并收掉 —— 接下来该由正式的报告（`report`）说话。
    fn finish(&mut self) {
        self.job = None;
        self.requests = None;
        self.responses = None;
        self.pending = None;
        self.progress = None;
        self.live = None;
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
        // 交互的两条线路：请求（后台→界面）、答复（界面→后台）。
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        let interaction = Interaction::new(request_tx, response_rx);
        // 进度：后台→界面，单方向。
        let (progress_tx, progress_rx) = mpsc::channel();
        let progress = Progress::new(progress_tx);
        std::thread::spawn(move || {
            let result = engine::run_with(
                &workflow,
                only_node.as_deref(),
                &output_root,
                Some(&interaction),
                Some(&progress),
            );
            // 收端已经走掉了就算了，不需要特别处理。
            let _ = tx.send(result);
        });

        self.report = None;
        self.error = None;
        self.job = Some(Job { rx });
        self.requests = Some(request_rx);
        self.responses = Some(response_tx);
        self.pending = None;
        self.progress = Some(progress_rx);
        self.live = Some(LiveRun::default());
    }

    /// 来收一次结果，不阻塞 —— 顺便看看紫色节点有没有发请求过来。
    pub fn poll(&mut self) {
        // 先把待办请求捞出来（正常运行一次只会有一条 —— 顺序执行）。
        let mut incoming: Option<InteractionRequest> = None;
        if let Some(requests) = &self.requests {
            while let Ok(request) = requests.try_recv() {
                incoming = Some(request);
            }
        }
        if incoming.is_some() {
            self.pending = incoming;
        }

        // 收一收进度：光标从上一个节点挪到下一个，跑完的记下来。
        let mut events: Vec<ProgressEvent> = Vec::new();
        if let Some(progress) = &self.progress {
            while let Ok(event) = progress.try_recv() {
                events.push(event);
            }
        }
        if !events.is_empty() {
            if let Some(live) = &mut self.live {
                for event in events {
                    match event {
                        ProgressEvent::Started { node_id } => {
                            live.running = Some(node_id);
                            live.step = None;
                        }
                        ProgressEvent::Step { node_id, step } => {
                            live.step = Some((node_id, step));
                        }
                        ProgressEvent::Finished { result } => {
                            if live.running.as_deref() == Some(result.node_id.as_str()) {
                                live.running = None;
                                live.step = None;
                            }
                            live.done.insert(result.node_id.clone(), result);
                        }
                    }
                }
            }
        }

        let Some(job) = &self.job else {
            return;
        };
        match job.rx.try_recv() {
            Ok(Ok(report)) => {
                self.report = Some(report);
                self.finish();
            }
            Ok(Err(err)) => {
                self.error = Some(err.to_string());
                self.finish();
            }
            Err(TryRecvError::Empty) => {}
            // 线程没了却没送结果 —— 只可能是它 panic 了。
            Err(TryRecvError::Disconnected) => {
                self.error = Some("运行线程异常退出".to_string());
                self.finish();
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
