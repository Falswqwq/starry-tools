# 无损 PNG 优化

「图像压缩」的无损模式不是「重新编码一次 PNG」，而是把能同时满足**像素一个都不变**的
选择都试一遍，最后留最小的那一份。

实现在 `src-tauri/src/png_opt/`（`mod.rs` + `tests.rs`）。

```rust
pub enum Scheme { Fast, Balanced, Maximum, Zopfli }
pub enum Strip  { Safe, All }

pub struct Options { pub scheme: Scheme, pub strip: Strip, pub verify: bool }

pub fn optimize(image: &DynamicImage, source_png: Option<&[u8]>, options: &Options)
    -> Result<Optimized, NodeError>
```

`source_png` 是输入本来就是 PNG 时的原始字节 —— 只用来搬运颜色相关的元数据块，以及和
原始体积作对比。输入不是 PNG 就传 `None`。

## 管线

| 这一层 | 做了什么 |
| --- | --- |
| 颜色类型 / 位深 | 分析实际用到的颜色与 Alpha：没有透明就不用 RGBA，是灰度就存灰度，颜色少到装得进调色板就转索引色，并按颜色数挑 1 / 2 / 4 / 8 位 |
| 逐行 filter | 交给 `png` 的「逐行最小熵」/「逐行自适应」按行挑 —— **不同的行可以用不同的 filter**；几种策略都真的编出来比大小 |
| DEFLATE | 快速 / 均衡 / 最大 三档 |
| Zopfli | 「Zopfli」档再把 IDAT 解出来交给 zopfli 重压。只换压缩后的表示，filter 过的那串数据一个字节没动 |
| 元数据 | 默认只留 `iCCP` / `sRGB` / `gAMA` / `cHRM`，其余文本 / 时间 / EXIF 去掉；选「全部剥离」连颜色块也一起去掉 |
| 不划算就不做 | 每种组合都真的编出来比大小，比原图大就原样返回原图 |

## 候选是怎么来的

`Analysis::of()` 先把图像**统一展开成 RGBA**（8 位源用 `to_rgba8`，16 位源用 `to_rgba16`），
走一遍统计出四件事：有没有真透明、是不是灰度、灰度能用多小的位深、以及唯一色有没有超过 256。

`Analysis::specs()` 据此列出要试的组合：

| 条件 | 候选 |
| --- | --- |
| 有透明 | `RGBA`；是灰度再加 `灰度 + 透明` |
| 不透明 | `RGB`；是灰度再加 `灰度`，能降位深就降到 1 / 2 / 4 位 |
| 唯一色 ≤ 256 | 再加一个**索引色**，位深取装得下这些颜色的最小值 |

索引色的调色板按 RGBA 四元组建表（不是 RGB），这样 alpha 可以通过 `tRNS` 精确还原 ——
半透明的颜色也能无损进调色板。调色板按字典序排好，保证同样的输入得到同样的结果。

## 逐行 filter

`png` 0.18 提供了 `Filter::{MinEntropy, Adaptive, Paeth, …}`，前两个都是**逐行**自选，
正好对上要求里「不同的行用不同 filter」。每个档位试哪些：

| 档位 | filter | DEFLATE |
| --- | --- | --- |
| 快速 | 逐行自适应 | fdeflate（`Compression::Fast`） |
| 均衡 | 逐行最小熵、逐行自适应 | flate2 均衡 |
| 最大 / Zopfli | 逐行最小熵、逐行自适应、Paeth | flate2 最高档 |

候选数 × filter 数是挨个真编的，没有偷懒的启发式 —— 这也正是「最大」慢的原因。

## Zopfli

`png` 不让我们换掉它的 DEFLATE 实现，所以走的是**重压缩**：

1. 把编码结果拆成块，把 `IDAT` 拼起来；
2. 用 `flate2` 的 zlib 解码器解出 **filter 之后的那串原始数据**；
3. 用 `zopfli` 以 zlib 格式重压；
4. 重建容器：其余块原样搬过去，`IDAT` 换成新的（长度和 CRC 重算）。

第 2 步解出来的东西就是每个扫描行前面那个 filter 字节加上滤波后的数据 —— 一个字节都
不改，所以像素不可能变。输入超过 4 MB 时会自动把 zopfli 的迭代轮数降到 5，
免得一次优化要等上好几分钟。

## 元数据搬运

PNG 的块结构很好拆（长度 + 类型 + 数据 + CRC），所以这里自己扫一遍：

- **能搬的**：`iCCP` / `sRGB` / `gAMA` / `cHRM` —— 像素值一个都没改，颜色空间的描述当然
  还是对的。搬过去时按惯例摆在 `IHDR` 后面。
- **不搬的**：`bKGD` / `hIST` / `sBIT` / `sPLT` / `tRNS` 这些和颜色类型绑死，换过类型就
  不敢搬了；文本、时间、EXIF 是「清理」的对象；APNG 的 `acTL` / `fcTL` / `fdAT` 直接丢 ——
  我们是单帧工具。

## 校验

收尾一定逐像素验一遍：把结果解回来，和原图比宽、高，以及每个像素的 R / G / B / A
（16 位源按 16 位比）。对不上就当作这次优化没发生 —— `optimize` 直接返回错误，
节点会把它报成失败。**宁可没优化，也不能悄悄改了图。**

这一趟解码 + 比对的代价相对编码可以忽略，所以默认一直开着（`Options::verify` 可以关，
但节点不会关）。

## 边界与取舍

- **16 位源**只走 RGB / RGBA / 灰度 / 灰度+透明（16 位），不做调色板也不压亚字节位深 ——
  PNG 的调色板只有 8 位，硬套会改变数值。
- **超大图**（超过 1600 万像素）不再穷举候选，只留最稳的那一种编码；像素画远够不着这条线。
- **已经压得很紧的 PNG** 会原样返回原图，并说明「优化后反而更大」。
- 什么时候会「反而更大」其实很常见，不是异常：给一张 16×16 的图配个 3 色调色板，
  光是 `PLTE` + `tRNS` 的头就比 RGBA 那点数据还多。目标是**最小**，不是「看上去最聪明」。

## 有损那条路

量化在 `src-tauri/src/png_quant.rs`，用最经典的中位切分（4 维含 alpha 一起切）。
它**只在「有损」模式里用**，和无损这条完全分开。量化完那张图会再交给 `png_opt` 去编码 ——
颜色一少，索引色编码自己就会被选中。详见[图像压缩](nodes/compress.md)。

## 测试

`src-tauri/src/png_opt/tests.rs`。每条都盯着同一个底线：**优化完的像素必须和原来一模一样**
（测试里的 `run()` 帮手顺手就断言了这一点）。

| 用例 | 盯什么 |
| --- | --- |
| `rich_rgba_image_is_lossless_and_keeps_alpha` | 颜色多又带透明的图，不能被降成 RGB |
| `opaque_image_never_keeps_an_alpha_channel` | 整张不透明就不该留 alpha |
| `grayscale_image_is_stored_as_grayscale` | 灰度图要走灰度或索引色 |
| `bilevel_image_compresses_to_one_bit` | 两极灰度得是 1 位 |
| `transparent_pixel_art_keeps_its_transparency` | 透明像素还是完全透明 |
| `colour_counts_pick_the_smallest_bit_depth` | 1 / 2 / 4 / 16 / 256 色各自的位深 |
| `sixteen_bit_images_stay_sixteen_bit` | 16 位不能偷偷降到 8 位 |
| `every_scheme_gives_the_same_pixels` | 三个档位编出来像素都一样 |
| `zopfli_scheme_keeps_pixels_and_does_not_grow` | zopfli 重压不动像素 |
| `a_tiny_image_may_prefer_the_simplest_encoding` | 小图选最朴素的编码才对 |
| `an_already_optimal_png_is_left_alone` | 再压一遍会原样返回 |
| `savings_are_reported_against_the_original` | 前后大小与节省比例算得对 |
| `text_chunks_are_stripped_but_colour_chunks_are_kept` | 默认只剥文本类 |
| `aggressive_stripping_drops_colour_chunks_too` | 「全部剥离」连颜色块一起去 |
| `sub_byte_rows_are_padded_per_row` | 亚字节位深每行补到整字节 |
| `gray_bit_depth_picks_the_narrowest_fit` | 灰度位深挑得最小 |
| `chunk_round_trip_preserves_everything` | 块拆开再拼回去一个字节不差 |
