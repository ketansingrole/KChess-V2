//! The argument arities of the core's methods and of the desktop's IPC invocations, for the
//! TypeScript frontends (`ts/generated/arity.ts`). The checks themselves run in Rust
//! (`kchess_domain::misc::contracts`); TypeScript only forwards arguments to them.

use kchess_domain::misc::{ASSISTANCE_METHODS, Scope, arities};

use crate::generate::Item;

/// `CORE_ARITY` and `IPC_ARITY`: method → `[min, max]` argument counts.
pub fn items() -> Vec<Item> {
    vec![
        arity_const("CORE_ARITY", Scope::Core),
        arity_const("IPC_ARITY", Scope::Ipc),
        assistance_const(),
    ]
}

fn arity_const(name: &str, scope: Scope) -> Item {
    let body: String = arities(scope)
        .iter()
        .map(|(method, min, max)| {
            #[expect(clippy::expect_used, reason = "serializing a &str cannot fail")]
            let key = serde_json::to_string(method).expect("method name serializes");
            format!("  {key}: [{min}, {max}],\n")
        })
        .collect();
    Item::Raw {
        name: name.to_string(),
        text: format!("export const {name} = {{\n{body}}} as const;"),
    }
}

/// `ASSISTANCE_METHODS`: core methods refused during a live human game, from
/// `kchess_domain::misc::contracts` so the CLI cannot drift from the facade's gate.
fn assistance_const() -> Item {
    let body: String = ASSISTANCE_METHODS
        .iter()
        .map(|method| {
            #[expect(clippy::expect_used, reason = "serializing a &str cannot fail")]
            let name = serde_json::to_string(method).expect("method name serializes");
            format!("  {name},\n")
        })
        .collect();
    Item::Raw {
        name: "ASSISTANCE_METHODS".to_string(),
        text: format!(
            "/** Core methods refused during a live human game (`assistance_allowed`). */\nexport const ASSISTANCE_METHODS: readonly string[] = [\n{body}];"
        ),
    }
}
