//! 运行进度：跑到哪个节点了、每个节点产出了什么。
//!
//! 工作流是在后台线程上跑的，界面每帧来收一次。除了最后的报告，引擎还会在**每个节点
//! 开始 / 结束时**推一条进度过来 —— 界面据此把「正在跑的那个」高亮出来，并且**node
//! 一跑完就把它自己的结果（缩略图 / 色板 / 产物）贴上卡片**，而不是等到最后一并显示。
//!
//! 和 [`crate::interaction`] 一样，这也是一条单方向的通道 —— 界面只管收。

use std::sync::mpsc::Sender;

use crate::engine::NodeRunResult;

/// 引擎推给界面的一条进度。
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// 开始跑某个节点了。
    Started { node_id: String },
    /// 某个节点结束了（跑完 / 失败 / 跳过），带着它的完整结果。
    ///
    /// 当初只发「状态 + 耗时」；现在连 `NodeRunResult` 一起发，界面就能逐一贴上缩略图、
    /// 色板和产物 —— 卡片在运行过程中一张张亮起来，而不是等到最后。
    Finished { result: NodeRunResult },
}

/// 运行线程这一端：往界面推进度。
pub struct Progress {
    sender: Sender<ProgressEvent>,
}

impl Progress {
    pub fn new(sender: Sender<ProgressEvent>) -> Self {
        Self { sender }
    }

    /// 推一条。界面已经走掉了就算了，不需要特别处理。
    pub fn send(&self, event: ProgressEvent) {
        let _ = self.sender.send(event);
    }

    pub fn started(&self, node_id: &str) {
        self.send(ProgressEvent::Started {
            node_id: node_id.to_string(),
        });
    }

    /// 某个节点跑完了。把它的结果整份带过去。
    pub fn finished(&self, result: NodeRunResult) {
        self.send(ProgressEvent::Finished { result });
    }
}
