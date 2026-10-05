# 开发

## 环境与命令

需要 Rust 1.90+ 和一个能跑 egui/wgpu 的图形环境。没有 Node，也不依赖 WebView。

```sh
cargo run --release -p starrytools-app   # 跑起来
cargo test                               # 全部测试（core + app）
cargo clippy --all-targets               # 静态检查
```

`cargo test` 在**仓库根**跑，会跑 `core/` 和 `app/`。`core/` 不依赖任何 GUI 框架，
所以那部分测试不需要图形环境，哪里都能跑；`app/` 里也只有少量不需要窗口的测试
（比如中文字体回退能不能真的出汉字字形）。

**界面的观感与手感只能本地起 `cargo run --release -p starrytools-app` 看。**
用 release：debug 构建里图像处理慢几十倍。

## 数据放在哪

| 内容 | 位置 |
| --- | --- |
| 工作流（一个一份 JSON） | `<应用数据目录>/com.falsw.starrytools/workflows/` |
| 运行产物 | `<应用数据目录>/com.falsw.starrytools/outputs/<工作流名>/` |
| 下载的 ONNX 模型 | `<应用数据目录>/com.falsw.starrytools/models/` |
| 应用设置（帧率上限、显示 fps） | `<应用数据目录>/com.falsw.starrytools/settings.json` |

设置和工作流存档**分开放**：设置是「影响运行方式」的，不随工作流走，也不进存档。
模型的下载与推理见[背景移除](nodes/background_removal.md) —— 模型不带在应用里，用时才下。

应用数据目录在 Linux 上是 `~/.local/share/`，macOS 上是 `~/Library/Application Support/`，
Windows 上是 `%APPDATA%\`。界面上「工作流 → 打开存放目录」可以直接跳过去。

产物文件名是 `序号-节点名.扩展名`，每次完整运行会先清掉上一次的同名产物（只清 `NN-`
开头的那些），然后每个非起点节点的图像输出都会写一份到那里。跑单步时不清，否则会把
别的节点的产物抹掉。

「[保存到目录](nodes/save.md)」是另一回事：它把图像写到你自己挑的目录里，
不走这条自动产物的路子。

## 加一个新工具

一个工具就是 `core/src/nodes/` 下的一个文件，加上在 `registry.rs` 的
`builtin_specs()` 里登记一行。**界面不用改** —— 它只认 core 给的元数据。

```rust
// core/src/nodes/invert.rs
use std::sync::Arc;

use image::DynamicImage;

use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{one_output, NodeArgs, Value, ValueMap};
use crate::registry::NodeSpec;

pub const KIND: &str = "invert";

pub fn spec() -> NodeSpec {
    NodeSpec::fixed(
        NodeKind {
            id: KIND.into(),
            name: "反色".into(),
            category: "图像".into(),
            description: "把每个像素的颜色取反。".into(),
            is_source: false,
            inputs: vec![
                PortDef::new("image", "PNG 图像", PortType::Image(ImageFormat::Png)).required(),
            ],
            outputs: vec![PortDef::new("image", "PNG 图像", PortType::Image(ImageFormat::Png))],
            params: vec![ParamDef::new(
                "strength",
                "强度",
                ParamSpec::Slider {
                    default: 100.0, min: 0.0, max: 100.0,
                    step: 1.0, integer: true, unit: Some("%".into()),
                },
            )
            .described("0 是原样，100 是完全反过来。")],
            notes: vec!["展开卡片时列在这里的几句话。".into()],
        },
        run,
    )
}

fn run(args: &mut NodeArgs<'_>) -> Result<ValueMap, NodeError> {
    // `image_in` 取出图像和它带着的名字（克隆只复制一个 Arc）。
    let (source, name) = args.image_in("image")?;
    let drawn = source.decode()?;
    let amount = params::number(args.params, "strength", 100.0) / 100.0;

    let mut pixels = (*drawn).clone().into_rgba8();
    for pixel in pixels.pixels_mut() {
        for channel in 0..3 {
            let value = f64::from(pixel.0[channel]);
            pixel.0[channel] = (value + (255.0 - 2.0 * value) * amount).round() as u8;
        }
    }

    let value = ImageValue::from_image(
        ImageFormat::Png,
        Arc::new(DynamicImage::ImageRgba8(pixels)),
        EncodeOptions::default(),
    )?
    .inherit_provenance(&source);

    Ok(one_output("image", Value::Image(value).with_name_hint(name)))
}
```

然后在 `registry.rs` 的 `builtin_specs()` 里加一行 `nodes::invert::spec()`。

节点库会自己长出这个节点，参数按 `ParamSpec` 渲染成对应的控件
（数字 / 滑杆 / 文本 / 下拉 / 开关 / 文件 / 目录）。

几点约定：

- **改了像素的节点要带出处。** 先 `args.image_in("image")` 拿到 `(图像, 名字)`，处理完用
  `inherit_provenance` 接住来源文件名（下游「保存到目录」要用），再用 `with_name_hint(name)`
  把名字挂回去；最后 `one_output("image", ..)` 收尾。
- **参数可以条件显示。** `visible_when("mode", &["lossy"])` 是「或」，
  后面还可以 `.and_visible_when("lossyFormat", &["palette"])` 加一条「而且」。
- **输出端口类型依赖参数**时用 `NodeSpec::dynamic(kind, resolve_outputs, run)`
  （「读取」「图像格式转换」「图像压缩」就是这么做的），别在 `fixed` 里写死。
- **停下来问用户的节点**用 `NodeSpec::fixed(..).interactive(fn)` 声明，`fn(params) -> bool`
  说清哪些参数下它会拦住运行（画布画成紫色）。运行里用 `args.ask(InteractionKind::..)`
  发请求、阻塞等答复；没有界面时会返回 `NodeError` 而不是 panic。见
  [紫色节点](interface.md#紫色阻塞节点) 与 `core/src/interaction.rs`。
- **别在节点里 panic。** 出错就返回 `NodeError`，它会按节点归类写进运行报告。

### 节点 id 与老存档

`id` 会写进存档，**改它等于改文件格式** —— 老工作流认的就是这个字符串。非要改的话，
在 `registry.rs` 的 `LEGACY_KIND_IDS` 里加一条老 id 到新 id 的映射，
缺的参数由默认值补上，老存档照常打开和运行。

「图像格式转换」当年叫 `convert_to_png`，就是这么迁过来的；「缩放图像」则干脆没改 id
（只改了显示名）。注意**端口 id 改了是补不回来的** —— 存档里的连线认的是端口 id，
那种情况只能重新连一次。

## 代码结构

现在是两个 Cargo 包：`core/`（纯逻辑）和 `app/`（egui 界面），都在根工作区里。

```
Cargo.toml          根工作区（成员：core/、app/）
core/               纯逻辑，不依赖任何 GUI 框架
  src/model/        port_type / node_kind / value / params / workflow
                    （node_kind 里放着「参数 → 可选输入端口」的推导规则）
  src/nodes/        内置工具，一个文件一个（literal.rs = 文本/数字/布尔字面量）
                    read.rs = 读文件
  src/engine/       静态检查 + 执行引擎（含测试）
  src/png_opt/      无损 PNG 优化：颜色类型 / 位深 / 调色板 / 逐行 filter / zopfli / 元数据
  src/png_quant.rs  调色板量化（有损）
  src/registry.rs   工具注册表
  src/storage.rs    工作流的读写
  src/image_io.rs   图像值的编解码与缩略图
app/                egui 界面
  assets/           窗口图标（PNG / ICO）、桌面入口
  assets/icons/     vendored 的 lucide 原始 SVG（运行时解析，不手抄路径）
  src/main.rs       外壳：画布 + 浮动控件
  src/catalog.rs    读 core 的注册表 —— 界面认识节点的唯一途径
  src/canvas/       画布
    graph.rs        交互 + 绘制 + 刀光 + 端口连线 + 动画
    node.rs         数据模型：节点 / 连线 / 端口引用 + 小工具（id / 端口重算）
    layout.rs       尺寸与纵向排版：卡片多高、参数控件落在哪（绘制 / 命中 / 端口定位共用）
    view.rs         视口：平移 / 缩放 / 坐标换算
    geometry.rs     刀光的几何（贝塞尔采样 / 判交 / 切分），有测试
  src/ui/           外观
    theme.rs        设计令牌 + 中文字体回退
    widgets.rs      自绘按钮与浮层外壳（实心 / 幽灵 / 主色 / 危险）
    controls.rs     参数控件与颜色系统（数字 / 文本 / 下拉 / 开关 / 文件 / 取色器）
    icons.rs        图标：把 lucide SVG 光栅化成贴图
    svgpath.rs      SVG 路径解析
    chrome.rs       左上（节点库 / 名字 / 说明）与右上（问题 / 加载 / 保存 / 运行）
    library.rs      节点库浮层：分类 / 卡片 / 展开动画 / 拖出
    prompt.rs       紫色（阻塞）节点的交互浮层
    report.rs       底部状态药丸与向上弹出的运行记录
  src/state/        应用状态与服务
    workspace.rs    工作流的存取：数据目录 / 当前是哪一份 / 未保存标记
    settings.rs     帧率上限 / 是否显示 fps
    run.rs          静态检查的缓存 + 后台跑工作流
    models.rs       要下载的模型（背景移除）的下载状态
```

界面不依赖任何外部运行库：窗口走 `eframe`/`wgpu`，文件对话框用 `rfd`，
在文件管理器里定位用 `open`，数据目录用 `dirs` 自己拼 `com.falsw.starrytools`。

### 界面

- **两块工具栏**浮在画布上、不用胶囊外框：左上角是「节点库 / 工作流名 / 说明」，
  右上角是「N 处问题 / 加载 / 保存（底下一颗未保存小蓝点）/ 运行」。底部居中一颗
  状态药丸，点开向上弹出运行记录。左下角一竖条缩放控件。
- **一种节点样式**：所有节点都是「标题栏 + 端口列 + 参数区」的卡片。字面量节点
  （文本 / 数字 / 布尔）只是没有输入，其余完全一样。
- **参数端口**：能被上游喂的参数，会在参数名前多一颗**小圆点**和一个类型徽标。
  从同类节点接进来后，那一行的控件就变成它自己的**禁用态**（每种控件各画各的，
  见 `widgets` 一层），不是统一换成灰块。这是从 `NodeKind::inputs_for(params)`
  **推导**出来的，不是一个参数一个参数手写的 —— 加新节点、新参数都不用碰界面代码。
- **颜色、圆角、阴影、字号**都收在 `theme.rs` 一处。
- **图标**不引第三方图标库、也不打包位图字体：图标是 vendored 在 `app/assets/icons/` 的
  lucide 原始 SVG。`icons.rs` 把路径折成中心线，再按「到中心线的距离」光栅化成一张贴图
  （按实际显示尺寸算、贴到物理像素网格、缓存起来；底色用 tint 染）。圆头端点、圆角连接、
  抗锯齿都是这个距离场的自然结果，因此不受 epaint 固定像素羽化的影响，小尺寸也不会糊。
  想加 / 换图标，把对应的 `.svg` 丢进去、在 `icons.rs` 里加一行即可。
- **动画**：卡片悬停上浮 / 展开、浮层淡入淡出、连线被刀光扫到时搏动、断开时两截回缩、
  开关滑块位移、运行中的 spinner、未保存蓝点的渐显、节点库展开时盖住文字的翻转箭头。

画布上的操作：左键拖节点、拖空白平移、滚轮缩放；**从端口往外拖可以接线**
（一头输出一头输入、类型对得上、一个输入端口只接一条，未接上时线会呼吸发光）；
**右键点节点**开菜单，可以删除或复制；**右键按住滑动划一刀**，扫到的连线会变红搏动，
松手就切断并各自弹回去。也可以用 `Delete` 删节点、`Ctrl+C/V` 复制粘贴、`Ctrl+S` 保存。
刀光的几何在 `app/src/canvas/geometry.rs`，是不碰界面的纯函数，有测试盯着。

## 性能

egui 是每帧重绘的，所以「哪些活儿不该每帧做」值得记一笔：

- **静态检查按画布版本号缓存。** `graph.revision` 内容一变才加一（平移缩放不算），
  `run::Check` 只在版本号变了时才重跑，否则每帧都会去读「读取」节点选的文件头（真的磁盘 IO）。
- **运行放后台线程。** zopfli 那一档能把界面卡死，`run::Runner` 在单独线程上跑、
  外壳轮询结果。
- **只有真的在动才申请重绘。** 动画自己会 `request_repaint`，外壳只在「时间在走」的
  那几种情况（运行中、刀光未散）补一次。
- **缩略图用最近邻**，卡片上的预览是像素画，放大不该糊。

想看帧时间，Linux 上先确认没被软件光栅化：`glxinfo -B` 里的 renderer 应该是有名字的
GPU（如 `NVIDIA GeForce …`、`AMD …`），而不是 `llvmpipe` / `swrast`。

## 打包

应用是单个可执行文件，没有运行期资源要部署 —— 窗口图标是 `include_bytes!` 打进二进制的。

**Linux** —— 仓库根的 `Makefile` 里有 `install` / `uninstall`：

```sh
make install                      # 默认装到 ~/.local
make install PREFIX=/usr/local    # 或系统级
```

它把二进制放进 `<PREFIX>/bin`，把 `app/assets/starrytools.desktop` 放进
`<PREFIX>/share/applications`，把三张 PNG 按 hicolor 规范放进
`<PREFIX>/share/icons/hicolor/<尺寸>/apps/`。装好后应用菜单里就能看到。

**macOS / Windows** —— `app/Cargo.toml` 里写好了 `[package.metadata.bundle]`
（名字、标识符、图标、分类），配合 [`cargo-bundle`](https://github.com/burtonageo/cargo-bundle)：

```sh
cargo install cargo-bundle
cargo bundle --release -p starrytools-app
```

macOS 需要 `.icns`（用 `iconutil` 从 `app/assets/` 的 PNG 生成）；Windows 用现成的
`app/assets/icon.ico`。
