# 重命名

**id** `rename` · **分类** 通用

给流过的值起个名字，接在它后面的输出节点就用这个名字写盘。它是**通用**节点：流过来的
可以是图像，也可以是文本、数字、布尔 —— 都原样透传，一个字节都不动，类型也不变。
它做的唯一一件事，是把名字挂在值上带下去。

## 端口

| 方向 | 端口 | 类型 |
| --- | --- | --- |
| 输入 | `in` 值 | `Any`（必填） |
| 输出 | `out` 值 | `Any` |

`Any` 是通配：编辑期什么都能接进来、接出去，运行期再要求实际值是个说得清的类型。
所以「重命名」的输出可以接到任何端口上，接到 PNG 端口上时，
运行期会核一遍里面包的到底是不是 PNG。

## 参数

| 参数 | 控件 | 默认 | 说明 |
| --- | --- | --- | --- |
| 文件名 | 文本 | 空 | 不用自己写扩展名，下面那个开关会补 |
| 自检测后缀名 | 开关 | 开 | 按值实际的格式自动补上扩展名，例如 `.png` |

## 行为约定

- 起名字这一步不动像素、也不换格式，只是把名字捎给下游。
- 开着「自检测后缀名」时，扩展名按**实际格式**补：重命名时还是 PNG，下游压成 JPEG 了，
  写出来就是 `.jpg`。
- 关掉它，文件名就原样使用 —— 想自己带扩展名，或者干脆不要扩展名，都可以。
- 文件名留空时下游退回原来的起名方式，并给出一条提示。

## 实现

`core/src/nodes/rename.rs`，`NodeSpec::fixed`。名字不是存在图像里的，而是包在值外面：

```rust
enum Value {
    Text(..), Number(..), Bool(..), Image(..),
    /// 带着名字的值 —— 「重命名」节点的产物。里面是什么类型都行。
    Named(Box<NamedValue>),
}
```

`Value::port_type()` / `describe()` / `as_image()` 这些都会**透过名字**看里面真正的值，
所以下游节点不需要知道这层包装的存在。名字跟着值一路走，中间再经过几次编码也不会丢：
转换类节点用 `args.input_name("image")` 把它取出来，重新构造值之后再 `with_name_hint` 挂回去。

「保存到目录」是唯一消费这个名字的地方。做成通用节点是为了以后加非图像的工具时，
它已经能用了 —— 名字不挑类型。

测试：`rename_names_the_saved_file`、`rename_without_auto_extension_uses_the_name_verbatim`、
`rename_survives_a_re_encode_and_follows_the_real_format`（名字跨一次重编码）、
`rename_passes_a_non_image_value_through_untouched`（文本原样透传）、
`a_wildcard_output_may_feed_any_input_at_edit_time`。
