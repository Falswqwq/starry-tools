# 开发

## 环境与命令

推荐当前 stable Rust。最低版本以 Cargo 清单及锁定依赖为准：`core/Cargo.toml` 声明
`rust-version = "1.91"`，但当前 GUI 依赖 `eframe 0.36.2` 自身要求 **1.95**，完整工作区
不能仅凭 core 的下限选工具链。运行窗口还需要支持 egui/wgpu 的图形环境；不需要 Node / WebView。

```sh
cargo run --release -p starrytools-app
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
```

默认开启 `onnx` 与 `video`。视频运行需要 `ffmpeg` 和 `ffprobe` 都在 `PATH` 中，缺少时节点禁用，
无头调用返回 `NodeError`。ONNX 推理用纯 Rust 的 `tract-onnx`，模型按需下载，不随程序分发。

### Feature 组合

`app/Cargo.toml` 对 core 使用 `default-features = false`，由 app 的 `onnx` / `video` 分别转发
到同名 core feature，避免 app 关掉功能后又被依赖的默认值打开。

| 组合 | 工作区参数 | 保留的可选功能 |
| --- | --- | --- |
| none | `--no-default-features` | 无 |
| video | `--no-default-features --features video` | 视频类型、节点及工具探测 |
| onnx | `--no-default-features --features onnx` | 背景移除节点与推理 |
| all | `--all-features` | 两者都开（当前也等同默认配置） |

例如只验证视频组合（其余组合替换参数即可）：

```sh
cargo check --locked --workspace --all-targets --no-default-features --features video
cargo clippy --locked --workspace --all-targets --no-default-features --features video -- -D warnings
cargo test --locked --workspace --no-default-features --features video
```

只运行 app 时同样支持 `cargo run --release -p starrytools-app --no-default-features --features video`。
关闭 `onnx` 会同时移除背景移除节点的注册及 `rembg` 推理模块，而不只是隐藏界面入口。

### Linux 依赖

app 保留 eframe 默认的 X11 / Wayland 后端；`rfd 0.17` 默认使用 `xdg-portal` / `wayland`，
没有启用 `gtk3`，因此不需要为它安装 GTK 开发包。Ubuntu 构建环境：

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libwayland-dev libxkbcommon-dev libx11-dev libegl1-mesa-dev fonts-noto-cjk
# 需要跑真实视频测试时安装：
sudo apt-get install -y ffmpeg
```

`pkg-config` 与 `libwayland-dev` 尤其不能省：rfd 的 `wayland-backend/client_system` 会查找
系统 Wayland 库。上述命令负责构建 / 无窗口单测；实际运行还需要图形会话、驱动及可用的
xdg-desktop-portal 后端。Linux 数据目录遵循 `dirs` / XDG 设置。

### Windows 工具链与排错

推荐标准 MSVC 目标，先安装 Visual Studio Build Tools 的 C++ / Windows SDK 组件：

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc --component rustfmt --component clippy
cargo +stable-x86_64-pc-windows-msvc test --locked --workspace --all-features
cargo +stable-x86_64-pc-windows-msvc run --release -p starrytools-app
```

CI 的 `windows-latest` 使用 `x86_64-pc-windows-msvc`，不依赖本机 GNU 工具链的原生库。
若本地坚持使用 `x86_64-pc-windows-gnu`，链接器报 `cannot find -l...`（如 Windows import
library）通常是 MinGW 发行版 / 库搜索路径不完整，不应通过删依赖或忽略链接错误绕过。
确认 `gcc -dumpmachine` 是 `x86_64-w64-mingw32`，用 `gcc -print-search-dirs` 和
`gcc -print-file-name=libuserenv.a` 查实际库路径；只返回文件名说明没找到。

使用一套完整且架构一致的 MinGW-w64（如 MSYS2 的 MINGW64），不要混用 MSYS、UCRT64 与 MINGW64。
可在独立 PowerShell 会话中临时指定，路径按实际安装位置调整：

```powershell
$env:PATH = "C:\msys64\mingw64\bin;$env:PATH"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = 'C:\msys64\mingw64\bin\gcc.exe'
# 指向真实包含所缺 lib*.a 的目录，不是 bin；先用 gcc 输出确认。
$env:LIBRARY_PATH = 'C:\msys64\mingw64\lib;C:\msys64\mingw64\x86_64-w64-mingw32\lib'
cargo +stable-x86_64-pc-windows-gnu test --locked --workspace --all-features
```

不要把这些本机路径写进项目配置。app 的中文字体从 `WINDIR` / `SystemRoot` 下的 `Fonts`
查找微软雅黑、黑体、宋体；西文字体候选包含 Cascadia Mono / Consolas。视频子进程统一设置
`CREATE_NO_WINDOW`，release GUI 自身也不弹控制台。仍需在实际 Windows 桌面确认字体与子进程表现。

### 持续验证与边界

`.github/workflows/ci.yml` 在 push / pull request / 手动触发时运行：

- 一次 `cargo fmt --all -- --check`；
- Ubuntu 与 Windows MSVC × none / video / onnx / all：严格 clippy（`-D warnings`）及工作区测试；
- Linux 安装上述系统开发包与中文字体；video / all 两类任务在两平台安装并检查
  `ffmpeg` / `ffprobe`，使内置真实视频测试不会仅因缺工具而提前返回。安装或检查失败会令任务失败。

本机 Windows GNU（Rust 1.98.1）已完成四种 feature 组合的工作区测试：

| 组合 | 通过 | 忽略 |
| --- | ---: | ---: |
| none | 251 | 0 |
| video | 256 | 0 |
| onnx | 252 | 1 |
| all / 默认 | 257 | 1 |

忽略项是真实 ONNX 模型推理测试（未下载模型）。默认和 none 的严格 clippy、完整 rustfmt
检查通过；Windows 原生窗口已启动、确认中文与界面正常显示并正常关闭。

GitHub CI **尚未运行**。本机 WSL 缺少 cargo / pkg-config；Windows 上的 Linux 交叉检查也因
缺少 `x86_64-linux-gnu-gcc` 而停在 ring 构建阶段，不能宣称 Linux 构建或测试已通过。
CI 不下载 ONNX 模型，不验证真实模型推理、窗口交互、帧率或撕裂。现有中文字体测试在找不到
候选字体时会提前返回，也不能单凭绿色测试证明所有系统有汉字字形。

真实视频测试可单独复核（先确认两程序可用，保留 `--nocapture` 以看到缺工具时的跳过提示）：

```sh
ffmpeg -version
ffprobe -version
cargo test --locked -p starrytools-core --no-default-features --features video features::video::compress::tests -- --nocapture
```

## 数据放在哪

| 内容 | 位置 |
| --- | --- |
| 工作流（一个一份 JSON） | `<应用数据目录>/com.falsw.starrytools/workflows/` |
| 自动运行产物 | `<应用数据目录>/com.falsw.starrytools/outputs/<slug>--<encodedID>/` |
| 下载的 ONNX 模型 | `<应用数据目录>/com.falsw.starrytools/models/` |
| 应用设置（帧率上限、显示 fps） | `<应用数据目录>/com.falsw.starrytools/settings.json` |

应用数据目录通常是 Linux `~/.local/share/`、macOS `~/Library/Application Support/`、
Windows `%APPDATA%\`。设置与工作流存档分开；「工作流 → 打开存放目录」可定位。

自动产物由 `core/src/engine/artifacts.rs` 管理：目录名使用工作流名称的 slug 加上 ID 的
**完整 UTF-8 字节十六进制编码**，同名工作流不会互相清理。文件仍使用序号、节点名等可读命名。
每次完整运行只回收 `.starrytools-products.json` 中归属该工作流、大小与校验和均未改变的
直接普通文件。已被用户修改的文件、未登记文件、链接与目录不按旧产物删除；写新产物时不覆盖
已有文件，重名会换名。清单归属、格式或路径不合法时报错，不退回猜测删除。

**旧 `NN-` 文件、旧版仅按工作流名创建的输出目录都保留**，不会按前缀批量删除，也不会自动迁移。
单步「运行至此」不清理其它产物。「[保存到目录](nodes/save.md)」写的是用户指定目录，
与自动产物清单无关。

## 注册类型与工具

节点与类型共用一个启动期构建、随后冻结的注册快照，入口是 `core/src/registry.rs`：

| API | 作用 |
| --- | --- |
| `RegistryBuilder::with_builtins()` | 带上内置类型、类别、节点和当前 feature 的贡献项，返回 `Result` |
| `types_mut()` | 取得类型构建器，调用 `register_trait` / `register_type` |
| `register_category(CategoryInfo)` | 注册稳定类别 ID 与显示名（分类不带颜色） |
| `register_accent(id, TypeColor)` | 注册一类**节点专色**（如 `logic` 暗蓝、`interactive` 紫） |
| `register_tool(ToolSpec)` | 注册外部工具 ID 与可捕获状态的探测回调 |
| `register_node(NodeSpec)` | 登记节点契约、执行回调、可选控制流与 `accent` 归属 |
| `register_alias(old, target)` | 旧节点 ID 直接映射到正式 ID，不允许别名链 |
| `freeze()` | 消耗构建器，校验类别 / 别名 / 专色引用及类型接口环等，得到只读 `Registry` |
| `registry::install(registry)` | 一次性安装全局快照，必须早于**任何首次查询** |

`registry()` / `types()` / `catalog::all()` 等查询会惰性安装默认内置快照；先查询再 `install`
将返回 `AlreadyInitialized`。组装扩展时不要提前调用需要全局类型查询的 badge / color 等方法。
冻结后没有写入 API，**不支持热注册**。

类型注册接受借用的 `&str`，来源可以是启动时配置中拥有的 `String`。构建期将 ID、标签、
徽标与扩展名去重驻留到进程生命周期，调用方可以释放原字符串；查询 / 每帧绘制不反复泄漏。
节点元数据使用 owned 字符串。`TypeColor` 是 GUI 无关的 RGB 值，不是需要 UI 补分支的颜色枚举。

注册顺序示意（`spec` 是扩展自己构造的 `NodeSpec`）：

```rust
use starrytools_core::registry::{self, CategoryInfo, RegistrationError, RegistryBuilder};
use starrytools_core::types::TypeColor;

fn install_extension(spec: registry::NodeSpec) -> Result<(), RegistrationError> {
    let mut builder = RegistryBuilder::with_builtins()?;
    let color = TypeColor::rgb(160, 90, 40);
    builder.types_mut().register_trait("audio", "音频", "AUD", color, &["file"])?;
    builder.types_mut().register_type("wav", "WAV 音频", "WAV", color, &["audio"], Some("wav"))?;
    builder.register_category(CategoryInfo::new("audio-tools", "音频工具"))?;
    // 一类节点的专色：节点用 .accent("audio") 声明归属，颜色由颜色册统一给。
    builder.register_accent("audio", color)?;
    // spec.kind.category 必须使用上面已注册的类别 ID；
    // spec.accent 则必须使用颜色册里已注册的专色 id。
    builder.register_node(spec)?;
    registry::install(builder.freeze()?)
}
```

这只是注册边界示例，**不代表已经实现音频节点或解码**。Python / 脚本插件加载也**尚未实现**。
执行、动态端口、交互条件与工具探测回调都以 `Arc<dyn Fn + Send + Sync>` 保存，可用 `move`
捕获拥有的配置或 `Arc` 状态，不再限定为裸函数指针。例如在构造好 `kind` 后：

```rust
let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
let counter = calls.clone();
let spec = NodeSpec::fixed(kind, move |_args| {
    let n = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    Ok(one_output("count", Value::Number(n as f64)))
});
```

其中 `kind` 必须声明 `count: NUMBER` 输出；`NodeSpec`、`one_output`、`Value` 分别来自
`registry`、`model::value`。增加内置工具时在 `core/src/nodes/` 实现 `spec()`，然后在
`nodes/mod.rs::register` 调 `register_node`；可选功能如 `features/video/mod.rs::register`
统一贡献类别、工具、类型与节点，并在 `#[cfg(feature = "...")]` 下接入。

### 参数与端口：单一 core 契约

- `NodeKind` / `ParamDef` / `ParamSpec` / `PortDef` 全部定义在 core。app 直接使用它们，
  `catalog.rs` 只补 UI 摘要和依赖快照，不镜像一套参数、默认值、可见性或端口类型。
- 默认值由 `NodeKind::default_params` 推导，可见性用 `ParamDef::visible`；参数输入端口由
  `ParamSpec::input_type` / `NodeKind::inputs_for` 推导。下拉与动态端口个数不生成参数端口。
  `NodeSpec` 默认给副作用节点补 `exec` / `then`；纯节点用 `.pure()` 显式声明，入口不补执行输入。
- 动态输出用 `NodeSpec::dynamic(kind, resolve_outputs, run)`；UI 先显示廉价声明端口，
  随后应用后台静态检查返回的 `ResolvedNode` 快照，不在绘制时探文件头。
- 输入类型随参数变的节点（如「运算」）用 `NodeSpec::resolve_inputs` 声明；
  它只返回声明输入；参数端口由 `NodeKind` 补齐，执行引脚由 `NodeSpec` 补齐。
  `ParamSpec::Ports` 的数量按注册的 default / min / max 取整约束，异常存档不能触发无界绘制；
  Sequence 固定为同步串行，动态输出行没有阻塞开关。
- 需要询问用户的节点用 `.interactive(predicate)` 与 `args.ask(...)`；缺少界面时返回错误。
  外部程序先 `register_tool(ToolSpec::new(id, probe))`，节点再 `.needs_tool(id)`，
  执行回调仍须检查可用性。长任务可用 `args.report(NodeStep { ... })` 报进度。
  `Progress::new` 接收 `SyncSender`；app 两级进度队列各最多 512 项，只有后台生产者背压，
  UI 每帧非阻塞收取。detach / 关闭会断开接收端，解除堵塞；这不是计算任务取消功能。
- 图像节点保留 `inherit_provenance` 与 `with_name_hint`，使下游保存继续使用源文件名。
  错误返回 `NodeError`，不要在节点中 panic。

节点 ID 会进入存档，迁移用 `register_alias`；例如 `convert_to_png → convert_image`。
旧参数缺省值由默认值补齐，但端口 ID 改名不能靠节点别名自动修复。

## 代码结构

```text
core/src/
  registry.rs       节点 / 类别 / 专色 / 工具 / 别名构建器、冻结快照与控制流协议
  types/            类型注册、接口兼容、共同类型收窄
  model/            参数 / 端口元数据、值、工作流
  nodes/            内置工具及 register 入口
  features/video/   可选视频类型、节点、ffmpeg 调用
  engine/mod.rs     静态检查、通用执行、报告
  engine/control.rs Exec 调度、完整序列分支、嵌套遍历与作用域
  engine/artifacts.rs 自动产物目录与所有权清单
  png_opt/          无损 PNG 优化；png_quant.rs 为有损量化
app/src/
  main.rs           应用协调与窗口配置
  catalog.rs        直接消费 core 契约，异步工具探测快照
  canvas/
    graph.rs        共享画布状态和每帧协调入口，不包揽所有实现
    document.rs     工作流读写适配、编辑、复制粘贴
    interaction.rs  输入事件、拖动、连线合法性、右键划刀
    placement.rs    命中测试、端口位置与视野适配
    render.rs       画布、节点、端口、连线及刀光绘制
    parameters.rs   参数控件交互与动态输出编辑
    execution.rs    实时 / 最终结果、预览与纹理生命周期
    resources.rs    文件选择、模型和工具面板
    node.rs         UI 节点状态；layout.rs 共用排版
    view.rs         视口换算；geometry.rs 纯几何
  ui/
    panels/         chrome / library / prompt / report 浮层
    widgets/        按钮 / 表面与 controls 参数控件
    theme/          colors / metrics / fonts
    icons/          图标光栅化与 svgpath 解析
    motion.rs       动画时长、缓动、呼吸、spinner 与进出场节奏
  state/            workspace / settings / run / models / dialog / pacing
```

布局、命中和端口定位共用 `layout`，参数和端口语义始终从 core 获取。引擎按
`NodeSpec::control_flow(ControlFlow::Sequence / Loop { ... })` 调度，不靠内置节点 ID
写特例；新增普通数据节点不需要改控制流模块。

执行由显式 Exec 控制链同步串行调度，Data 只传值；纯节点按需重新求值。
Sequence 固定为同步串行并等待完整分支；Loop 分开 body / item / completed。
变量节点通过 `.variable(VariablePorts::new(value_param, get, set))` 注册；
Set 口需声明为 `PortDef::multi()`，状态在单次运行内由引擎维护。
单节点调试仅支持纯节点及其纯数据依赖。v1 需要显式迁移，app 当前拒绝加载。
完整契约见 [Blueprint v2 引擎](engine-blueprint.md)。

## 性能与界面验收

- `revision` 跟踪文档改动；`check_revision` 只跟踪增删节点 / 改参数 / 改线。静态检查与
  workflow 快照按后者缓存，拖动和缩放不重查文件头。
- `state/run.rs::Check` 用单个后台 worker 合并最新请求、去抖；以 generation + revision
  丢弃换图或编辑前的旧结果。检查未完成或当前语义有变化时不能运行旧快照。
- 工作流执行及缩略图生成在后台；UI 接收运行进度、缓存预览并上传最近邻纹理。
  工具探测也在后台，`catalog` 保存 Pending / Available / Missing 快照，探测完成后唤醒 UI。
  原生文件对话框、模型下载和产物复制不占用 UI 线程。
- 配色与尺寸集中在 `ui/theme/`，节点专色从 core 的颜色册取（分类不带颜色）；类型徽标也直接用 core 的 RGB。
  动画统一走 `ui/motion.rs`，面板、参数控件和画布共享时长 / 缓动令牌。
- `state/pacing.rs` 在 UI 入口做**整帧软件 pacing**，输入帧、egui 内部动画与后台唤醒都受限。
  等待预算扣除已花的 UI / 渲染 / present 时间，不在帧尾再叠加完整周期，不忙等。
  使用 `AutoNoVsync` 避免双重限帧；平台不支持时由 wgpu 回退，非 vsync 可能撕裂。

**帧率手感必须起窗口验收，单测不等于显示器实测。** 分别在 Windows / Linux 检查中文、
拖动 / 缩放 / 连线 / 划刀、图像预览、模型下载、视频子进程不闪控制台；在不同 FPS 设置下比较
拖动与静止动画的读数、输入延迟和撕裂。本轮仅完成 Windows 启动与中文显示冒烟验证，
尚未完成交互 / 不同显示器的帧率实测，不能宣称精确达到设定 FPS。
Linux 若明显慢，先用 `glxinfo -B` 排除 `llvmpipe` / `swrast` 软件光栅化。

## 打包

资源通过 `include_bytes!` / `include_str!` 编入程序；视频程序与 ONNX 模型仍是外部资源。
本轮 CI 不新增打包任务。

Linux 使用根目录现有 `Makefile`：

```sh
make install                      # 默认 ~/.local
make install PREFIX=/usr/local
```

它安装二进制、desktop 入口和 hicolor 图标。macOS / Windows 的 bundle 元数据见
`app/Cargo.toml`，可用 `cargo-bundle`（不属于本轮验证）：

```sh
cargo install cargo-bundle
cargo bundle --release -p starrytools-app
```

macOS 还需要从 PNG 生成 `.icns`；Windows 使用 `app/assets/icon.ico`。
