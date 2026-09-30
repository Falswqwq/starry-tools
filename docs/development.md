# 开发

## 环境与命令

需要 Rust 1.90+、Node 18+，以及 Tauri 在各平台的前置依赖
（Linux 上是 `webkit2gtk-4.1`、`libgtk-3-dev`、`libsoup-3.0-dev` 等）。

```sh
npm install
npm run tauri dev      # 开发
npm run tauri build    # 打包
```

测试：

```sh
npm test                        # 前端：类型检查 + 单元测试
cd src-tauri && cargo test      # 后端
cd src-tauri && cargo clippy --all-targets
```

`cargo test` 会直接编译 `dist/` 里的前端产物，所以**得先 `npm run build` 一次**；
`npm run tauri dev/build` 会自动处理这个顺序。改完前端产物想让后端重新嵌入，
`touch src-tauri/build.rs` 再 build。

后端测试不需要图形环境，沙箱里也能跑；**界面的观感与手感只能本地起 `npm run tauri dev` 看**。

## 数据放在哪

| 内容 | 位置 |
| --- | --- |
| 工作流（一个一份 JSON） | `<应用数据目录>/com.falsw.starrytools/workflows/` |
| 运行产物 | `<应用数据目录>/com.falsw.starrytools/outputs/<工作流名>/` |

应用数据目录在 Linux 上是 `~/.local/share/`，macOS 上是 `~/Library/Application Support/`，
Windows 上是 `%APPDATA%\`。界面上「工作流 → 打开存放目录」可以直接跳过去。

产物文件名是 `序号-节点名.扩展名`，每次完整运行会先清掉上一次的同名产物（只清 `NN-`
开头的那些），然后每个非起点节点的图像输出都会写一份到那里。跑单步时不清，否则会把
别的节点的产物抹掉。

「[保存到目录](nodes/save.md)」是另一回事：它把图像写到你自己挑的目录里，
不走这条自动产物的路子。

## 加一个新工具

一个工具就是 `src-tauri/src/nodes/` 下的一个文件，加上在 `registry.rs` 的
`builtin_specs()` 里登记一行。**前端不用改** —— 它只认后端给的元数据。

```rust
// src-tauri/src/nodes/invert.rs
use std::sync::Arc;

use image::DynamicImage;

use crate::error::NodeError;
use crate::image_io::{EncodeOptions, ImageValue};
use crate::model::node_kind::{NodeKind, ParamDef, ParamSpec, PortDef};
use crate::model::params;
use crate::model::port_type::{ImageFormat, PortType};
use crate::model::value::{NodeArgs, Value, ValueMap};
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
    // 克隆一个图像值只是复制一个 Arc，很便宜；这样后面才好拿 args 记提示。
    let source = args.image("image")?.clone();
    let name = args.input_name("image");
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

    let mut outputs = ValueMap::new();
    outputs.insert("image".to_string(), Value::Image(value).with_name_hint(name));
    Ok(outputs)
}
```

然后在 `registry.rs` 的 `builtin_specs()` 里加一行 `nodes::invert::spec()`。

节点库会自己长出这个节点，参数按 `ParamSpec` 渲染成对应的控件
（数字 / 滑杆 / 文本 / 下拉 / 开关 / 文件 / 目录）。

几点约定：

- **改了像素的节点要带出处。** `inherit_provenance` 把来源文件名接下去
  （下游「保存到目录」要用），`with_name_hint(args.input_name(..))` 把「重命名」挂的名字接下去。
- **参数可以条件显示。** `visible_when("mode", &["lossy"])` 是「或」，
  后面还可以 `.and_visible_when("lossyFormat", &["palette"])` 加一条「而且」。
- **输出端口类型依赖参数**时用 `NodeSpec::dynamic(kind, resolve_outputs, run)`
  （「输入」「图像格式转换」「图像压缩」就是这么做的），别在 `fixed` 里写死。
- **别在节点里 panic。** 出错就返回 `NodeError`，它会按节点归类写进运行报告。

### 节点 id 与老存档

`id` 会写进存档，**改它等于改文件格式** —— 老工作流认的就是这个字符串。非要改的话，
在 `registry.rs` 的 `LEGACY_KIND_IDS` 里加一条老 id 到新 id 的映射，
缺的参数由默认值补上，老存档照常打开和运行。

「图像格式转换」当年叫 `convert_to_png`，就是这么迁过来的；「缩放图像」则干脆没改 id
（只改了显示名）。注意**端口 id 改了是补不回来的** —— 存档里的连线认的是端口 id，
那种情况只能重新连一次。

## 代码结构

```
src-tauri/src/
  model/
    port_type.rs   类型系统：ImageFormat / PortType 与相容规则
    node_kind.rs   节点元数据：端口声明、参数声明、注意事项
    value.rs       沿连线流动的值（含「重命名」挂的名字）
    params.rs      参数的读写
    workflow.rs    落盘形状
  nodes/           内置工具，一个文件一个
  png_opt/         无损 PNG 优化：颜色类型 / 位深 / 调色板 / 逐行 filter / zopfli / 元数据
  png_quant.rs     调色板量化（有损）
  registry.rs      工具注册表
  engine/          静态检查 + 执行引擎（含测试）
  storage.rs       工作流的读写
  image_io.rs      图像值的编解码与缩略图
  commands.rs      暴露给前端的命令

src/
  ui/              组件层：Radix 原语 + 一份 token，按钮/输入框/下拉框/开关/滑杆/弹层都在这儿
  lib/             类型镜像、命令包装、端口配色、参数可见性
  state/store.ts   编辑器状态
  graph/           画布、节点卡片、连线
  panels/          浮动控件：左上、右上、节点库、工作流、状态药丸
  styles/app.css   全部样式（颜色与字号都是变量）
```

## 拖动那条热路径

画布上每拖一帧就是一轮「store 更新 → 所有订阅者重算选择器 → 重渲染」。节点一多，
这条路上的常数因子会被放大成肉眼可见的卡顿，所以有几条规矩别破：

- **不要在节点的选择器里遍历连线或问题列表。** 以前 `ToolNodeView` 是每个节点自己去
  `edges.filter(...)`，一次 store 更新就是 **O(节点数 × 边数)**。现在「连了哪些端口」
  由 `linkedPorts(edges)` 一次算好（按数组引用缓存），问题列表走一张按 `resolved` 缓存的
  索引 —— 选择器里只剩 O(1) 的查表。
- **改位置不要触发重新解析。** `nodeChangeEffect` 把节点改动分成「不用管 / 只标脏 /
  请后端重算」三档：位置和尺寸都不影响类型检查，所以拖动只会点亮未保存的蓝点。
  以前一停下来就重解析，回来后所有卡片跟着重渲染 —— 那一下是最明显的卡顿。
- **节点卡片是 `memo` 的，而且刻意不比位置。** React Flow 每帧都会给卡片塞新的
  `positionAbsoluteX/Y`，但移动是外层 `transform` 干的，卡片本身不看坐标；比了就会
  每帧重建一整棵 Radix 子树（见 `sameNodeViewProps`）。同理，卡片自己订阅的 store 切片
  变了照样重渲染，`memo` 只挡属性驱动的那些。
- **传给 `<ReactFlow>` 的对象要么是常量，要么引用稳定。** `defaultEdgeOptions` 这类写成
  行内字面量的话，每帧都是新对象，React Flow 会把本来没变的连线也推倒重来。
- **别开 `onlyRenderVisibleElements`。** 刀光是靠读 DOM 上真实的连线路径来判交的，
  虚拟化之后视口外的连线不在 DOM 里，就切不到了（见 `graph/slash.ts`）。

这几条的行为都有测试盯着（`src/lib/graph.test.ts`、`src/graph/ToolNodeView.test.tsx`）。

### 怎么量：渲染计数器

`src/lib/dev-render.ts` 里有一个随手可用的渲染计数器。在组件顶部调一次
`countRender('节点卡片')`，开发构建下控制台就会每秒打一张表，写着每个标记这一秒
渲染了多少次。已有的标记：`画布` / `节点卡片` / `参数控件` / `连线`。

| 读数 | 含义 |
| --- | --- |
| `节点卡片` ≈ 0～2 | 对 —— 拖动只动外层 `transform`，卡片不重渲染 |
| `节点卡片` ≈ 节点数 × 60 | `memo` 没生效，回去看 `sameNodeViewProps` |
| `连线` ≈ 连线数 × 60 | `memo` 没生效，或者传给 `<ReactFlow>` 的某个 prop 每帧换了引用 |
| `画布` ≈ 60 | 正常，它必须把新的 nodes 数组传下去 |
| 只在**松手后**爆一次 | 是 `resolved` 变了带动全场重渲染 |

数字不合常理时再往下查：WebKit 的开发者工具（dev 构建里右键 → 检查元素）录一段
Performance，Call Tree 按 Self Time 排；或者临时把 store 挂到 `window` 上，用普通
浏览器打开 `http://localhost:1420` 手动塞一堆节点，这样能用完整的 Chrome DevTools
和 React DevTools Profiler（`Canvas` 不依赖后端，所以空白后端也能拖）。

计数本身一直跑着（就是一次 Map 自增），**每秒那张表只在开发构建里打** —— 生产产物里
连那段代码一起被摇掉了（`grep 每秒渲染 dist/assets/*.js` 应该是空的）。
想静音：`starryRenders.watch = false`；想立刻看：`starryRenders.report()`。

最后一句提醒：想看**运行**（图像处理）的性能，得用 release 构建 —— `tauri dev` 编的是
debug 版 Rust，慢几十倍。

## 在浏览器里跑

画布本身不依赖后端（`Canvas` 无论 `ready` 与否都会渲染），所以可以把它拿到浏览器里跑，
用 Chrome / Firefox 那套更好用的绘制工具（Paint flashing、Layer borders、FPS meter）
来查「拖起来为什么卡」。

```sh
npm run dev            # 或者让 `npm run tauri dev` 开着 —— 它已经在 1420 端口上跑了
# 浏览器打开 http://localhost:1420
```

打开后画布在，但**是空的**：工作流、节点元数据、静态检查结果全来自 Tauri 的 IPC，
在浏览器里 `invoke` 会失败。开发构建里挂了一个 `window.starry` 专门用来搬：

```js
// 1. 在 Tauri 窗口的开发者工具里
starry.seed(80)         // 铺开 80 个节点（真实元数据 + 默认参数）
copy(starry.grab())     // 等静态检查跑完，把整张图拷进剪贴板

// 2. 在浏览器里
starry.load('粘进来')    // 画布和 Tauri 里一模一样
```

**带 `resolved` 是必须的**：卡片上端口那一行是它画出来的，少了它，搬过去的卡片会明显
比真的轻，拿来做绘制性能对比就不准了。`starry` 上还挂着 `store` 和 `api` 两个逃生口。

**结论要拿回 Tauri 里验。** 浏览器是 Blink / Gecko，Tauri 在 Linux 上是 WebKitGTK ——
同一份 DOM 在两个引擎里可能一个顺一个卡（合成策略不同）。浏览器里的价值是「用更好的
工具看清是哪块区域在重绘、有没有真的走上合成」，最终还得在真环境里确认。

这几个东西都不进生产构建（`main.tsx` 里被 `import.meta.env.DEV` 包着，
`grep 'starry.grab' dist/assets/*.js` 应该是空的）。

## Linux 上的硬件加速合成

Tauri 在 Linux 上用的是 WebKitGTK，而它的 `hardware-acceleration-policy` 默认是
**`OnDemand`** —— 只有页面确实需要（视频、3D、canvas 之类）时才走加速合成。我们这个
应用是「一大片 DOM 卡片 + 2D transform」的画布，可能就一直待在软件光栅化那条路上：
平移 / 拖动时整屏重新光栅化，而成本按屏幕像素面积算 —— 所以缩得越小越顺。

所以 `src-tauri/src/lib.rs` 里在 `Builder` **之前**设了一个 WebKit 自己的环境变量：

```rust
#[cfg(target_os = "linux")]
prefer_accelerated_compositing();   // => WEBKIT_FORCE_COMPOSITING_MODE=1
```

必须放在前面：WebKit 是在**构造页面**的时候读这些变量的，webview 建完之后再调
`set_hardware_acceleration_policy` 已经晚了。变量名是在这台机器的
`libwebkit2gtk-4.1.so.0` 里查出来的：

| 变量 | 作用 |
| --- | --- |
| `WEBKIT_FORCE_COMPOSITING_MODE` | 强制开加速合成 |
| `WEBKIT_DISABLE_COMPOSITING_MODE` | 强制关（当对照组用） |
| `WEBKIT_FORCE_DMABUF_RENDERER` / `WEBKIT_DISABLE_DMABUF_RENDERER` | 强制 / 绕过 DMABUF 那条渲染路径 |

环境里自己设过前两个之一的话，代码就不插手 —— 所以对照实验还做得成：

```sh
npm run tauri dev                                      # 现在默认就是强制开
WEBKIT_DISABLE_COMPOSITING_MODE=1 npm run tauri dev     # 强制关，看手感有没有变
```

两段手感一样的话，说明瓶颈不在合成，别再往这个方向走。

**前提**：这台机器得真有一个能用的 GPU。`glxinfo -B` 里的 renderer 如果是 `llvmpipe` /
`swrast`，那就是没有硬件可用，强制开也不会凭空变快 —— 那种情况下只能从「让卡片
少画点」入手（去模糊阴影、`contain: layout paint`、预览图去掉棋盘格）。

## 前端那边测什么

前端测的是**前后端之间那条缝**：`toWorkflow` 产出的键名必须是 Rust 那边认得的名字、
端口相容规则必须和后端的 `accepts` 一致、参数控件该带的交互类名（`nodrag` / `nowheel`）
不能少，以及几条只在屏幕上看得出来的样式规矩（见[配色与样式](theme.md)）。
实打实的图像处理逻辑都在 Rust 那边测。
