//! 「视频压缩」节点 —— 用外部 ffmpeg 把视频重新编码压小。
//!
//! 四个预设：
//!
//! * **平衡** —— 参数均衡，跑得快，兼容性最好（H.264）。
//! * **激进** —— 参考小丸工具箱的思路：慢 preset + 方差 AQ + 更高 CRF，压到尽可能小，
//!   同时肉眼尽量看不出损失。
//! * **自动** —— 先 `ffprobe` 探一遍输入（分辨率、帧率、时长），再抽几帧估一下
//!   是不是动画 / 像素风，据此自动挑编码器、CRF、preset，以及要不要降分辨率。
//! * **高级** —— 把各个参数摊开来手动调，默认值就是「平衡」那一套。

use std::path::Path;

use crate::error::NodeError;
use crate::features::video::ffmpeg;
use crate::features::video::VideoFormat;
use crate::media::{MediaValue, Meta};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef, SelectOption};
use crate::model::params::{self, Params};
use crate::model::port_type::PortType;
use crate::model::value::{human_size, one_output, NodeArgs, Value, ValueMap};
use crate::progress::NodeStep;
use crate::registry::NodeSpec;

pub const KIND: &str = "video_compress";

const PARAM_PRESET: &str = "preset";
const PARAM_CONTAINER: &str = "container";
const PARAM_CODEC: &str = "codec";
const PARAM_CRF: &str = "crf";
const PARAM_SPEED: &str = "speed";
const PARAM_AQ: &str = "aqMode";
const PARAM_AUDIO: &str = "audioBitrate";
const PARAM_MAX_HEIGHT: &str = "maxHeight";
const PARAM_KEEP_RESOLUTION: &str = "keepResolution";

const PRESET_BALANCED: &str = "balanced";
const PRESET_AGGRESSIVE: &str = "aggressive";
const PRESET_AUTO: &str = "auto";
const PRESET_ADVANCED: &str = "advanced";

pub fn spec() -> NodeSpec {
    NodeSpec::dynamic(
        NodeKind {
            id: KIND.into(),
            name: "视频压缩".into(),
            category: "视频".into(),
            description: "用 ffmpeg 把视频重新编码压小。输入收下常见的视频格式，\
                          输出容器由「输出格式」决定。"
                .into(),
            is_source: false,
            inputs: vec![
                PortDef::new("video", "视频", PortType::Video(VideoFormat::Any))
                    .required()
                    .hint("常见视频格式"),
            ],
            outputs: vec![PortDef::new(
                "video",
                "视频",
                PortType::Video(VideoFormat::Any),
            )],
            params: vec![
                ParamDef::new(
                    PARAM_PRESET,
                    "压缩预设",
                    ParamSpec::Select {
                        default: PRESET_BALANCED.into(),
                        options: vec![
                            SelectOption::new(PRESET_BALANCED, "平衡")
                                .hint("参数均衡，跑得快，H.264 兼容性最好"),
                            SelectOption::new(PRESET_AGGRESSIVE, "激进")
                                .hint("压到尽可能小，慢一些，肉眼尽量无损"),
                            SelectOption::new(PRESET_AUTO, "自动")
                                .hint("探测输入后自动选编码器与参数"),
                            SelectOption::new(PRESET_ADVANCED, "高级").hint("手动调各项参数"),
                        ],
                    },
                )
                .described("先从「平衡」或「自动」试起。"),
                ParamDef::new(
                    PARAM_CONTAINER,
                    "输出格式",
                    ParamSpec::Select {
                        default: VideoFormat::Mp4.name().into(),
                        options: VideoFormat::CONCRETE
                            .iter()
                            .map(|format| {
                                let option = SelectOption::new(format.name(), format.badge());
                                match format {
                                    VideoFormat::Mp4 => option.hint("通用，网页首选"),
                                    VideoFormat::Webm => option.hint("网页友好，多配 VP9"),
                                    VideoFormat::Mkv => option.hint("什么编码都装得下"),
                                    _ => option,
                                }
                            })
                            .collect(),
                    },
                )
                .described("输出端口的类型跟着它变。"),
                // —— 自动 ——
                ParamDef::new(
                    PARAM_KEEP_RESOLUTION,
                    "保留原始分辨率",
                    ParamSpec::Bool { default: true },
                )
                .described("关掉的话，高分辨率素材会自动降一档，省体积。")
                .visible_when(PARAM_PRESET, &[PRESET_AUTO]),
                // —— 高级 ——
                ParamDef::new(
                    PARAM_CODEC,
                    "编码器",
                    ParamSpec::Select {
                        default: "auto".into(),
                        options: vec![
                            SelectOption::new("auto", "自动").hint("按输出格式选"),
                            SelectOption::new("h264", "H.264").hint("兼容性最好"),
                            SelectOption::new("h265", "H.265").hint("同画质更小，慢"),
                            SelectOption::new("av1", "AV1").hint("压得最小，很慢"),
                            SelectOption::new("vp9", "VP9").hint("WebM 常用"),
                        ],
                    },
                )
                .visible_when(PARAM_PRESET, &[PRESET_ADVANCED]),
                ParamDef::new(
                    PARAM_CRF,
                    "质量（CRF）",
                    ParamSpec::Slider {
                        default: 23.0,
                        min: 0.0,
                        max: 51.0,
                        step: 1.0,
                        integer: true,
                        unit: None,
                    },
                )
                .described("越大越糊、体积越小。18≈视觉无损，23≈日常。")
                .visible_when(PARAM_PRESET, &[PRESET_ADVANCED]),
                ParamDef::new(
                    PARAM_SPEED,
                    "编码速度",
                    ParamSpec::Select {
                        default: "medium".into(),
                        options: vec![
                            SelectOption::new("ultrafast", "极快"),
                            SelectOption::new("fast", "快"),
                            SelectOption::new("medium", "中"),
                            SelectOption::new("slow", "慢"),
                            SelectOption::new("veryslow", "极慢").hint("最小体积"),
                        ],
                    },
                )
                .described("越慢压得越小。")
                .visible_when(PARAM_PRESET, &[PRESET_ADVANCED]),
                ParamDef::new(
                    PARAM_AQ,
                    "自适应量化（AQ）",
                    ParamSpec::Select {
                        default: "auto".into(),
                        options: vec![
                            SelectOption::new("auto", "自动"),
                            SelectOption::new("none", "关闭").hint("各区域一视同仁"),
                            SelectOption::new("variance", "方差").hint("平坦区域压得更狠"),
                        ],
                    },
                )
                .described("「方差」对动画 / 像素风这类大片平色的画面更友好。")
                .visible_when(PARAM_PRESET, &[PRESET_ADVANCED]),
                ParamDef::new(
                    PARAM_MAX_HEIGHT,
                    "分辨率上限",
                    ParamSpec::Select {
                        default: "keep".into(),
                        options: vec![
                            SelectOption::new("keep", "保持原始"),
                            SelectOption::new("2160", "2160p"),
                            SelectOption::new("1440", "1440p"),
                            SelectOption::new("1080", "1080p"),
                            SelectOption::new("720", "720p"),
                            SelectOption::new("480", "480p"),
                        ],
                    },
                )
                .described("超过这个高度就等比缩小，宽度自动。")
                .visible_when(PARAM_PRESET, &[PRESET_ADVANCED]),
                ParamDef::new(
                    PARAM_AUDIO,
                    "音频码率",
                    ParamSpec::Select {
                        default: "128".into(),
                        options: vec![
                            SelectOption::new("96", "96 kbps"),
                            SelectOption::new("128", "128 kbps"),
                            SelectOption::new("192", "192 kbps"),
                            SelectOption::new("256", "256 kbps"),
                        ],
                    },
                )
                .visible_when(PARAM_PRESET, &[PRESET_ADVANCED]),
            ],
            notes: vec![
                "这一步需要本机装有 `ffmpeg`。找不到时运行会直接报错，不会静默失败。".into(),
                "「激进」参考小丸工具箱的思路：慢 preset + 方差 AQ + 较高 CRF。".into(),
                "「自动」的动画判断是启发式的（抽样比平坦度），判错了手动切「高级」即可。".into(),
                "重编码总会掉一点画质 —— 想完全无损请用另一种思路（保留原编码，只改容器）。".into(),
            ],
        },
        output_ports,
        run,
    )
    .needs_tool("ffmpeg")
}

fn target_format(params: &Params) -> VideoFormat {
    let name = params::string(params, PARAM_CONTAINER, VideoFormat::Mp4.name());
    VideoFormat::from_name(&name).unwrap_or(VideoFormat::Mp4)
}

fn output_ports(params: &Params) -> Vec<PortDef> {
    let format = target_format(params);
    vec![PortDef::new(
        "video",
        format.label(),
        PortType::Video(format),
    )]
}

/// 编码器选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codec {
    H264,
    H265,
    Av1,
    Vp9,
}

impl Codec {
    fn encoder(self) -> &'static str {
        match self {
            Codec::H264 => "libx264",
            Codec::H265 => "libx265",
            Codec::Av1 => "libsvtav1",
            Codec::Vp9 => "libvpx-vp9",
        }
    }

    /// 默认 CRF（各编码器量纲不同，这里给经验值）。
    fn default_crf(self) -> u32 {
        match self {
            Codec::H264 => 23,
            Codec::H265 => 26,
            Codec::Av1 => 30,
            Codec::Vp9 => 31,
        }
    }
}

/// 一次编码的完整设置。
#[derive(Debug, Clone)]
struct Settings {
    codec: Codec,
    crf: u32,
    speed: String,
    aq: String,
    audio_kbps: u32,
    max_height: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            codec: Codec::H264,
            crf: 23,
            speed: "medium".into(),
            aq: "auto".into(),
            audio_kbps: 128,
            max_height: None,
        }
    }
}

impl Settings {
    /// 翻译成 ffmpeg 参数。
    fn to_args(&self, container: VideoFormat) -> Vec<String> {
        let mut args = vec![
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-c:v".to_string(),
            self.codec.encoder().to_string(),
        ];

        match self.codec {
            Codec::H264 | Codec::H265 => {
                args.push("-crf".into());
                args.push(self.crf.to_string());
                args.push("-preset".into());
                args.push(self.speed.clone());
                match self.aq.as_str() {
                    "none" => {
                        if self.codec == Codec::H264 {
                            args.extend(["-aq-mode".into(), "0".into()]);
                        } else {
                            args.extend(["-x265-params".into(), "aq-mode=0".into()]);
                        }
                    }
                    "variance" => {
                        if self.codec == Codec::H264 {
                            args.extend(["-aq-mode".into(), "3".into()]);
                        } else {
                            args.extend(["-x265-params".into(), "aq-mode=3".into()]);
                        }
                    }
                    _ => {}
                }
            }
            Codec::Av1 => {
                // libsvtav1 用数字 preset：0 最慢最好，13 最快。
                let preset = match self.speed.as_str() {
                    "ultrafast" => 12,
                    "fast" => 10,
                    "slow" => 5,
                    "veryslow" => 3,
                    _ => 7,
                };
                args.extend([
                    "-crf".into(),
                    self.crf.to_string(),
                    "-preset".into(),
                    preset.to_string(),
                ]);
            }
            Codec::Vp9 => {
                let cpu_used = match self.speed.as_str() {
                    "ultrafast" => 8,
                    "fast" => 6,
                    "slow" => 3,
                    "veryslow" => 1,
                    _ => 4,
                };
                args.extend([
                    "-crf".into(),
                    self.crf.to_string(),
                    "-b:v".into(),
                    "0".into(),
                    "-deadline".into(),
                    "good".into(),
                    "-cpu-used".into(),
                    cpu_used.to_string(),
                ]);
            }
        }

        if let Some(height) = self.max_height {
            args.extend(["-vf".into(), format!("scale=-2:{height}")]);
        }

        // 音频：WebM 用 Opus，其余用 AAC。
        args.push("-c:a".into());
        args.push(if container == VideoFormat::Webm {
            "libopus".into()
        } else {
            "aac".into()
        });
        args.push("-b:a".into());
        args.push(format!("{}k", self.audio_kbps));

        // MP4 / MOV 加 faststart，方便边下边播；H.265 标上 hvc1 提高播放器兼容性。
        if matches!(container, VideoFormat::Mp4 | VideoFormat::Mov) {
            args.extend(["-movflags".into(), "+faststart".into()]);
            if self.codec == Codec::H265 {
                args.extend(["-tag:v".into(), "hvc1".into()]);
            }
        }
        args
    }
}

/// 「平衡」那一套 —— 也是高级档的默认值。
fn balanced() -> Settings {
    Settings::default()
}

/// 「激进」：目标是把体积压到最小、肉眼尽量无损 —— 换更高效的编码器（H.265），
/// 配慢 preset + 方差 AQ + 较高 CRF。WebM 里放不下 H.265，就用 VP9。
///
/// 和「平衡」（H.264 CRF 23 medium）比，参数更狠，**结果一定更小**。
fn aggressive(container: VideoFormat) -> Settings {
    let codec = if container == VideoFormat::Webm {
        Codec::Vp9
    } else {
        Codec::H265
    };
    Settings {
        codec,
        crf: codec.default_crf(),
        speed: "slow".into(),
        aq: "variance".into(),
        ..Settings::default()
    }
}

/// 「自动」：探测输入后按经验挑一套。
fn auto(probe: &ffmpeg::Probe, keep_resolution: bool, path: &std::path::Path) -> Settings {
    let animation = ffmpeg::animation_likeness(path).is_some_and(|flat| flat > 0.62);

    // 分辨率越高，越值得上更高效的编码器（也更慢）。
    let codec = if probe.height > 1440 {
        Codec::Av1
    } else if probe.height > 1080 {
        Codec::H265
    } else {
        Codec::H264
    };

    // 动画 / 像素风大片平色，能承受更狠的量化。
    let crf = codec.default_crf() + if animation { 2 } else { 0 };

    let max_height = if keep_resolution || probe.height <= 1080 {
        None
    } else {
        Some(1080)
    };

    Settings {
        codec,
        crf,
        speed: if animation {
            "slow".into()
        } else {
            "medium".into()
        },
        aq: "variance".into(),
        audio_kbps: 128,
        max_height,
    }
}

fn advanced(params: &Params, container: VideoFormat) -> Settings {
    let codec = match params::string(params, PARAM_CODEC, "auto").as_str() {
        "h264" => Codec::H264,
        "h265" => Codec::H265,
        "av1" => Codec::Av1,
        "vp9" => Codec::Vp9,
        // 「自动」按容器选：WebM 走 VP9，其余走 H.264。
        _ => {
            if container == VideoFormat::Webm {
                Codec::Vp9
            } else {
                Codec::H264
            }
        }
    };
    let max_height = match params::string(params, PARAM_MAX_HEIGHT, "keep").as_str() {
        "2160" => Some(2160),
        "1440" => Some(1440),
        "1080" => Some(1080),
        "720" => Some(720),
        "480" => Some(480),
        _ => None,
    };
    Settings {
        codec,
        crf: params::integer(params, PARAM_CRF, codec.default_crf() as i64) as u32,
        speed: params::string(params, PARAM_SPEED, "medium"),
        aq: params::string(params, PARAM_AQ, "auto"),
        audio_kbps: params::integer(params, PARAM_AUDIO, 128).clamp(32, 512) as u32,
        max_height,
    }
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    if !ffmpeg::available() {
        return Err(NodeError::new(
            "没有找到 `ffmpeg`。装好它（`ffmpeg`、`ffprobe` 都在 PATH 里）再运行这个节点。",
        ));
    }

    let container = target_format(args.params);

    // 把输入产物取出来：字节、写临时文件的扩展名、来源。字节要复制出来，
    // 后面还要改 `args` 记提示，不能一直借着它。
    let (bytes, extension, origin) = {
        let value = args.input("video")?;
        let (bytes, extension) = value
            .product()
            .ok_or_else(|| NodeError::new("输入不是一个视频产物"))?;
        (
            bytes.to_vec(),
            extension,
            value.product_origin().map(Path::to_path_buf),
        )
    };
    let before = bytes.len();

    // ffmpeg 需要能回退的输入/输出文件，走临时文件。
    let input = ffmpeg::temp_file(extension, &bytes)?;
    let output = std::env::temp_dir().join(format!(
        "starrytools-{}-{}.{}",
        std::process::id(),
        uuid::Uuid::new_v4(),
        container.extension()
    ));

    // 先探一遍输入：拿总帧数（进度条要用），自动档还要靠它选参数。
    let probe = ffmpeg::probe(&input)?;

    let settings = match params::string(args.params, PARAM_PRESET, PRESET_BALANCED).as_str() {
        PRESET_AGGRESSIVE => aggressive(container),
        PRESET_AUTO => {
            let keep = params::boolean(args.params, PARAM_KEEP_RESOLUTION, true);
            let settings = auto(&probe, keep, &input);
            args.warn(format!(
                "自动：{}×{}@{} 的 {} → {}，CRF {}，preset {}{}",
                probe.width,
                probe.height,
                (probe.fps * 10.0).round() / 10.0,
                if probe.video_codec.is_empty() {
                    "未知编码"
                } else {
                    &probe.video_codec
                },
                settings.codec.encoder(),
                settings.crf,
                settings.speed,
                match settings.max_height {
                    Some(height) => format!("，缩到 {height}p"),
                    None => String::new(),
                }
            ));
            settings
        }
        PRESET_ADVANCED => advanced(args.params, container),
        _ => balanced(),
    };

    let ffmpeg_args = settings.to_args(container);

    // 进度：ffmpeg 每报一段帧进度，就换算成界面要的形状推过去。
    // `progress` / `node_id` 都是复制出来的引用，不借 `args`。
    let progress = args.progress;
    let node_id = args.node_id;
    let total = probe.frames;
    let duration = probe.duration_s;
    let result = ffmpeg::encode(&input, &output, &ffmpeg_args, |frame| {
        let Some(progress) = progress else { return };
        let fraction = match total {
            Some(total) if total > 0 => Some(frame.frame as f32 / total as f32),
            _ if duration > 0.0 => Some(frame.out_time_s as f32 / duration as f32),
            _ => None,
        };
        progress.step(
            node_id,
            NodeStep {
                fraction: fraction.map(|value| value.clamp(0.0, 1.0)),
                frame: Some(frame.frame),
                total_frames: total,
                text: (frame.fps > 0.0).then(|| format!("{:.0} 帧/s", frame.fps)),
            },
        );
    });

    // 临时文件用完就删，成败都删。
    let _ = std::fs::remove_file(&input);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&output);
        return Err(error);
    }

    let encoded = std::fs::read(&output)
        .map_err(|err| NodeError::new(format!("读不出 ffmpeg 的产物：{err}")))?;
    let out_probe = ffmpeg::probe(&output).ok();
    let _ = std::fs::remove_file(&output);

    // 大小变化与压缩率（压后 / 原始）。
    let after = encoded.len();
    let ratio = after as f64 / before.max(1) as f64;
    if after < before {
        args.warn(format!(
            "{} → {}（压到 {:.0}%）",
            human_size(before),
            human_size(after),
            ratio * 100.0
        ));
    } else {
        args.warn(format!(
            "压缩后反而变大了（{} → {}，{:.0}%）—— 输入可能已经压得很紧，换个预设或调小 CRF 再试",
            human_size(before),
            human_size(after),
            ratio * 100.0
        ));
    }

    let mut meta: Meta = out_probe
        .as_ref()
        .map(|probe| {
            vec![
                ("时长".into(), format!("{:.1}s", probe.duration_s)),
                ("分辨率".into(), probe.resolution()),
            ]
        })
        .unwrap_or_default();
    meta.push(("原大小".into(), human_size(before)));
    meta.push(("压缩率".into(), format!("{:.0}%", ratio * 100.0)));

    let value = MediaValue::from_bytes(container.name(), encoded)
        .with_meta(meta)
        .with_origin(origin);

    Ok(one_output("video", Value::Media(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::run;
    use crate::model::workflow::{Edge, NodeInstance, Position, Workflow};
    use crate::registry::registry;
    use std::path::{Path, PathBuf};

    fn workspace(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("starrytools-video-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 用 ffmpeg 自己造一个小视频（mp4 / H.264）。
    fn make_video(dir: &Path) -> PathBuf {
        let path = dir.join("clip.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=duration=2:size=320x240:rate=24",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success(), "造测试视频失败");
        path
    }

    fn defaults(kind: &str) -> Params {
        registry().get(kind).unwrap().kind.default_params()
    }

    fn node(id: &str, kind: &str, params: Params) -> NodeInstance {
        NodeInstance {
            id: id.into(),
            kind: kind.into(),
            position: Position::default(),
            params,
        }
    }

    /// 跑一次「读取 → 视频压缩」，返回产物的字节数。
    fn compressed_size(dir: &Path, video: &Path, preset: &str) -> u64 {
        let mut workflow = Workflow::new("视频压缩");
        let mut read_params = defaults(crate::nodes::read::KIND);
        read_params.insert("valueType".into(), serde_json::json!("video"));
        read_params.insert(
            "videoPath".into(),
            serde_json::json!(video.to_string_lossy()),
        );
        workflow
            .nodes
            .push(node("in", crate::nodes::read::KIND, read_params));
        let mut compress_params = defaults(KIND);
        compress_params.insert("preset".into(), serde_json::json!(preset));
        workflow.nodes.push(node("c", KIND, compress_params));
        workflow.edges.push(Edge {
            id: "e".into(),
            source: "in".into(),
            source_port: "out".into(),
            target: "c".into(),
            target_port: "video".into(),
        });

        let report = run(&workflow, None, &dir.join("out")).unwrap();
        assert!(report.ok, "{preset}: {:?}", report.nodes);
        let path = report.nodes[1].outputs[0].path.as_ref().expect("落盘");
        std::fs::metadata(path).unwrap().len()
    }

    /// 跑的时候会报帧进度 —— 界面靠它画进度条。
    #[test]
    fn reports_frame_progress() {
        if !ffmpeg::available() {
            eprintln!("跳过：本机没有 ffmpeg");
            return;
        }
        let dir = workspace("progress");
        let video = make_video(&dir);

        let mut workflow = Workflow::new("视频压缩");
        let mut read_params = defaults(crate::nodes::read::KIND);
        read_params.insert("valueType".into(), serde_json::json!("video"));
        read_params.insert(
            "videoPath".into(),
            serde_json::json!(video.to_string_lossy()),
        );
        workflow
            .nodes
            .push(node("in", crate::nodes::read::KIND, read_params));
        workflow.nodes.push(node("c", KIND, defaults(KIND)));
        workflow.edges.push(Edge {
            id: "e".into(),
            source: "in".into(),
            source_port: "out".into(),
            target: "c".into(),
            target_port: "video".into(),
        });

        let (tx, rx) = std::sync::mpsc::channel();
        let progress = crate::progress::Progress::new(tx);
        let report =
            crate::engine::run_with(&workflow, None, &dir.join("out"), None, Some(&progress))
                .unwrap();
        assert!(report.ok, "{:?}", report.nodes);

        let steps: Vec<NodeStep> = rx
            .try_iter()
            .filter_map(|event| match event {
                crate::progress::ProgressEvent::Step { step, .. } => Some(step),
                _ => None,
            })
            .collect();
        assert!(!steps.is_empty(), "视频压缩该报帧进度");
        let last = steps.last().unwrap();
        assert!(last.frame.is_some(), "该带上帧数");
        assert!(
            last.fraction.is_some_and(|value| value > 0.0),
            "总帧数该估得出来，进度大于 0"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 「激进」必须真的比「平衡」压得小 —— 换更高效的编码器（H.265）+ 更狠的参数。
    #[test]
    fn aggressive_beats_balanced_on_size() {
        if !ffmpeg::available() {
            eprintln!("跳过：本机没有 ffmpeg");
            return;
        }
        let dir = workspace("presets");
        let video = make_video(&dir);

        let balanced = compressed_size(&dir, &video, "balanced");
        let aggressive = compressed_size(&dir, &video, "aggressive");
        assert!(
            aggressive < balanced,
            "激进模式该更小：balanced={balanced} aggressive={aggressive}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真的把一个小视频压一遍：读取 → 视频压缩 → 落盘。
    #[test]
    fn compresses_a_real_video_end_to_end() {
        if !ffmpeg::available() {
            eprintln!("跳过：本机没有 ffmpeg");
            return;
        }
        let dir = workspace("compress");
        let video = make_video(&dir);

        let mut workflow = Workflow::new("视频压缩");
        let mut read_params = defaults(crate::nodes::read::KIND);
        read_params.insert("valueType".into(), serde_json::json!("video"));
        read_params.insert(
            "videoPath".into(),
            serde_json::json!(video.to_string_lossy()),
        );
        workflow
            .nodes
            .push(node("in", crate::nodes::read::KIND, read_params));
        workflow.nodes.push(node("c", KIND, defaults(KIND)));
        workflow.edges.push(Edge {
            id: "e".into(),
            source: "in".into(),
            source_port: "out".into(),
            target: "c".into(),
            target_port: "video".into(),
        });

        let report = run(&workflow, None, &dir.join("out")).unwrap();
        assert!(report.ok, "{:?}", report.nodes);
        let out = &report.nodes[1].outputs[0];
        assert_eq!(out.ty, PortType::Video(VideoFormat::Mp4));
        assert!(
            out.summary.contains("MP4"),
            "摘要该说明是什么：{}",
            out.summary
        );
        assert!(
            out.summary.contains("压缩率"),
            "摘要该带上压缩率：{}",
            out.summary
        );
        let path = out.path.as_ref().expect("产物应该落盘");
        let size = std::fs::metadata(path).unwrap().len();
        assert!(size > 0, "产物不该是空文件");
        // 大小变化会写进这个节点的提示里。
        assert!(
            report.nodes[1]
                .warnings
                .iter()
                .any(|warning| warning.contains('→')),
            "该报一声大小变化：{:?}",
            report.nodes[1].warnings
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
