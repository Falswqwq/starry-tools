# StarryTools

一个节点式的小工具集合，面向像素画与游戏素材。

每个小功能是一个**节点**，由「输入 / 参数 / 输出」三部分构成：

```
   ●输入        参数        输出●
     │      ┌──────────┐      │
     └─────▶│ 压缩档位  │──────┘
            │ 色彩模式  │
            └──────────┘
```

把节点拖到画布上、用线连起来就是一个**工作流**，可以存下来下次接着用。
每个工作流都从「输入」节点开始。内置的节点在节点库里按用途分栏，随取随用。

一个典型的像素画工作流：

```
输入(webp) → 图像格式转换(→PNG) → 裁切到内容 → 缩放 400% → 重命名 → 保存到目录
```

## 跑起来

需要 Rust 1.90+、Node 18+，以及 Tauri 在各平台的前置依赖
（Linux 上是 `webkit2gtk-4.1`、`libgtk-3-dev`、`libsoup-3.0-dev` 等）。

```sh
npm install
npm run tauri dev            # 开发
npm run tauri build          # 打包

npm test                     # 前端：类型检查 + 单元测试
cd src-tauri && cargo test   # 后端（要先 npm run build 一次）
```

## 文档

全部文档在 [`docs/`](docs/README.md)：

| 主题 | 讲什么 |
| --- | --- |
| [节点](docs/nodes/) | 每个工具一篇：端口、参数、行为约定、实现与测试 |
| [端口类型系统](docs/types.md) | 连线时怎么判类型，两趟检查分别在哪做 |
| [界面与交互](docs/interface.md) | 画布、节点库、右键划刀、参数控件 |
| [配色与样式](docs/theme.md) | 一套 token、类型颜色、组件层的取舍 |
| [无损 PNG 优化](docs/png-optimization.md) | 「图像压缩」无损模式的内核 |
| [开发](docs/development.md) | 数据放在哪、怎么加一个新工具、代码结构 |

## 东西放在哪

| 内容 | 位置 |
| --- | --- |
| 工作流（一个一份 JSON） | `<应用数据目录>/com.falsw.starrytools/workflows/` |
| 运行产物 | `<应用数据目录>/com.falsw.starrytools/outputs/<工作流名>/` |

应用数据目录在 Linux 上是 `~/.local/share/`，macOS 上是 `~/Library/Application Support/`，
Windows 上是 `%APPDATA%\`。细节见[开发](docs/development.md#数据放在哪)。
