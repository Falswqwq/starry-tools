# 色彩分析

**id** `color_analysis` · **分类** 图像

从图里抽出一个**色板**。

## 端口

| 方向 | 端口 | 类型 |
| --- | --- | --- |
| 输入 | `image` 图像 | `Image(Any)`（必填） |
| 输出 | `palette` 色板 | `Text` |

输入不锁格式（`Image(Any)`）—— PNG、JPEG、WebP… 什么图都能直接分析，不用先转 PNG。

输出是一段**普通的文本**：每行一个 `#rrggbb`。所以「色板」不需要新类型：

- 它能接到任何「颜色」参数上（见下），
- 也能当普通文本传给别的节点。

## 参数

| 参数 | 控件 | 默认 | 何时显示 |
| --- | --- | --- | --- |
| 模式 | 下拉 | 提取调色板 | 总是 |
| 提取算法 | 下拉 | K-Means + OKLab | 提取调色板 |
| 颜色数 | 数字 2–256 色 | 8 | 提取调色板 |
| 相近色合并阈值 | 数字 0–255 | 16 | 数量统计 |

## 两种模式

### 提取调色板

把图里的颜色归成「颜色数」种**主色**。三种算法，各有各的脾气：

| 提取算法 | 一句话 | 什么时候用 |
| --- | --- | --- |
| **K-Means + OKLab** | 在感知均匀的 OKLab 空间里聚类，**对比度高的颜色也算数**，不只是被像素多少决定 | 通用主题色提取（推荐） |
| **Median Cut** | 经典中位切分，最快；在 RGB 里按像素数切，谁像素多谁说了算 | 像素画、快速调色板 |
| **K-Medoids + OKLab** | 同样在 OKLab 聚类，但每簇的代表**必须是原图里真实存在的颜色** | 要颜色自然、真实 |

拿不准就用 K-Means。颜色本来就不多于「颜色数」时原样列出，不硬凑。

几种算法的取舍：

- **K-Means 用 OKLab 而不是 RGB**：RGB 里「暗红 ↔ 亮红」的距离看起来很大、而两种鲜艳色的
  差别看起来很小，和眼睛不一致。OKLab 里数值距离≈人眼看到的差别，所以一撮像素少但很跳
  的颜色也能自己占一个位置，不会被一大片相近的底色吞掉。
- **K-Medoids 为什么不也一样**：K-Means 的每簇代表是**算出来的平均色**，可能原图里根本
  没有这个颜色；K-Medoids 把代表限制在簇内某个真实颜色上（medoid），所以输出更「自然」。
  代价是迭代更贵一点。
- **颜色很多时**（照片）会先把颜色按分位压到一个采样上限（4096）再聚类，保证速度；
  像素画那种颜色本来就少的图不受影响。

### 数量统计

数每种颜色有多少像素，把**相近色并成一种**再按像素数从上到下排。

- 相近的判定按分量**分桶**：把每个分量除以阈值（至少 1）取整当桶号，同一个桶算一种，
  代表色取桶内实际颜色的平均。
- 桶是网格对齐的：正好跨在桶边界上的两种相近色可能不会被并到一起，阈值调大一点通常
  就盖住了。阈值 0 就是完全按原色统计。
- 所以输出就是「用得最多的颜色排在最上面」。

## 色板当颜色用

「颜色」参数（`ParamSpec::Color`）的输入端口类型是 `Text`，可以接色板过来。一个节点
只要一种颜色、却拿到一整板多色的，就**取第一种颜色**（第一行）。所以：

```
读取 → 色彩分析（数量统计）→ 边框/边距「颜色」
```

边框就会用图里出现最多的那个颜色。

## 卡片上的展示

跑完之后节点卡片底下会摆一排**小色块**（色板）。色块太多时只画放得下的那些，不挤成一团。

## 实现

`core/src/nodes/palette.rs`（节点）+ `core/src/palette.rs`（算法）。

- **提取**：`palette::extract(&image, n, algorithm)`：
  - `MedianCut` 直接转发 `png_quant::representative_colors`；
  - `KMeans` / `KMedoids` 先把颜色转 **OKLab**，再用**加权 k-means++**（第一个中心取像素
    最多的颜色，之后按「越远 × 越多越容易被选中」抽样）初始化，然后 Lloyd 迭代；
  - K-Medoids 的每簇代表取簇内「到其他点的加权距离和」最小的真实颜色；
  - 抽样用固定种子的 SplitMix64，所以结果确定、可复现；
  - `SAMPLE_CAP = 4096`：颜色更多时先按 4 位/分量分桶压到上限再聚类。
- **统计**：`counted_colors(&rgba, threshold)` —— 一个 `HashMap<(u32, u32, u32), (sum, count)>`
  按桶累计，最后取均值并按 count 降序。
- 输出用 `model::value::palette_text` 拼成文本；引擎在打包运行报告时用
  `parse_palette` 认出它、把颜色拆到 `PortResult.palette` 里，界面据此画色块。

全透明的像素不算颜色；半透明的像素按 RGB 归类，色板里不带 alpha。

测试：`palette` 模块里有 `oklab_round_trips_through_its_inverse`、
`every_algorithm_finds_distinct_theme_colours`、`a_small_but_very_different_colour_still_gets_a_slot`
（对比度高的少数色也该占一席）、`kmedoids_only_returns_colours_from_the_image`、
`algorithms_are_deterministic`、`transparent_pixels_do_not_count`；
节点里有 `count_merges_near_colors_and_sorts_by_how_often_they_appear`、
`fully_transparent_pixels_are_ignored`；引擎侧还有
`color_analysis_outputs_a_palette_text`、`color_analysis_over_an_any_image_input_takes_any_format`、
`a_palette_feeds_a_colour_parameter_by_its_first_colour`。
