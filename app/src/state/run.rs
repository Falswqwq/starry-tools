//! 静态检查与运行，以及这两件事在画布上留下的痕迹。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use starrytools_core::engine::{
    self, NodeRunResult, NodeStatus, ResolvedWorkflow, RunReport, Severity,
};
use starrytools_core::interaction::{Interaction, InteractionRequest, InteractionResponse};
use starrytools_core::model::workflow::Workflow;
use starrytools_core::progress::{NodeStep, Progress, ProgressEvent, PROGRESS_CAPACITY};

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

/// 编辑只覆盖槽中的最新快照；整个 Check 生命周期只有一个 resolve worker。
const CHECK_DEBOUNCE: Duration = Duration::from_millis(80);
type CheckKey = (u64, u64);
type CheckResult = (CheckKey, ResolvedWorkflow);

struct CheckRequest {
    key: CheckKey,
    workflow: Workflow,
    submitted: Instant,
}

#[derive(Default)]
struct CheckQueue {
    latest: Option<CheckRequest>,
    stopped: bool,
}

struct CheckWorker {
    queue: Arc<(Mutex<CheckQueue>, Condvar)>,
    rx: Receiver<CheckResult>,
}

impl CheckWorker {
    fn new(ctx: &eframe::egui::Context) -> Self {
        let queue = Arc::new((Mutex::new(CheckQueue::default()), Condvar::new()));
        let worker_queue = queue.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let (lock, wake) = &*worker_queue;
            loop {
                let request = {
                    let mut queue = lock.lock().expect("check queue");
                    loop {
                        if queue.stopped {
                            return;
                        }
                        if let Some(request) = &queue.latest {
                            let wait = debounce_wait(request.submitted, Instant::now());
                            if wait.is_zero() {
                                break;
                            }
                            queue = wake.wait_timeout(queue, wait).expect("check queue").0;
                        } else {
                            queue = wake.wait(queue).expect("check queue");
                        }
                    }
                    queue.latest.take().expect("latest check request")
                };
                // 文件头/工具探测在锁外执行；UI 可立即提交下一个版本。
                let resolved = std::panic::catch_unwind(|| engine::resolve(&request.workflow))
                    .unwrap_or_else(|_| check_failed());
                if tx.send((request.key, resolved)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        Self { queue, rx }
    }

    fn submit(&self, key: CheckKey, workflow: Workflow) {
        let (lock, wake) = &*self.queue;
        lock.lock().expect("check queue").latest = Some(CheckRequest {
            key,
            workflow,
            submitted: Instant::now(),
        });
        wake.notify_one();
    }

    fn clear(&self) {
        self.queue.0.lock().expect("check queue").latest = None;
        self.queue.1.notify_one();
    }
}

impl Drop for CheckWorker {
    fn drop(&mut self) {
        let mut queue = self.queue.0.lock().expect("check queue");
        queue.stopped = true;
        queue.latest = None;
        self.queue.1.notify_one();
        // 不 join：已经开始的文件/工具探测自行结束，不阻塞窗口关闭。
    }
}

fn debounce_wait(submitted: Instant, now: Instant) -> Duration {
    CHECK_DEBOUNCE.saturating_sub(now.saturating_duration_since(submitted))
}

pub struct Check {
    generation: u64,
    revision: Option<u64>,
    ready: bool,
    resolved: ResolvedWorkflow,
    worker: Option<CheckWorker>,
}

impl Default for Check {
    fn default() -> Self {
        Self {
            generation: 0,
            revision: None,
            ready: false,
            resolved: unchecked(),
            worker: None,
        }
    }
}

fn unchecked() -> ResolvedWorkflow {
    ResolvedWorkflow {
        nodes: Vec::new(),
        issues: Vec::new(),
        runnable: false,
    }
}

fn check_failed() -> ResolvedWorkflow {
    ResolvedWorkflow {
        nodes: Vec::new(),
        runnable: false,
        issues: vec![engine::Issue {
            severity: Severity::Error,
            message: "检查线程异常退出，请修改参数后重试".into(),
            node_id: None,
            port_id: None,
            edge_id: None,
        }],
    }
}

impl Check {
    /// 换图时版本号会重置，必须同时更新世代；旧任务完成后不能污染新图。
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.revision = None;
        self.ready = false;
        self.resolved = unchecked();
        if let Some(worker) = &self.worker {
            worker.clear();
        }
    }

    pub fn get(
        &mut self,
        revision: u64,
        workflow: &Workflow,
        ctx: &eframe::egui::Context,
    ) -> &ResolvedWorkflow {
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.ready = false;
            self.resolved = unchecked();
            let worker = self.worker.get_or_insert_with(|| CheckWorker::new(ctx));
            worker.submit((self.generation, revision), workflow.clone());
        }
        self.poll();
        &self.resolved
    }

    fn accept(&mut self, key: CheckKey, resolved: ResolvedWorkflow) {
        if Some(key) == self.revision.map(|rev| (self.generation, rev)) {
            self.resolved = resolved;
            self.ready = true;
        }
    }

    fn poll(&mut self) {
        loop {
            let Some(worker) = &self.worker else {
                return;
            };
            match worker.rx.try_recv() {
                Ok((key, resolved)) => self.accept(key, resolved),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.resolved = check_failed();
                    self.ready = true;
                    self.worker = None;
                    return;
                }
            }
        }
    }

    pub fn pending(&self) -> bool {
        !self.ready
    }

    pub fn runnable(&self, revision: u64) -> bool {
        self.ready && self.revision == Some(revision) && self.resolved.runnable
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
    request_serial: u64,
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
        // 同一请求只能回复一次；重复点击不能给下一条请求预存一份答案。
        if self.pending.take().is_some() {
            if let Some(sender) = &self.responses {
                let _ = sender.send(response);
            }
        }
    }

    pub fn request_serial(&self) -> u64 {
        self.request_serial
    }

    /// 换图/关闭时断开交互通道，唤醒等待答案的引擎；不宣称取消正在计算的节点。
    pub fn detach(&mut self) {
        self.finish();
        self.report = None;
        self.error = None;
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
    pub fn start(
        &mut self,
        workflow: Workflow,
        only_node: Option<String>,
        output_root: PathBuf,
        ctx: &eframe::egui::Context,
    ) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        // 交互的两条线路：请求（后台→界面）、答复（界面→后台）。
        let (request_tx, request_events) = mpsc::channel();
        let (request_forward, request_rx) = mpsc::channel();
        forward_with_wake(request_events, request_forward, ctx.clone());
        let (response_tx, response_rx) = mpsc::channel();
        let interaction = Interaction::new(request_tx, response_rx);
        // 进度：后台→界面，单方向。
        let (progress_tx, progress_events) = mpsc::sync_channel(PROGRESS_CAPACITY);
        let (progress_forward, progress_rx) = mpsc::sync_channel(PROGRESS_CAPACITY);
        forward_progress_with_wake(progress_events, progress_forward, ctx.clone());
        let progress = Progress::new(progress_tx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine::run_with(
                    &workflow,
                    only_node.as_deref(),
                    &output_root,
                    Some(&interaction),
                    Some(&progress),
                )
            }));
            if let Ok(result) = result {
                let _ = tx.send(result);
            }
            drop(tx);
            ctx.request_repaint();
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
        // 未回答的请求不能被下一条覆盖；其余请求保留在通道中，按 FIFO 逐条处理。
        if self.pending.is_none() {
            if let Some(requests) = &self.requests {
                if let Ok(request) = requests.try_recv() {
                    self.request_serial = self.request_serial.wrapping_add(1);
                    self.pending = Some(request);
                }
            }
        }

        // 收一收进度：光标从上一个节点挪到下一个，跑完的记下来。
        let mut events: Vec<ProgressEvent> = Vec::new();
        if let Some(progress) = &self.progress {
            for _ in 0..256 {
                let Ok(event) = progress.try_recv() else {
                    break;
                };
                events.push(event);
            }
            if events.len() == 256 {
                if let Some(ctx) = super::ui_context() {
                    ctx.request_repaint();
                }
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

impl Drop for Runner {
    fn drop(&mut self) {
        self.finish();
    }
}

/// core 只知道普通通道；转发线程在请求/进度到达时唤醒窗口，结束后随通道自然退出。
fn forward_with_wake<T: Send + 'static>(
    rx: Receiver<T>,
    tx: Sender<T>,
    ctx: eframe::egui::Context,
) {
    std::thread::spawn(move || {
        for event in rx {
            if tx.send(event).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
}

/// 两级有界进度均只由后台发送；UI 丢弃接收端会逐级解除背压。
fn forward_progress_with_wake(
    rx: Receiver<ProgressEvent>,
    tx: SyncSender<ProgressEvent>,
    ctx: eframe::egui::Context,
) {
    std::thread::spawn(move || {
        for event in rx {
            ctx.request_repaint();
            if tx.send(event).is_err() {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use starrytools_core::engine::Issue;

    #[test]
    fn stale_check_is_discarded_and_pending_cannot_run() {
        let mut check = Check {
            revision: Some(2),
            ..Default::default()
        };
        check.accept(
            (0, 1),
            ResolvedWorkflow {
                runnable: true,
                ..unchecked()
            },
        );
        assert!(check.pending());
        assert!(!check.runnable(2));
        assert!(!check.resolved.runnable);
    }

    #[test]
    fn workflow_switch_invalidates_even_an_identical_revision() {
        let mut check = Check {
            revision: Some(0),
            ..Default::default()
        };
        check.invalidate();
        check.revision = Some(0);
        check.accept(
            (0, 0),
            ResolvedWorkflow {
                runnable: true,
                ..unchecked()
            },
        );
        assert!(!check.runnable(0));
    }

    #[test]
    fn only_the_checked_revision_can_start() {
        let mut check = Check {
            revision: Some(7),
            ..Default::default()
        };
        check.accept(
            (0, 7),
            ResolvedWorkflow {
                runnable: true,
                ..unchecked()
            },
        );
        assert!(check.runnable(7));
        assert!(!check.runnable(8));
    }

    #[test]
    fn check_debounce_is_bounded_and_restarts_with_latest_edit() {
        let now = Instant::now();
        assert_eq!(debounce_wait(now, now), CHECK_DEBOUNCE);
        assert_eq!(debounce_wait(now, now + CHECK_DEBOUNCE), Duration::ZERO);
        assert_eq!(
            debounce_wait(now, now + Duration::from_secs(10)),
            Duration::ZERO
        );
        let edited = now + Duration::from_millis(20);
        assert_eq!(debounce_wait(edited, edited), CHECK_DEBOUNCE);
    }

    #[test]
    fn check_queue_keeps_only_latest_and_clears_on_drop() {
        let (_tx, rx) = mpsc::channel();
        let queue = Arc::new((Mutex::new(CheckQueue::default()), Condvar::new()));
        let worker = CheckWorker {
            queue: queue.clone(),
            rx,
        };
        worker.submit((0, 1), Workflow::new("first"));
        worker.submit((0, 2), Workflow::new("latest"));
        assert_eq!(queue.0.lock().unwrap().latest.as_ref().unwrap().key, (0, 2));
        worker.clear();
        assert!(queue.0.lock().unwrap().latest.is_none());
        worker.submit((1, 0), Workflow::new("new graph"));
        drop(worker);
        let queue = queue.0.lock().unwrap();
        assert!(queue.stopped);
        assert!(queue.latest.is_none());
    }

    #[test]
    fn revisions_and_workflow_switch_reuse_one_worker() {
        let mut check = Check::default();
        let ctx = eframe::egui::Context::default();
        let workflow = Workflow::new("empty");
        check.get(1, &workflow, &ctx);
        let queue = check.worker.as_ref().unwrap().queue.clone();
        check.get(2, &workflow, &ctx);
        check.invalidate();
        check.get(0, &workflow, &ctx);
        assert!(Arc::ptr_eq(&queue, &check.worker.as_ref().unwrap().queue));
        for _ in 0..3 {
            if !check.pending() {
                break;
            }
            let (key, resolved) = check
                .worker
                .as_ref()
                .unwrap()
                .rx
                .recv_timeout(Duration::from_secs(5))
                .expect("check worker result");
            check.accept(key, resolved);
            if !check.pending() {
                break;
            }
        }
        assert!(!check.pending());
        assert_eq!(check.revision, Some(0));
        assert_eq!(check.generation, 1);
    }

    #[test]
    fn disconnected_check_is_not_left_pending() {
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let mut check = Check {
            revision: Some(1),
            worker: Some(CheckWorker {
                queue: Arc::new((Mutex::new(CheckQueue::default()), Condvar::new())),
                rx,
            }),
            ..Default::default()
        };
        check.poll();
        assert!(!check.pending());
        assert!(!check.runnable(1));
        assert_eq!(check.errors(), 1);
    }

    fn request(id: &str) -> InteractionRequest {
        use starrytools_core::interaction::{CropShapeRequest, InteractionKind, MaskShape};
        InteractionRequest {
            node_id: id.into(),
            node_name: id.into(),
            kind: InteractionKind::CropShape(CropShapeRequest {
                width: 1,
                height: 1,
                preview: String::new(),
                shape: MaskShape::Rect,
            }),
        }
    }

    #[test]
    fn requests_are_fifo_and_never_overwrite_an_unanswered_one() {
        let mut runner = Runner::default();
        let (tx, rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        runner.requests = Some(rx);
        runner.responses = Some(response_tx);
        tx.send(request("first")).unwrap();
        tx.send(request("second")).unwrap();
        runner.poll();
        let serial = runner.request_serial();
        runner.poll();
        assert_eq!(runner.waiting().unwrap().node_id, "first");
        runner.respond(InteractionResponse::Cancel);
        runner.respond(InteractionResponse::Cancel);
        assert_eq!(response_rx.try_recv().unwrap(), InteractionResponse::Cancel);
        assert!(matches!(response_rx.try_recv(), Err(TryRecvError::Empty)));
        runner.poll();
        assert_eq!(runner.waiting().unwrap().node_id, "second");
        assert_ne!(runner.request_serial(), serial);
    }

    #[test]
    fn detach_and_drop_release_waiting_workers() {
        let (tx, rx) = mpsc::channel();
        let mut runner = Runner::default();
        runner.responses = Some(tx);
        runner.pending = Some(request("old"));
        runner.detach();
        assert!(runner.waiting().is_none());
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
        let (tx, rx) = mpsc::channel();
        runner.responses = Some(tx);
        drop(runner);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn disconnected_runner_clears_progress_and_releases_response() {
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let (response_tx, response_rx) = mpsc::channel();
        let mut runner = Runner::default();
        runner.job = Some(Job { rx });
        runner.live = Some(LiveRun {
            running: Some("panic".into()),
            ..LiveRun::default()
        });
        runner.responses = Some(response_tx);
        runner.pending = Some(request("panic"));
        runner.poll();
        assert!(!runner.is_running());
        assert!(runner.live().is_none());
        assert!(runner.waiting().is_none());
        assert!(runner.error().is_some());
        assert!(matches!(
            response_rx.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn bounded_progress_backlog_releases_both_senders_on_detach_and_drop() {
        for detach in [true, false] {
            let (tx, events) = mpsc::sync_channel(PROGRESS_CAPACITY);
            let (forward, rx) = mpsc::sync_channel(PROGRESS_CAPACITY);
            for i in 0..PROGRESS_CAPACITY {
                forward
                    .try_send(ProgressEvent::Started {
                        node_id: i.to_string(),
                    })
                    .unwrap();
                tx.try_send(ProgressEvent::Started {
                    node_id: i.to_string(),
                })
                .unwrap();
            }
            forward_progress_with_wake(events, forward, eframe::egui::Context::default());
            let deadline = Instant::now() + Duration::from_secs(5);
            // 转发线程最多从第一层取出一个事件，随后被第二层的容量限制阻塞。
            loop {
                match tx.try_send(ProgressEvent::Started {
                    node_id: "in-flight".into(),
                }) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Full(_)) => {
                        assert!(Instant::now() < deadline);
                        std::thread::yield_now();
                    }
                    Err(err) => panic!("unexpected disconnect: {err}"),
                }
            }
            assert!(matches!(
                tx.try_send(ProgressEvent::Started {
                    node_id: "overflow".into()
                }),
                Err(mpsc::TrySendError::Full(_))
            ));
            let (done, completed) = mpsc::channel();
            std::thread::spawn(move || {
                done.send(
                    tx.send(ProgressEvent::Started {
                        node_id: "blocked".into(),
                    })
                    .is_err(),
                )
                .unwrap();
            });
            let mut runner = Runner::default();
            runner.progress = Some(rx);
            if detach {
                runner.detach();
            } else {
                drop(runner);
            }
            assert!(completed.recv_timeout(Duration::from_secs(5)).unwrap());
        }
    }

    #[test]
    fn bounded_progress_forwarding_preserves_fifo() {
        let (tx, events) = mpsc::sync_channel(PROGRESS_CAPACITY);
        let (forward, rx) = mpsc::sync_channel(PROGRESS_CAPACITY);
        forward_progress_with_wake(events, forward, eframe::egui::Context::default());
        std::thread::spawn(move || {
            for i in 0..PROGRESS_CAPACITY * 3 {
                tx.send(ProgressEvent::Started {
                    node_id: i.to_string(),
                })
                .unwrap();
            }
        });
        for i in 0..PROGRESS_CAPACITY * 3 {
            let ProgressEvent::Started { node_id } =
                rx.recv_timeout(Duration::from_secs(5)).unwrap()
            else {
                panic!("started event");
            };
            assert_eq!(node_id, i.to_string());
        }
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn final_report_survives_progress_backpressure_and_polling() {
        let (tx, events) = mpsc::sync_channel(PROGRESS_CAPACITY);
        let (forward, rx) = mpsc::sync_channel(PROGRESS_CAPACITY);
        forward_progress_with_wake(events, forward, eframe::egui::Context::default());
        let (report_tx, report_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let progress = Progress::new(tx);
            for i in 0..PROGRESS_CAPACITY * 3 {
                progress.started(&i.to_string());
            }
            report_tx
                .send(Ok(RunReport {
                    ok: true,
                    duration_ms: 0,
                    order: vec!["complete".into()],
                    nodes: Vec::new(),
                    issues: Vec::new(),
                    output_dir: None,
                    finished_at: 42,
                }))
                .unwrap();
        });
        let mut runner = Runner::default();
        runner.job = Some(Job { rx: report_rx });
        runner.progress = Some(rx);
        runner.live = Some(LiveRun::default());
        let deadline = Instant::now() + Duration::from_secs(5);
        while runner.is_running() {
            runner.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(runner.error().is_none());
        assert!(runner.live().is_none());
        assert!(runner.progress.is_none());
        let report = runner.report().unwrap();
        assert!(report.ok);
        assert_eq!(report.order, vec!["complete"]);
        assert_eq!(report.finished_at, 42);
    }

    #[test]
    fn progress_drain_is_bounded_and_resumes_on_next_poll() {
        let (tx, rx) = mpsc::sync_channel(PROGRESS_CAPACITY);
        let mut runner = Runner::default();
        runner.progress = Some(rx);
        runner.live = Some(LiveRun::default());
        for i in 0..300 {
            tx.send(ProgressEvent::Started {
                node_id: i.to_string(),
            })
            .unwrap();
        }
        runner.poll();
        assert_eq!(runner.live().unwrap().running.as_deref(), Some("255"));
        runner.poll();
        assert_eq!(runner.live().unwrap().running.as_deref(), Some("299"));
    }

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
