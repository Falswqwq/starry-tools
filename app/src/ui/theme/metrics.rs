//! 控件与表面的统一尺寸（逻辑像素，画布自行乘缩放）。

use eframe::egui::{self, Rect};

pub const R_CARD: u8 = 8;
pub const R_CTL: u8 = 6;
pub const CONTROL_H: f32 = 24.0;
pub const CONTROL_PAD_X: f32 = 8.0;
pub const CONTROL_PAD_Y: f32 = 3.0;
pub const PORTS_ROW_H: f32 = CONTROL_H;
pub const PORTS_GAP: f32 = 4.0;
pub const PORTS_ADD_H: f32 = CONTROL_H;
pub const BUTTON_H: f32 = 29.0;
pub const BUTTON_SM_H: f32 = 25.0;
pub const ICON_SM_H: f32 = 21.0;
pub const BUTTON_PAD_X: f32 = 11.0;
pub const BUTTON_SM_PAD_X: f32 = 9.0;
pub const CARD_SHADOW_BLUR: f32 = 3.0;
pub const RAISED_SHADOW_BLUR: f32 = 14.0;
pub const POP_SHADOW_OFFSET: [i8; 2] = [0, 10];
pub const POP_SHADOW_BLUR: u8 = 28;
pub const SHADOW_OFFSET_Y: f32 = 1.0;
pub const SWITCH_TRACK_RATIO: f32 = 0.8;
pub const SWITCH_ASPECT: f32 = 1.8;
pub const SWITCH_KNOB_PAD_RATIO: f32 = 0.15;

pub fn control_inner(rect: Rect, zoom: f32) -> Rect {
    rect.shrink2(egui::vec2(CONTROL_PAD_X * zoom, CONTROL_PAD_Y * zoom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_control_metrics_leave_the_same_text_inset() {
        assert_eq!(CONTROL_H, PORTS_ROW_H);
        assert_eq!(CONTROL_H, PORTS_ADD_H);
        for zoom in [0.5, 1.0, 2.0] {
            let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(160.0, CONTROL_H) * zoom);
            let inner = control_inner(rect, zoom);
            assert_eq!(inner.left(), CONTROL_PAD_X * zoom);
            assert_eq!(inner.height(), (CONTROL_H - CONTROL_PAD_Y * 2.0) * zoom);
        }
    }
}
