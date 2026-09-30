//! 节点参数的读写。
//!
//! 参数用 [`serde_json::Value`] 的 Map 表示，好处是节点实例可以直接原样
//! 落盘、原样传给前端；代价是取值时要自己处理类型。下面的辅助函数把这件事
//! 收拢在一处，并且刻意写得宽容一点（例如数字参数写了字符串也能读）。

use serde_json::Value;

pub type Params = serde_json::Map<String, Value>;

pub fn string(params: &Params, key: &str, default: &str) -> String {
    match params.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => default.to_string(),
    }
}

pub fn string_opt(params: &Params, key: &str) -> Option<String> {
    match params.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

pub fn number(params: &Params, key: &str, default: f64) -> f64 {
    match params.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(default),
        Some(Value::Bool(b)) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        _ => default,
    }
}

pub fn integer(params: &Params, key: &str, default: i64) -> i64 {
    number(params, key, default as f64).round() as i64
}

pub fn boolean(params: &Params, key: &str, default: bool) -> bool {
    match params.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(s)) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        _ => default,
    }
}
