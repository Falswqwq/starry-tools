//! 应用级设置：左上角那颗齿轮里调的东西。
//!
//! 目前就两项 —— 整帧的帧率上限、右下角要不要显示 fps 计数器。都是「影响运行方式」
//! 而不是「工作流内容」，所以**单独存一个文件**（`<数据目录>/settings.json`），
//! 与工作流存档（`<数据目录>/workflows/`）完全分开。

use std::path::{Path, PathBuf};

/// 帧率上限的下界 / 上界（帧/秒）。
pub const FPS_MIN: f32 = 15.0;
pub const FPS_MAX: f32 = 240.0;

#[derive(Clone)]
pub struct Settings {
    /// 整帧的帧率上限（帧/秒）。`main` 的节流按它走。
    pub max_fps: f32,
    /// 右下角是否显示 fps 计数器。
    pub show_fps: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_fps: 60.0,
            show_fps: true,
        }
    }
}

impl Settings {
    /// 夹到合法范围，免得滑块以外的入口塞进 0 或负数把节流搞出除零。
    pub fn clamped_fps(&self) -> f32 {
        self.max_fps.clamp(FPS_MIN, FPS_MAX)
    }

    /// 从默认位置读一份；文件不在 / 坏了就用默认值，绝不拦住启动。
    pub fn load() -> Self {
        Self::load_from(&settings_path())
    }

    /// 写回默认位置。失败静默 —— 设置存不下不该影响用。
    pub fn save(&self) {
        self.save_to(&settings_path());
    }

    fn load_from(path: &Path) -> Self {
        let mut settings = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return settings;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return settings;
        };
        if let Some(fps) = value.get("maxFps").and_then(serde_json::Value::as_f64) {
            settings.max_fps = fps as f32;
        }
        if let Some(show) = value.get("showFps").and_then(serde_json::Value::as_bool) {
            settings.show_fps = show;
        }
        settings.max_fps = settings.clamped_fps();
        settings
    }

    fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let value = serde_json::json!({
            "maxFps": self.clamped_fps(),
            "showFps": self.show_fps,
        });
        if let Ok(text) = serde_json::to_string_pretty(&value) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// 设置文件的位置 —— 和数据目录同一个根，但单独一个文件。
fn settings_path() -> PathBuf {
    crate::state::workspace::data_root().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "starrytools-settings-{name}-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn settings_round_trip() {
        let path = temp_file("round-trip");
        let settings = Settings {
            max_fps: 144.0,
            show_fps: false,
        };
        settings.save_to(&path);
        let back = Settings::load_from(&path);
        assert_eq!(back.max_fps, 144.0);
        assert!(!back.show_fps);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_or_broken_file_falls_back_to_defaults() {
        let path = temp_file("missing");
        let back = Settings::load_from(&path);
        assert_eq!(back.max_fps, 60.0);
        assert!(back.show_fps);

        std::fs::write(&path, "这不是 JSON").unwrap();
        let back = Settings::load_from(&path);
        assert_eq!(back.max_fps, 60.0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_out_of_range_value_is_clamped_on_load() {
        let path = temp_file("clamp");
        std::fs::write(&path, r#"{"maxFps": 9999, "showFps": true}"#).unwrap();
        let back = Settings::load_from(&path);
        assert_eq!(back.max_fps, FPS_MAX);
        let _ = std::fs::remove_file(path);
    }

    /// 设置文件必须和工作流存档分开：不能落在 `workflows/` 目录里。
    #[test]
    fn settings_live_outside_the_workflow_directory() {
        let path = settings_path();
        assert_eq!(path.file_name().unwrap(), "settings.json");
        assert!(!path.starts_with(crate::state::workspace::data_root().join("workflows")));
    }
}
