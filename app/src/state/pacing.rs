//! 整帧 pacing：输入、egui 内部动画和自主重绘走同一个入口。
//!
//! eframe 0.36 的 native event_loop_builder 只配置构建阶段，App 没有逐帧的
//! WaitUntil/跳过 present 钩子；SurfaceConfig 只提供 vsync/队列延迟，并非任意 FPS。
//! 因而在 ui 开头有界 sleep。这是最多一帧的调度等待，不是磁盘/网络 IO，绝不忙等。
//! 软件 pacer 使用非 vsync present；等待预算扣除上一帧 UI/渲染/present 已花的时间，
//! 而不是在每帧末尾另加一个完整周期。deadline 按时间网格推进，避免调度误差累积。
//! 平台不支持非 vsync 时 wgpu 会回退；操作系统调度及显示器 present 的实际 FPS
//! 仍需在 Windows/Linux 窗口实测。

use std::time::{Duration, Instant};

#[derive(Default)]
pub struct FramePacer {
    next: Option<Instant>,
    period: Option<Duration>,
    frame: Option<u64>,
}

impl FramePacer {
    pub fn pace(&mut self, frame: u64, fps: f32) {
        // egui 的布局重试不是新的 present，不应再次等待。
        if self.frame == Some(frame) {
            return;
        }
        self.frame = Some(frame);
        let period = Duration::from_secs_f64(1.0 / f64::from(fps));
        if self.period != Some(period) {
            self.period = Some(period);
            self.next = None;
        }
        let now = Instant::now();
        let deadline = self.next.unwrap_or(now);
        let wait = wait_until(now, deadline, period);
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        self.next = Some(advance_deadline(deadline, Instant::now(), period));
    }
}

fn wait_until(now: Instant, deadline: Instant, period: Duration) -> Duration {
    deadline.saturating_duration_since(now).min(period)
}

fn advance_deadline(deadline: Instant, now: Instant, period: Duration) -> Instant {
    // 空闲、慢帧或窗口恢复后重新对齐，不补发积压帧。
    if now.saturating_duration_since(deadline) >= period {
        now + period
    } else {
        deadline + period
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_and_animation_share_a_bounded_deadline() {
        let t = Instant::now();
        let period = Duration::from_millis(20);
        assert_eq!(wait_until(t, t, period), Duration::ZERO);
        assert_eq!(wait_until(t, t + period, period), period);
        assert_eq!(wait_until(t, t + period * 10, period), period);
        assert_eq!(wait_until(t + period, t, period), Duration::ZERO);
    }

    #[test]
    fn rendering_and_present_time_are_subtracted_from_the_budget() {
        let start = Instant::now();
        let period = Duration::from_secs_f64(1.0 / 60.0);
        let deadline = start + period;
        let render = Duration::from_millis(4);
        assert_eq!(
            wait_until(start + render, deadline, period),
            period - render
        );
        // present 已经等了整帧，就不能再等 16.7ms，否则默认 60fps 会变成 30fps。
        assert_eq!(wait_until(start + period, deadline, period), Duration::ZERO);
        assert_eq!(
            advance_deadline(deadline, deadline, period),
            start + period * 2
        );
    }

    #[test]
    fn scheduler_jitter_does_not_accumulate_on_top_of_vsync() {
        let t = Instant::now();
        let period = Duration::from_secs_f64(1.0 / 60.0);
        let late = Duration::from_micros(300);
        let mut deadline = t;
        for _ in 0..120 {
            deadline = advance_deadline(deadline, deadline + late, period);
        }
        assert_eq!(deadline, t + period * 120);
    }

    #[test]
    fn slow_frames_and_resume_never_catch_up_in_a_burst() {
        let t = Instant::now();
        let period = Duration::from_millis(20);
        let now = t + Duration::from_secs(10);
        assert_eq!(advance_deadline(t, now, period), now + period);
    }

    #[test]
    fn changing_fps_resets_deadline_and_layout_retries_do_not_wait() {
        let mut pacer = FramePacer::default();
        pacer.pace(1, 60.0);
        let first = pacer.next;
        pacer.pace(1, 60.0);
        assert_eq!(pacer.next, first);
        pacer.pace(2, 30.0);
        assert_eq!(pacer.period, Some(Duration::from_secs_f64(1.0 / 30.0)));
    }
}
