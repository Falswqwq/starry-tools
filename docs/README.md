# StarryTools 文档

| 主题 | 讲什么 |
| --- | --- |
| [节点](nodes/) | 每个工具一篇：端口、参数、行为约定 |
| [端口类型系统](types.md) | 类型怎么判、两趟检查分别在哪做 |
| [界面与交互](interface.md) | 画布、节点库、右键划刀、参数控件 |
| [配色与样式](theme.md) | 一套 token、类型颜色、组件层的取舍 |
| [无损 PNG 优化](png-optimization.md) | 「图像压缩」无损模式的内核 |
| [开发](development.md) | 环境与命令、数据放在哪、怎么加一个新工具、代码结构、性能与打包 |

## 节点

分类来自节点自己声明的 `category`，节点库左栏就是按它分栏的。

### 来源

| 节点 | id | 一句话 |
| --- | --- | --- |
| [读取](nodes/read.md) | `read` | 从硬盘读一个文件：图像按图像解码，文本按文本读，视频读成视频产物（输出类型自动推断） |
| [文本](nodes/literal_text.md) | `literal_text` | 一段文本（字面量） |
| [数字](nodes/literal_number.md) | `literal_number` | 一个数字（字面量） |
| [布尔](nodes/literal_bool.md) | `literal_bool` | 一个开关（字面量） |

字面量节点一个装一个值，接到别的节点的参数端口上就能把那个参数改从这里取。

### 图像

| 节点 | id | 一句话 |
| --- | --- | --- |
| [图像格式转换](nodes/convert.md) | `convert_image` | 在 PNG / JPEG / WebP / GIF / BMP / TIFF 之间转 |
| [图像压缩](nodes/compress.md) | `compress_image` | 无损：反复试组合留最小的；有损：转 JPEG 或量化成调色板 |
| [图像裁切](nodes/crop.md) | `crop_image` | 按比例、按尺寸、裁掉四周留白，或手动框一块（形状裁切是紫色交互节点） |
| [边框/边距](nodes/border.md) | `border_image` | 沿非透明像素边界描边（透明的洞也描）；整张不透明时就加边距 |
| [色彩分析](nodes/palette.md) | `color_analysis` | 抽色板：按主色归类，或数每种颜色多少像素（输出是一段 hex 文本） |
| [颜色剔除](nodes/remove_color.md) | `remove_color` | 把跟目标色相近的像素抠成透明 |
| [背景移除](nodes/background_removal.md) | `background_removal` | 用 AI 模型认出主体、抠掉背景（模型要先下载） |
| [图像变换](nodes/transform.md) | `transform_image` | 翻转（左右 / 上下）与旋转（90° / 180° / 270°） |
| [缩放图像](nodes/upscale.md) | `upscale` | 缩放到原来的百分之 n，像素画配「邻近」插值 |

### 视频

启用 `video` feature（默认开）时才有。视频的编解码交给外部 `ffmpeg`（缺了会给警报）。

| 节点 | id | 一句话 |
| --- | --- | --- |
| [视频压缩](nodes/video_compress.md) | `video_compress` | 用 ffmpeg 重编码压小：平衡 / 激进 / 自动 / 高级 |

### 通用

| 节点 | id | 一句话 |
| --- | --- | --- |
| [重命名](nodes/rename.md) | `rename` | 给流过的值起个名字，下游写盘就用它 |

### 产出

| 节点 | id | 一句话 |
| --- | --- | --- |
| [保存到目录](nodes/save.md) | `save_output` | 把产物（图像 / 视频……）写到你挑的目录，输出是写下的路径 |

## 一篇节点文档写什么

每个工具一篇，说清四件事：

- **端口** —— 输入 / 输出的类型。
- **参数** —— 每个参数的控件、默认值、一句话说明。
- **行为约定** —— 不看代码看不出来的规矩：什么时候会提示、什么时候原样放行、
  什么情况下会报错。
- **注意事项** —— 节点库里展开卡片时列出的那几句。

端口 / 参数 / 注意事项是节点的**元数据**，写在 `core/src/nodes/*.rs` 里，界面直接渲染出来；
文档把它们列全，重点补在「行为约定」上。
