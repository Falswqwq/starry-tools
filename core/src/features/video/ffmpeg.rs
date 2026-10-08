//! 与外部 `ffmpeg` / `ffprobe` 打交道。
//!
//! 只用命令行，不依赖任何 crate：探测输入信息用 `ffprobe`，转码用 `ffmpeg`。
//! 输入输出都走临时文件 —— ffmpeg 对某些容器需要可回退的输入，管道不可靠。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use serde_json::Value as Json;

use crate::error::NodeError;

/// 这台机器上有可用的 ffmpeg 吗（结果缓存一次）。
pub fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    })
}

/// 输入视频的探测结果。
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_s: f64,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub bit_rate: u64,
    /// 总帧数（`nb_frames` 给不出时用时长 × 帧率估）。
    pub frames: Option<u64>,
}

impl Probe {
    pub fn resolution(&self) -> String {
        format!("{}×{}", self.width, self.height)
    }
}

/// 用 `ffprobe` 读输入的参数。
pub fn probe(path: &Path) -> Result<Probe, NodeError> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .map_err(|err| NodeError::new(format!("无法运行 ffprobe：{err}")))?;
    if !output.status.success() {
        return Err(NodeError::new("ffprobe 读不出这个文件的信息"));
    }
    let json: Json = serde_json::from_slice(&output.stdout)
        .map_err(|err| NodeError::new(format!("ffprobe 的输出解析失败：{err}")))?;

    let mut probe = Probe::default();
    if let Some(streams) = json.get("streams").and_then(Json::as_array) {
        for stream in streams {
            match stream.get("codec_type").and_then(Json::as_str) {
                Some("video") if probe.width == 0 => {
                    probe.width = stream.get("width").and_then(Json::as_u64).unwrap_or(0) as u32;
                    probe.height = stream.get("height").and_then(Json::as_u64).unwrap_or(0) as u32;
                    probe.fps = stream
                        .get("avg_frame_rate")
                        .and_then(Json::as_str)
                        .and_then(parse_rational)
                        .unwrap_or(0.0);
                    probe.video_codec = stream
                        .get("codec_name")
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_string();
                    probe.frames = stream
                        .get("nb_frames")
                        .and_then(Json::as_str)
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|count| *count > 0);
                    if let Some(duration) = stream
                        .get("duration")
                        .and_then(Json::as_str)
                        .and_then(|value| value.parse::<f64>().ok())
                    {
                        probe.duration_s = duration;
                    }
                }
                Some("audio") if probe.audio_codec.is_none() => {
                    probe.audio_codec = stream
                        .get("codec_name")
                        .and_then(Json::as_str)
                        .map(str::to_string);
                }
                _ => {}
            }
        }
    }
    if let Some(format) = json.get("format") {
        if probe.duration_s == 0.0 {
            if let Some(duration) = format
                .get("duration")
                .and_then(Json::as_str)
                .and_then(|value| value.parse::<f64>().ok())
            {
                probe.duration_s = duration;
            }
        }
        probe.bit_rate = format
            .get("bit_rate")
            .and_then(Json::as_str)
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
    }

    if probe.width == 0 || probe.height == 0 {
        return Err(NodeError::new("这个文件里没有找到视频流"));
    }
    // 有些容器不给 `nb_frames`，用时长×帧率估一个。
    if probe.frames.is_none() && probe.duration_s > 0.0 && probe.fps > 0.0 {
        probe.frames = Some((probe.duration_s * probe.fps).round() as u64);
    }
    Ok(probe)
}

fn parse_rational(text: &str) -> Option<f64> {
    let (num, den) = text.split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    (den != 0.0).then(|| num / den)
}

/// ffmpeg `-progress` 报的一帧进度。
pub struct FrameReport {
    pub frame: u64,
    pub fps: f64,
    /// 已经编码到的输出时间（秒）。
    pub out_time_s: f64,
    /// ffmpeg 给的速度串，比如 `"2.4x"`。
    pub speed: String,
}

/// 跑一次 ffmpeg。参数里不含输入输出的路径，这两个由本函数补上。
///
/// 带上 `-progress pipe:1`，逐行解析 ffmpeg 报的帧进度，每来一段就回调一次
/// `on_frame` —— 界面靠它画进度条与帧计数。
pub fn encode(
    input: &Path,
    output: &Path,
    args: &[String],
    mut on_frame: impl FnMut(FrameReport),
) -> Result<(), NodeError> {
    let mut child = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-nostats"])
        .args(["-progress", "pipe:1"])
        .arg("-i")
        .arg(input)
        .args(args)
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| NodeError::new(format!("无法运行 ffmpeg：{err}")))?;

    // stderr 在另一个线程上收，免得两边管道互相堵死。
    let stderr = child.stderr.take().expect("stderr 是 piped 的");
    let drain = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut text);
        text
    });

    let stdout = child.stdout.take().expect("stdout 是 piped 的");
    let mut report = FrameReport {
        frame: 0,
        fps: 0.0,
        out_time_s: 0.0,
        speed: String::new(),
    };
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key {
            "frame" => report.frame = value.parse().unwrap_or(report.frame),
            "fps" => report.fps = value.parse().unwrap_or(report.fps),
            "speed" => report.speed = value.to_string(),
            // ffmpeg 的 `out_time_ms` 名字骗人 —— 它其实是微秒，和 `out_time_us` 一样。
            "out_time_us" | "out_time_ms" => {
                report.out_time_s = value.parse::<f64>().unwrap_or(0.0) / 1_000_000.0;
            }
            "progress" => on_frame(FrameReport {
                frame: report.frame,
                fps: report.fps,
                out_time_s: report.out_time_s,
                speed: report.speed.clone(),
            }),
            _ => {}
        }
    }

    let status = child
        .wait()
        .map_err(|err| NodeError::new(format!("等待 ffmpeg 失败：{err}")))?;
    let stderr = drain.join().unwrap_or_default();
    if !status.success() {
        let tail: String = stderr
            .lines()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(NodeError::new(format!("ffmpeg 转码失败：\n{tail}")));
    }
    Ok(())
}

/// 给「自动」模式用：粗略判断输入是不是偏「动画 / 像素风」。
///
/// 做法是从视频里抽几帧缩到很小，统计**相邻像素相同的比例**（平坦度）。
/// 赛璐璐动画 / 像素画大片平色，平坦度显著更高；实拍画面则低。返回 `None` 表示
/// 采样失败 —— 那时按实拍处理。
pub fn animation_likeness(path: &Path) -> Option<f64> {
    const SIZE: usize = 48;
    const MAX_FRAMES: usize = 4;

    let output = Command::new("ffmpeg")
        .args(["-v", "quiet", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("fps=2,scale={SIZE}:{SIZE}:flags=area"),
            "-frames:v",
            &MAX_FRAMES.to_string(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let frame_len = SIZE * SIZE * 3;
    let frames = output.stdout.len() / frame_len;
    if frames == 0 {
        return None;
    }

    let mut flat_total = 0u64;
    let mut pairs_total = 0u64;
    for frame in output.stdout.chunks_exact(frame_len) {
        for y in 0..SIZE {
            for x in 1..SIZE {
                let left = (y * SIZE + x - 1) * 3;
                let here = (y * SIZE + x) * 3;
                pairs_total += 1;
                if frame[left..left + 3] == frame[here..here + 3] {
                    flat_total += 1;
                }
            }
        }
    }
    if pairs_total == 0 {
        return None;
    }
    Some(flat_total as f64 / pairs_total as f64)
}

/// 把一段字节写进一个带扩展名的临时文件，返回路径。
pub fn temp_file(extension: &str, bytes: &[u8]) -> Result<PathBuf, NodeError> {
    let name = format!(
        "starrytools-{}-{}.{}",
        std::process::id(),
        uuid::Uuid::new_v4(),
        extension
    );
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, bytes)
        .map_err(|err| NodeError::new(format!("无法写入临时文件：{err}")))?;
    Ok(path)
}
