# StarryTools

一个节点式的小工具集合，面向像素画与游戏素材。

每个小功能是一个**节点**，由「输入 / 参数 / 输出」三部分构成。把节点拖到画布上、用线连起来
就是一个**工作流**，可以存下来下次接着用。内置节点在节点库里按用途分栏。

一个典型的像素画工作流：

```text
读取(webp) → 图像格式转换(→PNG) → 裁切到内容 → 缩放 400% → 重命名 → 保存到目录
```

## Blueprint v2

执行线（Exec）决定节点何时运行，数据线（Data）只传值，两者不能混接。
上例还需从「起点」沿各节点的 `exec` / `then` 连接执行线，并单独连接数据端口；
只接数据不会自动运行读取、处理或保存节点。变量 Get 按需纯求值；运算节点由执行线显式触发。

数字、文本、布尔节点在「变量」分类中，各有一个 Get 和一个 Set 数据口，不接执行线。
Get 可供多个下游读取；Set 可接多个上游，按上游实际产生值的顺序以后写到的为准。
写入先影响本次运行中的后续读取；成功后最终值写回节点参数，下一次运行从这里继续，
失败运行不提交。Set 本身不会触发任何节点。

Sequence 按序执行每个完整分支；遍历用 `body` 触发循环体、`item` 传当前元素，
全部成功后走 completed（端口 id 为 `finally`，空列表也触发）。Sequence 固定为同步串行。
v1 存档需要显式迁移入口、执行线和分支顺序；目前拒绝加载，不会自动升级或只修改版本号。
详见 [Blueprint v2 引擎](docs/engine-blueprint.md)。

## 跑起来

使用当前 stable Rust 和一个能跑 egui/wgpu 的图形环境。最低版本以 Cargo 清单及锁定依赖为准：
core 声明 `rust-version = "1.91"`，当前 GUI 依赖 `eframe 0.36.2` 要求 **Rust 1.95+**。
没有 Node，也不依赖 WebView，界面是纯 Rust。

```sh
cargo run --release -p starrytools-app
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
```

默认启用 `onnx`（背景移除）和 `video`（需外部 `ffmpeg` / `ffprobe`）。也可只编译基础功能：

```sh
cargo run --release -p starrytools-app --no-default-features
```

Windows 推荐标准 **MSVC** Rust 工具链。Linux 系统依赖、四种 feature 的验证命令、
CI 覆盖范围及 GNU 工具链排错见[开发](docs/development.md#环境与命令)。
打包见[开发 → 打包](docs/development.md#打包)。

## 文档

全部文档在 [`docs/`](docs/README.md)：

| 主题 | 讲什么 |
| --- | --- |
| [节点](docs/nodes/) | 每个工具一篇：端口、参数、行为约定 |
| [Blueprint v2 引擎](docs/engine-blueprint.md) | 执行线 / 数据线、变量、序列与遍历、版本迁移 |
| [端口类型系统](docs/types.md) | 注册式类型、连线兼容性、参数单一契约 |
| [界面与交互](docs/interface.md) | 画布、节点库、参数控件、整帧限速 |
| [配色与样式](docs/theme.md) | 主题、注册式类别颜色、统一动画 |
| [无损 PNG 优化](docs/png-optimization.md) | 「图像压缩」无损模式的内核 |
| [开发](docs/development.md) | 注册 API、代码结构、数据安全、跨平台验证与打包 |

## 东西放在哪

| 内容 | 位置 |
| --- | --- |
| 工作流（一个一份 JSON） | `<应用数据目录>/com.falsw.starrytools/workflows/` |
| 运行产物 | `<应用数据目录>/com.falsw.starrytools/outputs/<slug>--<encodedID>/` |
| 下载的 ONNX 模型 | `<应用数据目录>/com.falsw.starrytools/models/` |

应用数据目录在 Linux 上默认是 `~/.local/share/`，macOS 上是 `~/Library/Application Support/`，
Windows 上是 `%APPDATA%\`。产物目录以净化名称和完整编码的工作流 ID 隔离同名工作流；
完整运行只回收清单中归属自己且未被修改的文件，不按 `NN-` 前缀猜删旧文件。
细节见[开发](docs/development.md#数据放在哪)。
