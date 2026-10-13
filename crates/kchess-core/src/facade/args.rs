use serde_json::Value;

use crate::error::{CoreError, Result};

/// `Date.now()`.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Null)
}

pub(crate) fn text_arg(args: &[Value], index: usize) -> String {
    arg(args, index).as_str().unwrap_or_default().to_string()
}

/// A validator's normalized value (`assertX(value)` in the TypeScript).
pub(crate) fn normal(name: &str, value: &Value) -> Result<Value> {
    normal_args(name, std::slice::from_ref(value))
}

/// A validator of several arguments, normalized (`assertX(a, b)` in the TypeScript).
pub(crate) fn normal_args(name: &str, args: &[Value]) -> Result<Value> {
    kchess_domain::misc::normalize(name, args).map_err(CoreError::new)
}
