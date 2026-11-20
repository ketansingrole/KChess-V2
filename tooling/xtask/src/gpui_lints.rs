//! The GPUI mistakes that compile and then fail at run time (AGENTS.md, "Writing GPUI views"),
//! found in the app's sources as Zed's dylint lints find them in its own (`tooling/lints` there):
//! `cargo xtask lints`, and in `check-fast` when an app source changed; CI runs it as a test.
//!
//! Read from the syntax, not the types: Zed's lints run inside rustc on a pinned nightly with its
//! private crates, which would build the whole workspace a second time in CI. A view's `render` is
//! known by its trait (`impl Render for …`, `impl RenderOnce for …`), and what a closure or an
//! async block in it does runs later, so it is left alone, as GPUI allows.
//!
//! - `notify_in_render`: `cx.notify()` in `render` asks for another frame every frame.
//! - `update_in_render`: `entity.update(cx, …)` in `render` changes state while drawing it.
//! - `blocking_in_render`: the disk, a process or a sleep in `render` stops every window.
//! - `reentrant_update`: `x.update(cx, |…| x.update(…))` borrows the entity twice: GPUI panics.
//!
//! A line that must stay says why on the line before: `// gpui-lints: allow(<lint>) <reason>`.

use std::path::Path;

use proc_macro2::Span;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};

use crate::members::APP_SOURCES;
use crate::util::Result;

/// What one lint found: where, which lint, and what to do instead.
#[derive(Debug, PartialEq)]
struct Finding {
    line: usize,
    lint: &'static str,
    message: String,
}

/// Calls in `render` that wait on the disk, another process or the clock, by their path's last
/// two segments (`fs::read`, `File::open`); `*` is any function of that module.
const BLOCKING_CALLS: [(&str, &str); 6] = [
    ("fs", "*"),
    ("File", "open"),
    ("File", "create"),
    ("thread", "sleep"),
    ("Command", "new"),
    ("Path", "exists"),
];

/// Methods that ask the disk about a path, in `render`.
const BLOCKING_METHODS: [&str; 7] = [
    "exists",
    "try_exists",
    "is_file",
    "is_dir",
    "canonicalize",
    "read_dir",
    "symlink_metadata",
];

/// Every finding in `text`, a file of the app's sources; tests and test modules are left out.
fn lint(text: &str) -> std::result::Result<Vec<Finding>, String> {
    let file = syn::parse_file(text).map_err(|cause| cause.to_string())?;
    let mut lints = Lints::default();
    lints.visit_file(&file);
    let lines: Vec<&str> = text.lines().collect();
    let allowed = |finding: &Finding| {
        let marker = format!("gpui-lints: allow({})", finding.lint);
        (finding.line >= 2)
            .then(|| lines.get(finding.line - 2))
            .flatten()
            .is_some_and(|before| before.contains(&marker))
    };
    Ok(lints
        .found
        .into_iter()
        .filter(|finding| !allowed(finding))
        .collect())
}

fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        path.is_ident("test")
            || path
                .segments
                .last()
                .is_some_and(|last| last.ident == "test")
            || (path.is_ident("cfg")
                && attr
                    .parse_args::<syn::Ident>()
                    .is_ok_and(|ident| ident == "test"))
    })
}

fn line(span: Span) -> usize {
    span.start().line
}

/// The whole file: views' `render`s, and every `update` for its reentrancy.
#[derive(Default)]
struct Lints {
    found: Vec<Finding>,
}

impl Lints {
    fn report(&mut self, span: Span, lint: &'static str, message: impl Into<String>) {
        self.found.push(Finding {
            line: line(span),
            lint,
            message: message.into(),
        });
    }
}

impl<'ast> Visit<'ast> for Lints {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !is_test(&item.attrs) {
            visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !is_test(&item.attrs) {
            visit::visit_item_fn(self, item);
        }
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if is_test(&item.attrs) {
            return;
        }
        let view = item.trait_.as_ref().and_then(|(_, path, _)| {
            let last = path.segments.last()?;
            (last.ident == "Render" || last.ident == "RenderOnce").then_some(())
        });
        if view.is_some() {
            for inner in &item.items {
                if let syn::ImplItem::Fn(function) = inner
                    && function.sig.ident == "render"
                {
                    let mut render = InRender { lints: self };
                    render.visit_block(&function.block);
                }
            }
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if is_update(call)
            && let Some(syn::Expr::Closure(closure)) = call.args.iter().nth(1)
        {
            let entity = receiver(&call.receiver);
            let mut nested = SameUpdate {
                entity: &entity,
                found: Vec::new(),
            };
            nested.visit_expr(&closure.body);
            for span in nested.found {
                self.report(
                    span,
                    "reentrant_update",
                    format!(
                        "`{entity}` is updated inside its own `update`: GPUI panics on the second \
                         borrow. Use the closure's own entity and `cx`"
                    ),
                );
            }
        }
        visit::visit_expr_method_call(self, call);
    }
}

/// `x.update(cx, |…| …)` or `x.update_in(…)`: an entity's update, by its two arguments.
fn is_update(call: &syn::ExprMethodCall) -> bool {
    (call.method == "update" || call.method == "update_in") && call.args.len() == 2
}

/// The receiver as written, without spaces: `self.board`, `entity`.
fn receiver(expr: &syn::Expr) -> String {
    quote::ToTokens::to_token_stream(expr)
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Updates of one entity inside a closure of its own update.
struct SameUpdate<'a> {
    entity: &'a str,
    found: Vec<Span>,
}

impl<'ast> Visit<'ast> for SameUpdate<'_> {
    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        // An async closure runs after the update returns: its updates are its own.
        if closure.asyncness.is_none() {
            visit::visit_expr_closure(self, closure);
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        // `self.x` in the closure is the same entity only when written the same way, and a
        // closure's own `this` is never the outer receiver's name.
        if is_update(call) && receiver(&call.receiver) == self.entity {
            self.found.push(call.method.span());
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {
        // An async block runs after the update returns: its updates are its own.
    }
}

/// A view's `render` body, outside its closures and async blocks.
struct InRender<'a> {
    lints: &'a mut Lints,
}

impl<'ast> Visit<'ast> for InRender<'_> {
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {
        // Handlers and listeners run later, outside the frame.
    }

    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}

    fn visit_item(&mut self, _: &'ast syn::Item) {}

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if method == "notify" && call.args.is_empty() {
            self.lints.report(
                call.method.span(),
                "notify_in_render",
                "`notify` in `render` asks for another frame every frame: notify where the state \
                 changes, in the handler or task that changed it",
            );
        } else if is_update(call) {
            self.lints.report(
                call.method.span(),
                "update_in_render",
                "`update` in `render` changes state while drawing it: change it in the handler or \
                 task that learned of the change",
            );
        } else if BLOCKING_METHODS.contains(&method.as_str()) && call.args.is_empty() {
            self.lints.report(
                call.method.span(),
                "blocking_in_render",
                format!(
                    "`{method}` asks the disk from `render`, on the main thread: read it in \
                     `cx.background_spawn` and keep the answer"
                ),
            );
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            let segments: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            if let [.., module, function] = segments.as_slice() {
                let blocking = BLOCKING_CALLS
                    .iter()
                    .any(|(m, f)| module == m && (*f == "*" || function == f));
                if blocking {
                    self.lints.report(
                        call.func.span(),
                        "blocking_in_render",
                        format!(
                            "`{module}::{function}` waits from `render`, on the main thread: run it \
                             in `cx.background_spawn` and keep the answer"
                        ),
                    );
                }
            }
        }
        visit::visit_expr_call(self, call);
    }
}

/// Whether `path` holds only tests.
fn is_test_file(path: &str) -> bool {
    path.contains("/e2e/") || path.contains("/tests/") || path.ends_with("/tests.rs")
}

/// Every finding in `repo`'s app sources, as `path:line: lint: message`: `cargo xtask lints`.
pub fn check(repo: &Path) -> Result {
    let mut files = Vec::new();
    for dir in APP_SOURCES {
        crate::util::sources_under(repo, dir, &mut files)?;
    }
    let mut found = Vec::new();
    for (path, text) in files.iter().filter(|(path, _)| !is_test_file(path)) {
        let findings = lint(text).map_err(|cause| format!("{path}: {cause}"))?;
        found.extend(findings.into_iter().map(|finding| {
            format!(
                "{path}:{}: {}: {}",
                finding.line, finding.lint, finding.message
            )
        }));
    }
    if found.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} GPUI mistake{} (tooling/xtask/src/gpui_lints.rs):\n{}",
        found.len(),
        if found.len() == 1 { "" } else { "s" },
        found.join("\n")
    ))
}

/// Whether a change to `file` can add a finding: an app source.
pub fn reads(file: &str) -> bool {
    file.ends_with(".rs") && APP_SOURCES.iter().any(|dir| file.starts_with(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lints_of(text: &str) -> Vec<(&'static str, usize)> {
        lint(text)
            .expect("the snippet parses")
            .into_iter()
            .map(|finding| (finding.lint, finding.line))
            .collect()
    }

    #[test]
    fn notify_and_update_in_render_are_found_outside_its_closures() {
        let text = r#"
impl Render for Board {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        cx.notify();
        self.clock.update(cx, |clock, cx| clock.tick(cx));
        let later = cx.listener(|this, _, _, cx| {
            this.flipped = true;
            cx.notify();
            this.clock.update(cx, |clock, _| clock.stop());
        });
        cx.spawn(async move |this, cx| { this.update(cx, |_, cx| cx.notify()).ok(); }).detach();
        div().on_click(later)
    }
}
"#;
        assert_eq!(
            lints_of(text),
            [("notify_in_render", 4), ("update_in_render", 5)]
        );
    }

    #[test]
    fn a_render_once_is_a_view_too_and_other_methods_are_not() {
        let text = r#"
impl RenderOnce for Banner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let shown = std::fs::read_to_string("x");
        let found = Path::new("y").is_file();
        div()
    }
}
impl Board {
    fn flip(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        let _ = std::fs::read("x");
    }
}
impl Render for Board {
    fn layout(&mut self, cx: &mut Context<Self>) { cx.notify(); }
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        std::thread::sleep(Duration::from_millis(1));
        let program = Command::new("stockfish");
        hasher.update(bytes);
        div()
    }
}
"#;
        assert_eq!(
            lints_of(text),
            [
                ("blocking_in_render", 4),
                ("blocking_in_render", 5),
                ("blocking_in_render", 18),
                ("blocking_in_render", 19),
            ]
        );
    }

    #[test]
    fn an_entity_updated_inside_its_own_update_is_found() {
        let text = r#"
fn sync(board: &Entity<Board>, other: &Entity<Board>, cx: &mut App) {
    board.update(cx, |this, cx| {
        other.update(cx, |_, _| {});
        board.update(cx, |_, _| {});
        this.clock.update(cx, |_, _| {});
        cx.spawn(async move |cx| board.update(cx, |_, _| {})).detach();
    });
    self.page.update(cx, |page, cx| self . page.update_in(cx, |_, _, _| {}));
}
"#;
        assert_eq!(
            lints_of(text),
            [("reentrant_update", 5), ("reentrant_update", 9)]
        );
    }

    #[test]
    fn tests_and_lines_allowed_with_a_reason_are_left_alone() {
        let text = r#"
impl Render for Board {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // gpui-lints: allow(blocking_in_render) a fixed file the system keeps in memory
        let theme = std::fs::read("x");
        // gpui-lints: allow(update_in_render) another lint's reason is none for this one
        cx.notify();
        div()
    }
}
#[cfg(test)]
mod tests {
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            cx.notify();
            std::fs::read("x");
        }
    }
}
#[gpui_kit::test]
fn a_test(cx: &mut TestAppContext) {
    board.update(cx, |_, cx| board.update(cx, |_, _| {}));
}
"#;
        assert_eq!(lints_of(text), [("notify_in_render", 7)]);
    }

    #[test]
    fn a_module_is_read_unless_it_is_a_test_module_and_a_feature_is_no_test() {
        let text = r#"
mod views {
    impl Render for Board {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            cx.notify();
        }
    }
}
#[cfg(feature = "bench")]
impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        cx.notify();
    }
}
"#;
        assert_eq!(
            lints_of(text),
            [("notify_in_render", 5), ("notify_in_render", 12)]
        );
    }

    #[test]
    fn an_update_in_a_closure_run_inside_the_update_is_reentrant_too() {
        let text = r#"
fn sync(board: &Entity<Board>, cx: &mut App) {
    board.update(cx, |_, cx| {
        moves.iter().for_each(|_| board.update(cx, |_, _| {}));
    });
}
"#;
        assert_eq!(lints_of(text), [("reentrant_update", 4)]);
    }

    #[test]
    fn test_files_are_known_by_their_path() {
        assert!(is_test_file("apps/gpui/src/e2e/shell.rs"));
        assert!(is_test_file("crates/kchess-core/tests/core/lichess.rs"));
        assert!(is_test_file("crates/kchess-ui/src/widgets/tests.rs"));
        assert!(!is_test_file("crates/kchess-ui/src/widgets/panel.rs"));
    }

    /// A whole tree: a mistake in an app source is reported with its path and line, and the same
    /// mistake in a test file is not.
    #[test]
    fn a_tree_is_checked_but_its_tests() {
        let repo = tempfile::tempdir().expect("a scratch tree");
        for dir in APP_SOURCES {
            std::fs::create_dir_all(repo.path().join(dir)).expect("a source folder");
        }
        let view = "impl Render for V {\n    fn render(&mut self, cx: &mut Context<Self>) {\n        cx.notify();\n    }\n}\n";
        std::fs::create_dir_all(repo.path().join("apps/gpui/src/e2e")).expect("e2e");
        std::fs::write(repo.path().join("apps/gpui/src/e2e/probe.rs"), view).expect("a test");
        assert_eq!(check(repo.path()), Ok(()));
        std::fs::write(repo.path().join("crates/kchess-ui/src/view.rs"), view).expect("a view");
        let found = check(repo.path()).expect_err("the view's notify");
        assert!(
            found.contains("1 GPUI mistake ")
                && found.contains("crates/kchess-ui/src/view.rs:3: notify_in_render: "),
            "{found}"
        );
    }

    /// The app keeps every rule: a finding fails here with its file and line.
    #[test]
    fn the_app_keeps_the_gpui_rules() {
        check(&crate::util::root()).unwrap_or_else(|found| panic!("{found}"));
    }

    #[test]
    fn only_app_sources_are_read() {
        assert!(reads("apps/gpui/src/shell.rs"));
        assert!(reads("crates/kchess-board/src/board/mod.rs"));
        assert!(!reads("crates/kchess-core/src/core.rs"));
        assert!(!reads("apps/gpui/Cargo.toml"));
    }
}
