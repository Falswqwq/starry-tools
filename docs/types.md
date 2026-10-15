# 端口类型系统

端口带着类型，连线时就会检查。实现在 `core/src/types/`，与节点表共用一个冻结注册快照。

## 接口与具体类型

- **接口（trait）**是一族类型的共同身份，如 `IMG`（图像）、`VID`（视频）、
  `FILE`（能写盘的产物）、`ANY`（通配）。接口可声明其它接口为父接口。
- **具体类型**如 `PNG` / `JPEG` / `TEXT` / `MP4`，登记它实现的接口。没有具体类型继承；
  接口关系会传递解析，冻结时拒绝未知接口与环。

```text
ANY        通配接口（一切类型都兼容）
FILE       能写盘的产物接口
  IMG      图像接口，实现 FILE
    PNG / JPEG / GIF / WEBP / BMP / TIFF / ICO / QOI / TGA / PNM
  VID      视频接口，实现 FILE，启用 video 时注册
    MP4 / MKV / WEBM / MOV / AVI
TEXT / NUMBER / BOOL
VOID       空信号，只用于定执行先后
```

`PortType` 指向具体类型或接口；新类型不需要增加核心枚举分支。图像格式转换收 `IMG`，
缩放图像收 `PNG`，保存到目录收 `FILE`。

## 兼容规则

- **具体类型只接同一种**：`PNG` 接不下 `JPEG`，得先转换。
- **接口收下实现它的类型**：`FILE` 收 PNG / MP4，`IMG` 只收图像。
- **编辑期宽松、运行期严格**：`ANY` 或尚未确定具体格式的输出在编辑期可放行，
  执行时必须以实际值通过严格校验。

`accepts()` 用于编辑期，`strictly_accepts()` 用于运行期。

### 列表 `List[T]`

`PortType` 用一层列表标志表示集合，徽标为 `[IMG]` / `[TXT]` / `[ANY]`，颜色沿用元素类型。
列表只能接列表，元素仍按相同兼容规则；`List[Img]` 不是 `IMG` / `FILE` 本身。
当前不支持嵌套列表端口。

值列表的共同类型由注册表计算：同型保留具体类型，否则选择最具体的共同接口；不可比较的候选
按稳定 ID 排序打破平局。例如混合图像格式得到 `List[Img]`，混合图像与视频可得到
`List[File]`，没有更具体共同接口或空列表则为 `List[Any]`。不是在引擎里硬编码图像 / 视频分支。

[读取目录](nodes/read_dir.md)产出列表，[遍历](nodes/iterate.md)让下游对元素重复执行。

### 空输入与控制流

`VOID` 不携带数据。`NodeKind::inputs_for` 在节点没有任何其它输入（包括参数端口）时自动
添加可选 `seq: VOID` 输入，把起点或逻辑节点挂到控制流输出上只表示执行先后，不喂数据。

顺序屏障和循环调度实现在 `core/src/engine/control.rs`，根据注册表的
`ControlFlow::Sequence` / `ControlFlow::Loop` 描述执行，不根据内置节点 ID 判断种类。

## 启动期注册

通过 `registry::RegistryBuilder::with_builtins()` 建表，向 `types_mut()` 返回的类型构建器
调用 `register_trait` / `register_type`。视频的完整示例在
`core/src/features/video/mod.rs::register`：同时登记类别、工具、接口、具体类型与节点。

注册输入可以来自 owned 动态 `String`；类型 ID、标签、徽标及扩展名在构建期统一去重驻留，
原字符串随后可释放。不是每次查询都分配或泄漏，`PortType` 因而仍可 `Copy`。

`freeze()` 校验并消耗构建器，得到只读表；`registry::install(...)` 必须早于第一次
`registry()` / `types()` 等全局查询，否则默认内置快照已经初始化。冻结后无热注册。
节点 / 类别 / 工具 / 别名 API 与可捕获状态的 `Arc` 回调见[开发](development.md#注册类型与工具)。
这提供扩展边界，但不代表已经实现 Python 插件加载。

## 徽标颜色

同一类型的徽标在哪都同色，与节点选中 / 运行 / 报错状态无关。`TypeColor([r, g, b])`
是 core 的 GUI 无关 RGB 值，扩展直接注册新颜色，不需要增加颜色枚举或 UI 映射分支。
`app/src/ui/theme/colors.rs` 的 `badge_color` / `type_color` 只把注册值转成 `Color32`。
类别边线色来自单独的 `CategoryInfo.color`，不要与端口类型色混为一谈。

## 两趟检查

| 时机 | 做什么 |
| --- | --- |
| 图的语义改变后 | 后台 `engine::resolve` 静态检查：解析端口类型、判连线合法性并标注问题 |
| 执行时 | 根据上游实际值再次严格校验 |

界面直接消费 core 的 `PortDef.ty`，不维护另一套类型推导。同步拖线判定在
`app/src/canvas/interaction.rs::can_link` 调 `PortType::accepts`；后台检查的结果仍为准。
UI 先展示声明端口，后台解析完成后以当前 generation / revision 的 `ResolvedNode` 快照覆盖
动态端口；换图或改参数后的旧结果不会写回当前画布。

### 动态输出

用 `NodeSpec::dynamic(kind, resolve_outputs, run)`，解析回调可捕获拥有的状态：

| 节点 | 输出跟着什么变 |
| --- | --- |
| [读取](nodes/read.md) | 类型参数与所选文件的真实格式 |
| [图像格式转换](nodes/convert.md) | 目标格式 |
| [图像压缩](nodes/compress.md) | 压缩方式与有损格式 |
| [视频压缩](nodes/video_compress.md) | 输出格式（仅 video feature） |
| [顺序执行](nodes/sequence.md) | 输出个数 |
| [遍历](nodes/iterate.md) | 解包项数 |

可变个数由 `ParamSpec::Ports` 声明，参数存数量。`canvas/parameters.rs` 与
`ui/widgets/controls.rs` 根据元数据提供行、删除按钮和加号；`PortDef::panel_row`
指定输出挂在参数区哪一行，不必占据顶栏端口列。

节点库的卡片是**声明模板**，`registry::kinds()` 不运行动态解析，所以显示泛化的
`ANY` / `IMG` / `VID`。拖出后后台检查才解析为具体格式，不在每帧绘制时读取文件头。

## 参数端口与单一契约

节点、端口、参数、默认值与条件可见性都定义在 `core/src/model/node_kind.rs`。
app 直接使用 `NodeKindInfo` / `PortDef` / `ParamDef` / `ParamSpec`，不复制一套 UI 类型。
`catalog.rs` 只补 UI 摘要、工具 / 模型依赖快照。

`ParamSpec::input_type` 声明参数能否被上游喂值：

| 参数 | 参数端口类型 |
| --- | --- |
| 数字 / 滑块 | `NUMBER` |
| 文本 / 文件 / 目录 / 颜色 | `TEXT` |
| 开关 | `BOOL` |
| 下拉 / 动态输出个数 | 不生成 |

`NodeKind::inputs_for(params)` 根据 `ParamDef::visible` 只为可见、可喂值参数生成
`param:<id>` 可选输入；起点节点不生成参数输入。默认值由 `NodeKind::default_params`
推导，不能在 UI 手抄。连线后 UI 禁用本地控件，执行时 core 将输入值覆盖对应参数，
而不是当成普通数据输入；拔线后恢复本地值。

## 色板与通用产物

色板就是 `TEXT`，约定每行一个 `#rrggbb`；颜色参数收到多色时取第一种。引擎用
`model::value::parse_palette` 提取 `PortResult.palette`，UI 据此画色块。

图像使用带解码缓存与预览的 `ImageValue`；视频等通用产物使用 `core/src/media.rs`
的 `MediaValue`（注册类型 ID、编码字节与元信息），写盘扩展名也查登记表。
「保存到目录」接 `FILE` 接口，因而不必为每一种产物改节点实现。
