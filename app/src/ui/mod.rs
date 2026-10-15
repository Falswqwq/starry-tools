//! 主题令牌、通用控件和浮动面板。

pub(crate) mod icons;
pub(crate) mod motion;
pub(crate) mod panels;
pub(crate) mod theme;
pub(crate) mod widgets;

// 画布使用的公共模块入口；实现按职责归组，不复制契约或控件。
pub(crate) use icons::svgpath;
pub(crate) use motion as easing;
pub(crate) use widgets::controls;
