//! 「阻塞节点」与界面之间的对话。
//!
//! 绝大多数节点是纯函数：拿到输入、读一遍参数、立刻产出。少数节点做不了这种承诺 ——
//! 它必须**停下来问用户**（例如「图像裁切 · 形状裁切」要先让人在图上框一块）。
//! 这类节点在画布上画成紫色，运行到它时会拦住整条链，等用户给个答案再往下走。
//!
//! 运行的工作线程在这些节点上阻塞：把一条 [`InteractionRequest`] 发给界面，
//! 等界面把 [`InteractionResponse`] 送回来再继续。执行本来就是**按拓扑序同步**的，
//! 所以「一个节点连着好几个紫色上游」天然成立 —— 轮到它的时候，那几个上游早就问完了。

use std::sync::mpsc::{Receiver, Sender};

use crate::error::NodeError;

/// 遮罩的形状。矩形就是矩形；椭圆内切于矩形，长宽相等时即正圆。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskShape {
    Rect,
    Ellipse,
}

/// 请求界面做的具体事情。
#[derive(Debug, Clone)]
pub enum InteractionKind {
    /// 在一张图上框出一块，用来裁切。
    CropShape(CropShapeRequest),
}

/// 「形状裁切」要界面提供的东西。
#[derive(Debug, Clone)]
pub struct CropShapeRequest {
    /// 原图尺寸（像素）。
    pub width: u32,
    pub height: u32,
    /// 预览图，`data:` URL，和运行报告里的缩略图同一种编码。
    pub preview: String,
    /// 遮罩形状由参数决定，界面只负责画。
    pub shape: MaskShape,
}

/// 用户做完之后送回运行线程的答案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionResponse {
    /// 裁切区域：像素坐标，相对原图，已经夹在图像范围内。
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// 用户放弃了这次操作。
    Cancel,
}

/// 界面收到的一条请求。
#[derive(Debug, Clone)]
pub struct InteractionRequest {
    /// 提出请求的节点实例 id —— 画布据此高亮它。
    pub node_id: String,
    pub node_name: String,
    pub kind: InteractionKind,
}

/// 运行线程这一端的两个端点：发请求、收答案。
///
/// 它被移进运行的工作线程；界面那边留着相对的
/// `Receiver<InteractionRequest>` 和 `Sender<InteractionResponse>`。
pub struct Interaction {
    requests: Sender<InteractionRequest>,
    responses: Receiver<InteractionResponse>,
}

impl Interaction {
    pub fn new(
        requests: Sender<InteractionRequest>,
        responses: Receiver<InteractionResponse>,
    ) -> Self {
        Self {
            requests,
            responses,
        }
    }

    /// 问一次界面，阻塞到用户给出答案为止。
    pub fn ask(
        &self,
        node_id: &str,
        node_name: &str,
        kind: InteractionKind,
    ) -> Result<InteractionResponse, NodeError> {
        self.requests
            .send(InteractionRequest {
                node_id: node_id.to_string(),
                node_name: node_name.to_string(),
                kind,
            })
            .map_err(|_| NodeError::new("界面已经关闭，无法继续这次交互"))?;
        self.responses
            .recv()
            .map_err(|_| NodeError::new("界面已经关闭，无法继续这次交互"))
    }
}
