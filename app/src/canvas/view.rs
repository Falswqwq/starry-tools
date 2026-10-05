//! 画布的视口：平移与缩放，以及流坐标 ↔ 屏幕坐标的换算。
//!
//! 这两件事是整体视图状态，和节点数据、交互瞬态分开 —— 单独一个结构，画布那边只
//! 管「把流坐标画到屏幕哪儿」。

use eframe::egui::{Pos2, Vec2};

/// 画布当前的平移与缩放。
#[derive(Clone, Copy)]
pub(crate) struct Viewport {
    /// 平移（屏幕像素）。
    pub pan: Vec2,
    /// 缩放倍数。
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Viewport {
    /// 流坐标 → 屏幕坐标。
    pub(crate) fn to_screen(self, p: Pos2) -> Pos2 {
        (p.to_vec2() * self.zoom + self.pan).to_pos2()
    }

    /// 屏幕坐标 → 流坐标。
    pub(crate) fn to_flow(self, p: Pos2) -> Pos2 {
        ((p.to_vec2() - self.pan) / self.zoom).to_pos2()
    }
}
