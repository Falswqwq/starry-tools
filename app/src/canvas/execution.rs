//! 将实时/最终运行结果同步到节点，并管理预览解码和纹理生命周期。
use super::Graph;
use crate::canvas::node::NodeRun;
use crate::state::run::LiveRun;
use crate::ui::controls::color_rgba;
use eframe::egui;
use starrytools_core::engine::{NodeRunResult, RunReport};

impl Graph {
    /// 点过空白处之后，上一次运行的高亮该收起来了。外壳据此把 `Marks` 里那几个
    /// 「跑过了」的集合清掉。
    pub fn run_marks_hidden(&self) -> bool {
        self.run_marks_hidden
    }

    /// 把这次运行的缩略图传成纹理、并把运行痕迹贴到各节点上。
    ///
    /// 报告一换（`finished_at` 变了）就把旧纹理全丢掉重建 —— `TextureHandle`
    /// 一 drop，纹理由 egui 回收，不会越攒越多。
    pub(super) fn sync_previews(
        &mut self,
        ctx: &egui::Context,
        report: Option<&RunReport>,
        live: Option<&LiveRun>,
    ) {
        // 运行过程中：每跑完一个节点就把它的结果贴上卡片（状态 / 缩略图 / 色板 / 产物）——
        // 图片于是跟着流程一张张出现，而不是等到最后一并显示。
        if let Some(live) = live {
            for result in live.done.values() {
                self.apply_result(ctx, result);
            }
        }
        // 报告到了：再整体对一遗（幂等 —— 缩略图只传一次，其余值一样）。
        if let Some(report) = report {
            for result in &report.nodes {
                self.apply_result(ctx, result);
            }
        }
    }

    /// 把「正在跑的那个节点报的进度」贴到它自己的卡片上，其余节点清掉。
    ///
    /// 卡片底部的那段信息区靠它决定要不要长出来（见 [`crate::canvas::layout`]）。
    pub(super) fn sync_step(&mut self, live: Option<&LiveRun>) {
        let step = live.and_then(|live| live.step.clone());
        for node in &mut self.nodes {
            node.step = match &step {
                Some((id, step)) if *id == node.id => Some(step.clone()),
                _ => None,
            };
        }
    }

    /// 把一个节点的结果贴到它的卡片上：运行痕迹 + 色板 + 缩略图。
    ///
    /// **幂等**，所以运行中（实时结果）和跑完（正式报告）可以都调它一遍 ——
    /// 缩略图只传一次（`textures` 里有了就跳过）。
    pub(super) fn apply_result(&mut self, ctx: &egui::Context, result: &NodeRunResult) {
        let Some(index) = self.nodes.iter().position(|node| node.id == result.node_id) else {
            return;
        };

        let file = result.outputs.iter().find_map(|output| output.path.clone());
        // 输出是色板（hex 一行一个）的话，把颜色拆出来，卡片底下画一排小色块。
        let palette: Option<Vec<[u8; 4]>> = result
            .outputs
            .iter()
            .find_map(|output| output.palette.as_ref())
            .map(|colors| colors.iter().map(|color| color_rgba(color)).collect());

        // 幂等短路：内容没变就不重写 —— 否则每帧都要 clone 一遍运行文本。
        let node = &self.nodes[index];
        let unchanged = node.palette == palette
            && node.run.as_ref().is_some_and(|run| {
                run.status == result.status
                    && run.ms == result.elapsed_ms
                    && run.error == result.error
                    && run.warnings == result.warnings
                    && run.file == file
            });
        if !unchanged {
            self.nodes[index].palette = palette;
            self.nodes[index].run = Some(NodeRun {
                status: result.status,
                ms: result.elapsed_ms,
                error: result.error.clone(),
                warnings: result.warnings.clone(),
                file,
            });
        }

        // 缩略图一个节点只传一次。
        if self.textures.card.contains_key(&result.node_id) {
            return;
        }
        let Some(url) = result
            .outputs
            .iter()
            .find_map(|output| output.preview.as_deref())
        else {
            return;
        };
        if self
            .textures
            .failed
            .get(&result.node_id)
            .is_some_and(|failed| failed == url)
        {
            return;
        }
        let Some((width, height, rgba)) = starrytools_core::image_io::decode_preview_data_url(url)
        else {
            self.textures
                .failed
                .insert(result.node_id.clone(), url.to_owned());
            return;
        };
        self.textures.failed.remove(&result.node_id);
        let image =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
        let texture = ctx.load_texture(
            format!("preview:{}", result.node_id),
            image,
            egui::TextureOptions::NEAREST,
        );
        self.textures.card.insert(result.node_id.clone(), texture);
        self.nodes[index].preview = true;
    }

    /// 清掉上一次运行留在画布上的痕迹：缩略图 / 色板 / 状态 / 高亮。
    ///
    /// 开始新的一次运行、或用户清空运行记录时调。
    pub fn reset_run_marks(&mut self) {
        self.textures.card.clear();
        self.textures.failed.clear();
        for node in &mut self.nodes {
            node.preview = false;
            node.palette = None;
            node.run = None;
        }
        // 高亮亮回来（点过空白处那下作废）。
        self.run_marks_hidden = false;
    }
}
