use serde_json::Value;

use kchess_domain::misc::value::J;

use super::Out;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    if method == "validateCoreArguments" {
        return Some(validate_core_arguments(args));
    }
    kchess_domain::misc::call(method, args)
}

/// `[sidecar, method, args]`: the core's argument check of one call, null when it passes.
fn validate_core_arguments(args: &[Value]) -> Out {
    let Some((sidecar, user)) = args.split_first() else {
        return Err("validateCoreArguments was called without its lossless arguments".into());
    };
    match kchess_domain::misc::inputs(user, sidecar)?.as_slice() {
        [J::Str(name), J::Arr(list)] => {
            kchess_domain::misc::contracts::check(name, list).map(|()| Value::Null)
        }
        _ => Err("The arguments of an argument check are a method name and a list.".into()),
    }
}
