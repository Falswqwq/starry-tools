# StarryTools 文档

| 主题 | 讲什么 |
| --- | --- |
| [节点](nodes/) | 每个工具一篇：端口、参数、行为约定 |
| [Blueprint v2 引擎](engine-blueprint.md) | Exec/Data、变量、Sequence/Loop、版本迁移与运行报告 |
| [端口类型系统](types.md) | 类型怎么判、两趟检查分别在哪做 |
| [界面与交互](interface.md) | 画布、节点库、右键划刀、参数控件 |
| [配色与样式](theme.md) | 一套 token、类型颜色、组件层的取舍 |
| [无损 PNG 优化](png-optimization.md) | 「图像压缩」无损模式的内核 |
| [开发](development.md) | 环境与命令、数据放在哪、怎么加一个新工具、代码结构、性能与打包 |

v2 用执行线触发副作用、数据线传值；仅支持同步串行。v1 需显式迁移，目前拒绝加载且保留原存档。
文本 / 数字 / 布尔是节点本地变量，见[变量节点](variables.md)。

## 节点

分类 ID 来自节点的 `category`；显示名来自注册表的 `CategoryInfo`，节点库据此分栏。
分类**不带颜色** —— 节点的专属边线由**节点专色**决定（注册颜色册 + 节点声明所属类），
见[配色与样式](theme.md)。
默认 feature 为 `onnx` 与 `video`；关闭时对应节点不注册，详见[开发](development.md#feature-组合)。

### 来源

| 节点 | id | 一句话 |
| --- | --- | --- |
| [起点](nodes/start.md) | `start` | 显式执行入口（无输入，`out` 为 Exec） |
| [读取](nodes/read.md) | `read` | 从硬盘读一个文件：图像按图像解码，文本按文本读，视频读成视频产物（输出类型自动推断） |
| [读取目录](nodes/read_dir.md) | `read_dir` | 把一个目录里的文件读成一个列表（`List[Img]` / `List[Text]`） |

### 变量

| 节点 | id | 一句话 |
| --- | --- | --- |
| [文本](nodes/literal_text.md) | `literal_text` | 保存一段文本，带 Get / Set 数据口 |
| [数字](nodes/literal_number.md) | `literal_number` | 保存一个数字，带 Get / Set 数据口 |
| [布尔](nodes/literal_bool.md) | `literal_bool` | 保存一个开关，带 Get / Set 数据口 |
| [运算](nodes/operation.md) | `operation` | 两数加减乘除 / 取余 / 取整，或拼接两段文本（Exec 显式触发） |
| [Set](nodes/set.md) | `set` | 把数字 / 文本 / 布尔送上执行线，再接到变量的 Set 口 |

文本 / 数字 / 布尔没有执行线：Get 可以同时供多个下游读取，Set 可以接多个上游，
按上游实际产生值的顺序以后写到的为准。写入先影响本次运行中的后续读取，成功运行后
最终值写回节点参数，失败不提交。运算和 Set 是同一分类中的显式执行节点。
Exec 线控制副作用，Data 线只传值，二者不可混接。

### 图像

| 节点 | id | 一句话 |
| --- | --- | --- |
| [图像格式转换](nodes/convert.md) | `convert_image` | 在 PNG / JPEG / WebP / GIF / BMP / TIFF 之间转 |
| [图像压缩](nodes/compress.md) | `compress_image` | 无损：反复试组合留最小的；有损：转 JPEG 或量化成调色板 |
| [图像裁切](nodes/crop.md) | `crop_image` | 按比例、按尺寸、裁掉四周留白，或手动框一块（形状裁切是紫色交互节点） |
| [边框/边距](nodes/border.md) | `border_image` | 沿非透明像素边界描边（透明的洞也描）；整张不透明时就加边距 |
| [色彩分析](nodes/palette.md) | `color_analysis` | 抽色板：按主色归类，或数每种颜色多少像素（输出是一段 hex 文本） |
| [颜色剔除](nodes/remove_color.md) | `remove_color` | 把跟目标色相近的像素抠成透明 |
| [背景移除](nodes/background_removal.md) | `background_removal` | 用 AI 模型抠背景（仅 onnx feature，模型按需下载） |
| [图像变换](nodes/transform.md) | `transform_image` | 翻转、旋转，或按长宽比例分别拉伸 |
| [缩放图像](nodes/upscale.md) | `upscale` | 缩放到原来的百分之 n，像素画配「邻近」插值 |
| [图像分割](nodes/split.md) | `split_image` | 按透明部分把 sprite 图切成多张（输出 `List[Png]`） |

### 逻辑

控制流：给分支定先后、把列表解包。Exec 线与 Data 线始终分开。Sequence 固定为同步串行；
Loop 当前也只支持同步执行。它们的边线是**暗蓝色**（像紫色节点那样，一眼认出这一类）。

| 节点 | id | 一句话 |
| --- | --- | --- |
| [顺序执行](nodes/sequence.md) | `sequence` | 把接到各输出上的分支按序号从上到下依次执行（输出个数可增删） |
| [遍历](nodes/iterate.md) | `iterate` | `body` 触发循环体、`item` 传值，成功后走 completed |
| [重复执行](nodes/repeat.md) | `repeat` | 按次数重复循环体，`index` 输出当前轮次，成功后走 completed |

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
