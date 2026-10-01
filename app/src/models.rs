//! 要下载的模型（ONNX）在界面这一层的状态。
//!
//! 模型不带在应用里 —— 节点拖出来时本地没有，就整个禁用、卡片上挂一个下载面板。
//! 面板里能挑**下载源**（原始源 / 镜像源），点一下就在后台线程下到
//! `<数据目录>/models/`；下完节点自动恢复。
//!
//! 节点的「缺不缺模型」由 [`crate::catalog::Kind::model_missing`] 看磁盘上有没有文件
//! 来判断（每帧都看一次 —— 就是一次 `stat`，节点没几个）。这里只管**正在下**的那些。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use starrytools_core::bg_model::{self, Model};

/// 后台线程推回来的一件事。
enum Event {
    /// 已下多少字节 / 总共多少字节（不知道就是 `None`）。
    Progress(u64, Option<u64>),
    /// 下完了：`Ok` 收尾成功、`Err` 失败（带一句原因）。
    Done(Result<(), String>),
}

/// 一个正在进行的下载。
pub struct Download {
    /// 已经下了多少字节。
    pub received: u64,
    /// 总共多少字节（服务器没给就是 `None`）。
    pub total: Option<u64>,
    /// 失败原因（`None` = 还在下 / 一直没失败）。
    pub failed: Option<String>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Event>,
}

impl Download {
    /// 进度：0–1。不知道总量时返回 `None`（画成不确定进度）。
    pub fn fraction(&self) -> Option<f32> {
        self.total
            .filter(|total| *total > 0)
            .map(|total| (self.received as f32 / total as f32).clamp(0.0, 1.0))
    }
}

/// 画布上所有节点的下载状态。
///
/// - `tasks`：正在下的（键是**节点 id**）；
/// - `sources`：每个节点选的下载源下标（与任务分开存 —— 还没点下载时也要记住选的是哪个源）。
#[derive(Default)]
pub struct Downloads {
    tasks: HashMap<String, Download>,
    sources: HashMap<String, usize>,
}

impl Downloads {
    /// 某个节点是不是正下着。
    pub fn busy(&self, node_id: &str) -> bool {
        self.tasks.contains_key(node_id)
    }

    /// 还有下载在进行吗 —— 有的话界面要持续重绘，进度条才会动。
    pub fn any(&self) -> bool {
        !self.tasks.is_empty()
    }

    /// 某个节点当前的下载任务。
    pub fn get(&self, node_id: &str) -> Option<&Download> {
        self.tasks.get(node_id)
    }

    /// 某个节点选中的下载源下标（没选过就是 0）。
    pub fn source(&self, node_id: &str) -> usize {
        self.sources.get(node_id).copied().unwrap_or(0)
    }

    /// 记住某个节点选的下载源。
    pub fn set_source(&mut self, node_id: &str, source: usize) {
        self.sources.insert(node_id.to_string(), source);
    }

    /// 节点没了：把它留下的状态一并清掉。
    pub fn forget(&mut self, node_id: &str) {
        self.cancel(node_id);
        self.sources.remove(node_id);
    }

    /// 开一个下载。已经在下了就什么都不做（防止重复点）。
    pub fn start(&mut self, node_id: &str, model: &'static Model, source: usize) {
        if self.tasks.contains_key(node_id) || model.sources.is_empty() {
            return;
        }
        let source = source.min(model.sources.len() - 1);
        let url = model.sources[source].url;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        let worker_cancel = cancel.clone();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = bg_model::download(model, url, &worker_cancel, &move |received, total| {
                let _ = progress_tx.send(Event::Progress(received, total));
            });
            let _ = tx.send(Event::Done(result.map(|_| ())));
        });

        self.tasks.insert(
            node_id.to_string(),
            Download {
                received: 0,
                total: None,
                failed: None,
                cancel,
                rx,
            },
        );
        self.sources.insert(node_id.to_string(), source);
    }

    /// 取消一个下载。立刻把这项拿掉（节点马上变回「可下载」），后台线程收到旗标后自己收拾。
    pub fn cancel(&mut self, node_id: &str) {
        if let Some(task) = self.tasks.remove(node_id) {
            task.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// 收一次后台的进度。下完了就把这一项收掉 —— 下一帧节点会因为磁盘上有了文件而恢复。
    /// 失败则把原因留下来，让卡片显示「重试」。
    pub fn poll(&mut self) {
        let mut done: Vec<String> = Vec::new();
        for (id, task) in &mut self.tasks {
            loop {
                match task.rx.try_recv() {
                    Ok(Event::Progress(received, total)) => {
                        task.received = received;
                        if total.is_some() {
                            task.total = total;
                        }
                    }
                    Ok(Event::Done(Ok(()))) => {
                        done.push(id.clone());
                        break;
                    }
                    Ok(Event::Done(Err(err))) => {
                        task.failed = Some(err);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if task.failed.is_none() {
                            task.failed = Some("下载线程异常退出".to_string());
                        }
                        break;
                    }
                }
            }
        }
        for id in done {
            self.tasks.remove(&id);
        }
    }
}
