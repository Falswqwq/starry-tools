//! 动画时长、缓动与浮层进出共用的节奏。

use eframe::egui;

pub const QUICK: f32 = 0.16;
pub const PANEL: f32 = 0.16;
pub const EXPAND: f32 = 0.20;
pub const OVERLAY: f32 = 0.24;
pub const EXIT: f64 = OVERLAY as f64;
pub const TOAST: f64 = 4.0;
pub const TOAST_FADE: f64 = 0.6;
pub const BREATH: f64 = 5.0;
pub const SPINNER_HZ: f32 = 1.2;
pub const PANEL_TRAVEL: f32 = 4.0;
pub const CANVAS_ENTER: f64 = 0.34;
pub const CANVAS_SEVER: f64 = 0.46;
pub const CANVAS_OVERTAKE: f64 = 0.28;

pub fn pulse(now: f64) -> f32 {
    0.35 + 0.65 * ((now * BREATH).sin() as f32 * 0.5 + 0.5)
}

pub fn animate(ctx: &egui::Context, id: egui::Id, on: bool, seconds: f32) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, on, seconds, ease_out_quad)
}

pub fn panel_offset(t: f32) -> egui::Vec2 {
    egui::vec2(0.0, (1.0 - t.clamp(0.0, 1.0)) * -PANEL_TRAVEL)
}

pub fn progress(now: f64, started: f64, seconds: f64) -> f32 {
    ((now - started) / seconds).clamp(0.0, 1.0) as f32
}

pub fn presence(now: f64, shown: f64, leaving: Option<f64>) -> f32 {
    let enter = ease_out_quad(progress(now, shown, PANEL as f64));
    enter.min(leaving.map_or(1.0, |at| 1.0 - ease_out_quad(progress(now, at, EXIT))))
}

pub fn toast_alpha(elapsed: f64) -> f32 {
    ((TOAST - elapsed) / TOAST_FADE).clamp(0.0, 1.0) as f32
}

pub fn ease_out_quad(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion_tokens_and_easing_are_consistent() {
        const { assert!(QUICK > 0.0 && QUICK <= EXPAND && EXPAND <= OVERLAY) };
        assert_eq!(EXIT, f64::from(OVERLAY));
        assert_eq!(
            (CANVAS_ENTER, CANVAS_SEVER, CANVAS_OVERTAKE),
            (0.34, 0.46, 0.28)
        );
        const { assert!(TOAST_FADE > 0.0 && TOAST_FADE < TOAST) };
        assert_eq!(panel_offset(1.0), egui::Vec2::ZERO);
        assert_eq!(panel_offset(0.0).y, -PANEL_TRAVEL);
        for ease in [ease_out_quad as fn(f32) -> f32, ease_out_cubic] {
            assert_eq!(ease(-1.0), 0.0);
            assert_eq!(ease(2.0), 1.0);
            for step in 0..100 {
                let t = step as f32 / 100.0;
                assert!(ease(t) <= ease(t + 0.01));
            }
        }
        assert!(presence(f64::from(PANEL) * 0.5, 0.0, None) > 0.5);
        for step in 0..100 {
            assert!((0.35..=1.0).contains(&pulse(step as f64 * 0.1)));
        }
    }

    #[test]
    fn presence_and_toast_are_bounded() {
        assert_eq!(presence(0.0, 0.0, None), 0.0);
        assert_eq!(presence(1.0, 0.0, None), 1.0);
        assert_eq!(presence(2.0, 0.0, Some(1.0)), 0.0);
        assert_eq!(toast_alpha(TOAST), 0.0);
        assert_eq!(toast_alpha(1.0), 1.0);
    }
}
