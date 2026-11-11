use serde_json::Value;

use super::Out;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    kchess_domain::theme::call(method, args)
}
