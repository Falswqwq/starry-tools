//! 画布上的数据模型：节点、连线、端口引用，以及几个围绕它们的小工具。
//!
//! 这些是纯粹的**数据**与构造 / 维护函数，不含画布状态、也不含绘制 —— 单独放一处，
//! `graph` 那边只剩交互与绘制。

use eframe::egui::Pos2;
use starrytools_core::engine::NodeStatus;
use starrytools_core::model::params::Params;
use starrytools_core::progress::NodeStep;

use crate::canvas::geometry::Cubic;
use crate::catalog::{Kind, Port};

#[derive(Clone)]
pub(crate) struct Node {
    /// 节点实例 id，落盘时用它（存档里的 `source` / `target` 认的就是它）。
    pub id: String,
    pub pos: Pos2,
    /// 节点类型 id。参数声明按这个 id 到注册表里查。
    pub kind: String,
    pub title: String,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    /// 这个节点实例自己的参数值。
    pub params: Params,
    /// 跑完之后有没有缩略图可看。有的活卡片底下多留一条预览区（**不落盘**，是视图状态）。
    pub preview: bool,
    /// 跑出来是个色板（「色彩分析」）时，卡片底下摆一排小色块。**不落盘**。
    pub palette: Option<Vec<[u8; 4]>>,
    /// 上一次运行的结果（状态 / 耗时 / 警告 / 错误 / 产物）。**不落盘**。
    pub run: Option<NodeRun>,
    /// 运行**进行中**的实时进度（视频压缩这类长任务才有）。**不落盘**。
    pub step: Option<NodeStep>,
    /// 当前「缺模型 / 缺外部程序」吗。这两项都要碰磁盘 / 探一次工具，缓存下来，
    /// 免得每帧被反复问（见 [`refresh_ports`]）。
    pub requirements: Requirements,
}

/// 一个节点当前缺不缺「跑起来必需的外部依赖」。
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Requirements {
    /// 要一个得下载的模型，而本地还没有。
    pub model_missing: bool,
    /// 要一个外部程序（如 `ffmpeg`），而本机没有。
    pub tool_missing: bool,
}

/// 一个节点上一次运行时留下的痕迹，画在卡片上。
#[derive(Clone)]
pub struct NodeRun {
    pub status: NodeStatus,
    pub ms: u64,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// 写下去的产物路径（有的话卡片底部会出现一行操作）。
    pub file: Option<String>,
}

#[derive(Clone)]
pub struct Wire {
    pub from: usize,
    /// 起点端口 id（**不是下标**）。端口会随参数增减，下标靠不住。
    pub from_port: String,
    pub to: usize,
    pub to_port: String,
    /// 稳定 id，创建时算一次。落盘、静态检查、绘制都认它。
    ///
    /// 存下来是为了不在**每帧**为每条连线重新 `format!` 一遍 —— 节点 id 一旦定下
    /// 就不会变，这个串也不会变。
    pub id: String,
}

/// 卡片右上角工具按钮的边长（屏幕像素，随缩放走）。
/// 卡片右上角工具按钮的边长（屏幕像素）。
pub(crate) const TOOL_SIZE: f32 = 20.0;

/// 一个端口的位置引用。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PortRef {
    pub node: usize,
    pub port: usize,
    /// 是输入端口吗？否则是输出端口。
    pub input: bool,
}

/// 正在拉的那根线。
pub(crate) struct Connect {
    pub(crate) from: PortRef,
    /// 当前指针位置（屏幕坐标）。
    pub(crate) to: Pos2,
    /// 落点候选 —— 只在悬停到一个**合法**端口上时才有值。
    pub(crate) target: Option<PortRef>,
}

impl Wire {
    /// 用两端的节点 id 和端口 id 拼出稳定 id。落盘和静态检查的结果都认它 ——
    /// 两边必须是同一个拼法。
    pub fn make_id(from: &str, from_port: &str, to: &str, to_port: &str) -> String {
        format!("{from}:{from_port}->{to}:{to_port}")
    }
}

/// 一道刀光。
pub(crate) struct Slash {
    /// 屏幕坐标。刀口要始终锐利，所以不跟画布缩放位移。
    pub(crate) from: Pos2,
    pub(crate) to: Pos2,
    /// 这一刀会切到的连线（`wires` 的下标）与切点参数（曲线上 0..1），拖动过程中实时更新。
    pub(crate) doomed: Vec<(usize, f32)>,
    /// 收刀跟出的开始时刻。`None` 表示还按着。
    pub(crate) release: Option<f64>,
}

/// 正在断裂的连线。
///
/// 切下去的那一刻就把这条线从 `wires` 里摘掉，动画自己带着曲线走 ——
/// 这样就不会出现「动画还在放、下标已经挪位」的问题。
pub(crate) struct Severing {
    pub(crate) curve: Cubic,
    /// 断口在曲线上的参数。
    pub(crate) t: f32,
    pub(crate) started: f64,
}

/// 节点右键菜单里选中的动作。
#[derive(Clone, Copy)]
pub(crate) enum MenuAction {
    Delete(usize),
    Duplicate(usize),
}

/// 按端口 id 找下标。认不出来就退回第一个 —— 老存档里可能存着已经删掉的端口。
pub(crate) fn port_index(ports: &[Port], id: &str) -> usize {
    ports.iter().position(|port| port.id == id).unwrap_or(0)
}

/// 新的节点实例 id。
pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 按当前参数重算一个节点的端口。
///
/// 参数一改就调 —— 端口类型可能跟着参数走（「输入」选的文件、「图像格式转换」的目标格式、
/// 「图像压缩」的模式）。
pub(crate) fn refresh_ports(node: &mut Node) {
    let (inputs, outputs) = crate::catalog::ports_for(&node.kind, &node.params);
    node.inputs = inputs;
    node.outputs = outputs;
    node.requirements = crate::catalog::requirements(&node.kind, &node.params);
}

/// 按 id 找节点类型。
///
/// 故意写成不挂在 `self` 上的自由函数：这样「读类型声明」和「改节点参数」
/// 是两个互不相干的借用，不用为了避开借用检查去克隆参数。
pub(crate) fn kind_of<'a>(kinds: &'a [Kind], node: &Node) -> Option<&'a Kind> {
    kinds.iter().find(|kind| kind.id == node.kind)
}
