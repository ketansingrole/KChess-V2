//! The GPUI mistakes that compile and then fail at run time (AGENTS.md, "Writing GPUI views"),
//! found in the app's sources as Zed's dylint lints find them in its own (`tooling/lints` there):
//! `cargo xtask lints`, and in `check-fast` when an app source changed; CI runs it as a test.
//!
//! Read from the syntax, not the types: Zed's lints run inside rustc on a pinned nightly with its
//! private crates, which would build the whole workspace a second time in CI. A view's `render` is
//! known by its trait (`impl Render for …`, `impl RenderOnce for …`), and the parts it draws
//! through by their name (`fn render_moves`, in any other impl); what a closure or an async block
//! in them does runs later, so it is left alone, as GPUI allows.
//!
//! - `notify_in_render`: `cx.notify()` in `render` asks for another frame every frame.
//! - `update_in_render`: `entity.update(cx, …)` in `render` changes state while drawing it.
//! - `assign_in_render`: `self.field = …` in `render` changes the view's own state while drawing
//!   it: Home once made a new board there whenever a key it formatted every frame changed.
//! - `blocking_in_render`: the disk, a process or a sleep in `render` stops every window.
//! - `reentrant_update`: `x.update(cx, |…| x.update(…))` borrows the entity twice: GPUI panics.
//! - `unlogged_failure`: a failure's message reaches the page (`Err(cause) => this.error =
//!   Some(cause.message)`) but not the log, so a reported failure leaves no trace in the
//!   diagnostics. Show it through `Loading` or `loading::failure`, which log it too.
//! - `dropped_failure`: a core call's answer whose failure is thrown away, neither shown nor
//!   logged (`answer.ok()`, `if let Ok(x) = answer` with no `else`, `let _ = answer.await`, an
//!   `Err(_)` arm): the page keeps what it showed and nobody learns why. Log it
//!   (`.warn_on_err()`, `loading::failure`) or show it through `Loading`.
//! - `small_target`: a GPUI Kit button made `.xsmall()` is 20 px tall, under the 24 px a target
//!   needs (WCAG 2.5.8) and the layout audit holds; it only fails there in a state the audit
//!   draws. Use `.small()`, or `widgets::text_button` (xsmall with a 24 px `min_h`).
//! - `unnamed_button`: a GPUI Kit button with neither `.label` nor `.accessibility_label` has no
//!   name for a screen reader, whatever it draws: text as a child names nothing, and a tooltip
//!   only describes. `widgets::trailing_icon` names one whose icon follows its text.
//! - `loading_grows`: a GPUI Kit button with `.loading(…)` and no `.icon(…)` grows by a spinner
//!   while it is busy, moving what is beside it (Cancel beside Import, a dialog's whole footer).
//!   With an icon, the spinner takes the icon's place: give it one.
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

/// The expressions a macro's body lists (`vec![a, b]`, `format!("{}", x)`), which `syn` leaves
/// as tokens: views build children in `vec![…]`, and a mistake there is the same mistake.
fn macro_exprs(mac: &syn::Macro) -> Vec<syn::Expr> {
    use syn::parse::Parser as _;
    syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
        .parse2(mac.tokens.clone())
        .map(|exprs| exprs.into_iter().collect())
        .unwrap_or_default()
}

fn line(span: Span) -> usize {
    span.start().line
}

/// The whole file: views' `render`s, and every `update` for its reentrancy.
#[derive(Default)]
struct Lints {
    found: Vec<Finding>,
    /// The links of the button chains already judged (`button`), by where their method is.
    judged: std::collections::HashSet<(usize, usize)>,
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
            self.dropped(&item.block);
            visit::visit_item_fn(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !is_test(&item.attrs) {
            self.dropped(&item.block);
            visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if is_test(&item.attrs) {
            return;
        }
        let view = item.trait_.as_ref().and_then(|(path, _)| {
            let last = path.segments.last()?;
            (last.ident == "Render" || last.ident == "RenderOnce").then_some(())
        });
        for inner in &item.items {
            if let syn::ImplItem::Fn(function) = inner {
                let name = function.sig.ident.to_string();
                let draws = match view {
                    Some(()) => name == "render",
                    None => name.starts_with("render_"),
                };
                if draws {
                    let mut render = InRender { lints: self };
                    render.visit_block(&function.block);
                }
            }
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        // `widgets::trailing_icon(button, label, glyph)` names the button it is given.
        if let syn::Expr::Path(path) = &*call.func
            && path
                .path
                .segments
                .last()
                .is_some_and(|last| last.ident == "trailing_icon")
            && let Some(syn::Expr::MethodCall(button)) = call.args.first()
        {
            self.button(button, true);
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.button(call, false);
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

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        for expr in macro_exprs(mac) {
            self.visit_expr(&expr);
        }
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        self.unlogged(&arm.pat, |read| read.visit_expr(&arm.body));
        visit::visit_arm(self, arm);
    }

    fn visit_expr_if(&mut self, branch: &'ast syn::ExprIf) {
        if let syn::Expr::Let(binding) = &*branch.cond {
            self.unlogged(&binding.pat, |read| read.visit_block(&branch.then_branch));
        }
        visit::visit_expr_if(self, branch);
    }
}

impl Lints {
    /// The kit button `call`'s chain builds, judged once, from the whole chain: its shorter tails
    /// are visited after it, and would miss a `min_h` or a `label` that follows. An `.xsmall()`
    /// without a `min_h` that keeps it a 24 px target, no name anywhere in the chain (its
    /// `when` closures too) unless the call it is given to names it (`named`), and a `.loading`
    /// with no icon for its spinner to stand in.
    fn button(&mut self, call: &syn::ExprMethodCall, named: bool) {
        let mut methods = Vec::new();
        let mut link = call;
        let root = loop {
            methods.push(&link.method);
            match &*link.receiver {
                syn::Expr::MethodCall(inner) => link = inner,
                other => break other,
            }
        };
        let is_button = matches!(root, syn::Expr::Call(constructor)
            if matches!(&*constructor.func, syn::Expr::Path(path)
                if path.path.segments.len() >= 2
                    && path.path.segments.iter().rev().nth(1).is_some_and(|s| s.ident == "Button")
                    && path.path.segments.last().is_some_and(|s| s.ident == "new")));
        let mut first = false;
        for method in &methods {
            let start = method.span().start();
            first |= self.judged.insert((start.line, start.column));
        }
        if !is_button || !first {
            return;
        }
        if !methods.iter().any(|method| *method == "min_h") {
            for method in methods.iter().filter(|method| **method == "xsmall") {
                self.report(
                    method.span(),
                    "small_target",
                    "a kit button made `.xsmall()` is 20 px, under the 24 px a target needs: use \
                     `.small()`, or `widgets::text_button` for one that reads as text",
                );
            }
        }
        if let Some(loading) = methods.iter().find(|method| **method == "loading") {
            let mut icon = Calls("icon", false);
            icon.visit_expr_method_call(call);
            if !icon.1 {
                self.report(
                    loading.span(),
                    "loading_grows",
                    "a kit button with `.loading` and no `.icon` grows by its spinner while busy, \
                     moving what is beside it: give it an icon, which the spinner stands in for",
                );
            }
        }
        let mut name = Named(named);
        name.visit_expr_method_call(call);
        if !name.0 {
            self.report(
                root.span(),
                "unnamed_button",
                "a kit button with no `.label` or `.accessibility_label` has no name for a screen \
                 reader (a child's text names nothing): add one, or use `widgets::trailing_icon`",
            );
        }
    }

    /// The core calls' failures `body`, one function's, throws away.
    fn dropped(&mut self, body: &syn::Block) {
        let mut answers = Answers::default();
        answers.visit_block(body);
        for span in answers.found {
            self.report(
                span,
                "dropped_failure",
                "a core call's failure is thrown away, neither shown nor logged: log it \
                 (`.warn_on_err()`, `loading::failure(area, &cause)`) or show it through `Loading`",
            );
        }
    }

    /// A failure bound by `pattern` (`Err(cause)`) whose message `body` reads without logging it.
    fn unlogged(&mut self, pattern: &syn::Pat, body: impl FnOnce(&mut FailureUse<'_>)) {
        let mut bound = Vec::new();
        failures(pattern, &mut bound);
        if bound.is_empty() {
            return;
        }
        let mut read = FailureUse {
            bound: &bound,
            message: None,
            logged: false,
        };
        body(&mut read);
        if let (Some(span), false) = (read.message, read.logged) {
            self.report(
                span,
                "unlogged_failure",
                "a failure's message is shown but not logged: show it through `Loading` \
                 (`load`, `settle`, `failed`) or `loading::failure(area, &cause)`, which log it",
            );
        }
    }
}

/// Whether a button chain names its button anywhere: `.label(…)`, `.accessibility_label(…)`.
struct Named(bool);

impl<'ast> Visit<'ast> for Named {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.0 |= call.method == "label" || call.method == "accessibility_label";
        visit::visit_expr_method_call(self, call);
    }
}

/// Whether a button chain calls the method `.0` anywhere, its `when` closures too.
struct Calls(&'static str, bool);

impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.1 |= call.method == self.0;
        visit::visit_expr_method_call(self, call);
    }
}

/// The names `Err(…)` binds anywhere in `pattern`.
fn failures(pattern: &syn::Pat, bound: &mut Vec<String>) {
    match pattern {
        syn::Pat::TupleStruct(tuple) => {
            let is_err = tuple
                .path
                .segments
                .last()
                .is_some_and(|last| last.ident == "Err");
            for inner in &tuple.elems {
                match inner {
                    syn::Pat::Ident(ident) if is_err => bound.push(ident.ident.to_string()),
                    other => failures(other, bound),
                }
            }
        }
        syn::Pat::Tuple(tuple) => tuple.elems.iter().for_each(|inner| failures(inner, bound)),
        syn::Pat::Or(or) => or.cases.iter().for_each(|inner| failures(inner, bound)),
        syn::Pat::Reference(reference) => failures(&reference.pat, bound),
        syn::Pat::Paren(paren) => failures(&paren.pat, bound),
        // syn 3 keeps an arm's guard in its pattern: `Err(cause) if !cause.aborted`.
        syn::Pat::Guard(guarded) => failures(&guarded.pat, bound),
        _ => {}
    }
}

/// Calls that log a failure: the log itself and the views' wrappers of it (`log`, `log_warn`),
/// `Loading`'s and `loading::failure`.
const LOGGING_CALLS: [&str; 8] = [
    "write",
    "log",
    "log_warn",
    "failed",
    "settle",
    "failure",
    "log_err",
    "warn_on_err",
];

/// Whether `name` is one of the calls that log a failure.
fn is_logging(name: &syn::Ident) -> bool {
    LOGGING_CALLS.iter().any(|logging| name == logging)
}

/// Whether running `node` logs: it calls one of `LOGGING_CALLS`.
fn logs(node: impl FnOnce(&mut Logs)) -> bool {
    let mut found = Logs(false);
    node(&mut found);
    found.0
}

/// Whether a logging call was seen.
struct Logs(bool);

impl<'ast> Visit<'ast> for Logs {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func
            && path
                .path
                .segments
                .last()
                .is_some_and(|last| is_logging(&last.ident))
        {
            self.0 = true;
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.0 |= is_logging(&call.method);
        visit::visit_expr_method_call(self, call);
    }
}

/// A call to the core: `request`, `request_now` or `lookup` with one argument, the call itself
/// (`core.request(calls::Library {})`); HTTP clients' `request(method, url)` take two.
fn is_core_call(call: &syn::ExprMethodCall) -> bool {
    ["request", "request_now", "lookup"]
        .iter()
        .any(|name| call.method == name)
        && call.args.len() == 1
}

/// Whether `expr`'s value comes from a core call: the call itself, awaited or not, a closure's
/// answer (`this.update(cx, |_, cx| core.request(…))`), a branch's or a block's last value, or one
/// wrapped (`Some(core.request(…))`). A value merely computed after a call asked the core
/// (`let rebuilt = { let rated = core.request(…).await…; Some(series(rated)) }`) is not one.
fn asks_core(expr: &syn::Expr) -> bool {
    let tail = |block: &syn::Block| match block.stmts.last() {
        Some(syn::Stmt::Expr(last, None)) => asks_core(last),
        _ => false,
    };
    match expr {
        syn::Expr::MethodCall(call) => {
            is_core_call(call) || asks_core(&call.receiver) || call.args.iter().any(asks_core)
        }
        syn::Expr::Call(call) => call.args.iter().any(asks_core),
        syn::Expr::Closure(closure) => asks_core(&closure.body),
        syn::Expr::Await(awaited) => asks_core(&awaited.base),
        syn::Expr::Paren(paren) => asks_core(&paren.expr),
        syn::Expr::Reference(reference) => asks_core(&reference.expr),
        syn::Expr::Block(block) => tail(&block.block),
        syn::Expr::If(branch) => {
            tail(&branch.then_branch)
                || branch
                    .else_branch
                    .as_ref()
                    .is_some_and(|(_, otherwise)| asks_core(otherwise))
        }
        syn::Expr::Match(matched) => matched.arms.iter().any(|arm| asks_core(&arm.body)),
        _ => false,
    }
}

/// The names a pattern binds.
fn bound_names(pattern: &syn::Pat, names: &mut Vec<String>) {
    struct Names<'a>(&'a mut Vec<String>);
    impl<'ast> Visit<'ast> for Names<'_> {
        fn visit_pat_ident(&mut self, ident: &'ast syn::PatIdent) {
            self.0.push(ident.ident.to_string());
            visit::visit_pat_ident(self, ident);
        }
    }
    Names(names).visit_pat(pattern);
}

/// Whether `pattern` is `Ok(…)`.
fn is_ok_pattern(pattern: &syn::Pat) -> bool {
    matches!(pattern, syn::Pat::TupleStruct(tuple)
        if tuple.path.segments.last().is_some_and(|last| last.ident == "Ok"))
}

/// Whether `pattern` is `Err(_)`: the failure matched and not bound.
fn is_ignored_failure(pattern: &syn::Pat) -> bool {
    matches!(pattern, syn::Pat::TupleStruct(tuple)
        if tuple.path.segments.last().is_some_and(|last| last.ident == "Err")
            && tuple.elems.iter().all(|inner| matches!(inner, syn::Pat::Wild(_))))
}

/// One function's core answers, by the names they are bound to (the call, its awaited answer and
/// what that is bound to next), and where a failure of one is thrown away.
#[derive(Default)]
struct Answers {
    names: Vec<String>,
    found: Vec<Span>,
}

impl Answers {
    /// Whether `expr` is an answer: a name bound to one, awaited or not, or a core call itself.
    fn is_answer(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Await(awaited) => self.is_answer(&awaited.base),
            syn::Expr::Paren(paren) => self.is_answer(&paren.expr),
            syn::Expr::Path(path) => path
                .path
                .get_ident()
                .is_some_and(|ident| self.names.iter().any(|name| ident == name)),
            syn::Expr::MethodCall(call) => is_core_call(call),
            _ => false,
        }
    }

    /// The names `pattern` binds from `init` that hold an answer.
    fn bind(&mut self, pattern: &syn::Pat, init: &syn::Expr) {
        let pattern = match pattern {
            syn::Pat::Type(typed) => &*typed.pat,
            other => other,
        };
        match (pattern, init) {
            (syn::Pat::Tuple(patterns), syn::Expr::Tuple(values)) => {
                for (pattern, value) in patterns.elems.iter().zip(&values.elems) {
                    self.bind(pattern, value);
                }
            }
            (syn::Pat::Wild(_), _) => {}
            _ if self.is_answer(init) || asks_core(init) => {
                bound_names(pattern, &mut self.names);
            }
            _ => {}
        }
    }

    /// Answers passed to a call are its to handle (`loading.settle(turn, area, &answer)`,
    /// `this.show(answer)`): no longer this function's.
    fn hand_over<'a>(&mut self, args: impl IntoIterator<Item = &'a syn::Expr>) {
        for arg in args {
            let arg = match arg {
                syn::Expr::Reference(reference) => &*reference.expr,
                other => other,
            };
            if let syn::Expr::Path(path) = arg
                && let Some(ident) = path.path.get_ident()
            {
                self.names.retain(|name| ident != name);
            }
        }
    }
}

impl<'ast> Visit<'ast> for Answers {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        self.hand_over(&call.args);
        visit::visit_expr_call(self, call);
    }

    fn visit_item(&mut self, _: &'ast syn::Item) {
        // A function inside this one is read as its own.
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Some(init) = &local.init {
            let answer = self.is_answer(&init.expr);
            match (&local.pat, &init.diverge) {
                // `let _ = answer.await`: the failure goes with the rest.
                (syn::Pat::Wild(wild), None) if answer => self.found.push(wild.span()),
                // `let Ok(x) = answer else { return }`: unless the `else` logs.
                (pattern, Some((_, otherwise)))
                    if answer
                        && is_ok_pattern(pattern)
                        && !logs(|log| log.visit_expr(otherwise)) =>
                {
                    self.found.push(pattern.span());
                }
                _ => {}
            }
            self.bind(&local.pat, &init.expr);
        }
        visit::visit_local(self, local);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if self.is_answer(&call.receiver) {
            let method = call.method.to_string();
            let dropped = match method.as_str() {
                "ok" | "unwrap_or" | "unwrap_or_default" => true,
                // `unwrap_or_else(|cause| …)` may log it; `|_|` cannot.
                "unwrap_or_else" => call.args.iter().any(|arg| {
                    matches!(arg, syn::Expr::Closure(closure)
                        if closure.inputs.iter().all(|input| matches!(input, syn::Pat::Wild(_)))
                            && !logs(|log| log.visit_expr(&closure.body)))
                }),
                _ => false,
            };
            if dropped {
                self.found.push(call.method.span());
            }
        }
        self.hand_over(&call.args);
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_if(&mut self, branch: &'ast syn::ExprIf) {
        if branch.else_branch.is_none() {
            // `if let Ok(x) = answer` and `if let Ok(x) = answer && …` with no `else`.
            let mut conditions = vec![&*branch.cond];
            while let Some(condition) = conditions.pop() {
                match condition {
                    syn::Expr::Binary(both) if matches!(both.op, syn::BinOp::And(_)) => {
                        conditions.extend([&*both.left, &*both.right]);
                    }
                    syn::Expr::Let(binding)
                        if is_ok_pattern(&binding.pat) && self.is_answer(&binding.expr) =>
                    {
                        self.found.push(binding.pat.span());
                    }
                    _ => {}
                }
            }
        }
        visit::visit_expr_if(self, branch);
    }

    fn visit_expr_match(&mut self, matched: &'ast syn::ExprMatch) {
        if self.is_answer(&matched.expr) {
            // `Err(cause) if !cause.aborted => …` before it: `Err(_)` is only what that one
            // leaves (a cancelled search).
            let mut sorted = false;
            for arm in &matched.arms {
                if is_ignored_failure(&arm.pat) && !sorted && !logs(|log| log.visit_expr(&arm.body))
                {
                    self.found.push(arm.pat.span());
                }
                let mut bound = Vec::new();
                failures(&arm.pat, &mut bound);
                sorted |= matches!(arm.pat, syn::Pat::Guard(_)) && !bound.is_empty();
            }
        }
        visit::visit_expr_match(self, matched);
    }
}

/// Where a failure's `.message` is read, and whether the same arm logs.
struct FailureUse<'a> {
    bound: &'a [String],
    message: Option<Span>,
    logged: bool,
}

impl<'ast> Visit<'ast> for FailureUse<'_> {
    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if let (syn::Expr::Path(base), syn::Member::Named(member)) = (&*field.base, &field.member)
            && member == "message"
            && base
                .path
                .get_ident()
                .is_some_and(|ident| self.bound.iter().any(|name| ident == name))
        {
            self.message.get_or_insert(member.span());
        }
        visit::visit_expr_field(self, field);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func
            && path
                .path
                .segments
                .last()
                .is_some_and(|last| LOGGING_CALLS.iter().any(|name| last.ident == name))
        {
            self.logged = true;
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if LOGGING_CALLS.iter().any(|name| call.method == name) {
            self.logged = true;
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        // `format!("…{}", cause.message)` reads it as an expression list.
        if let Ok(args) = mac.parse_body_with(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        ) {
            args.iter().for_each(|arg| self.visit_expr(arg));
        }
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

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        for expr in macro_exprs(mac) {
            self.visit_expr(&expr);
        }
    }

    fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
        self.assigned(&assign.left);
        visit::visit_expr_assign(self, assign);
    }

    fn visit_expr_binary(&mut self, binary: &'ast syn::ExprBinary) {
        use syn::BinOp::*;
        let assigns = matches!(
            binary.op,
            AddAssign(_)
                | SubAssign(_)
                | MulAssign(_)
                | DivAssign(_)
                | RemAssign(_)
                | BitXorAssign(_)
                | BitAndAssign(_)
                | BitOrAssign(_)
                | ShlAssign(_)
                | ShrAssign(_)
        );
        if assigns {
            self.assigned(&binary.left);
        }
        visit::visit_expr_binary(self, binary);
    }

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

impl InRender<'_> {
    /// `self.field = …` (or `+=`, or into a field of a field): the view's state changed while
    /// drawing it.
    fn assigned(&mut self, place: &syn::Expr) {
        let mut root = place;
        loop {
            root = match root {
                syn::Expr::Field(field) => &field.base,
                syn::Expr::Paren(paren) => &paren.expr,
                _ => break,
            };
        }
        let of_self = matches!(root, syn::Expr::Path(path) if path.path.is_ident("self"));
        if of_self && !std::ptr::eq(root, place) {
            self.lints.report(
                place.span(),
                "assign_in_render",
                "`render` changes the view's state while drawing it: change it where its inputs \
                 change (the handler, task or subscription that learned of it), and let `render` \
                 read it",
            );
        }
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
    fn a_field_set_in_render_is_found_outside_its_closures() {
        let text = r#"
impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut shown = 0;
        shown = 1;
        self.board = Some(cx.new(|_| ChessBoard::new()));
        self.cache.key += 1;
        if self.board.is_none() {
            (self.opening) = None;
        }
        let later = cx.listener(|this, _, _, cx| {
            this.flipped = true;
            self.flipped = true;
        });
        // gpui-lints: allow(assign_in_render) a tick reads it
        self.faces = [true, false];
        div().on_click(later)
    }
}
"#;
        assert_eq!(
            lints_of(text),
            [
                ("assign_in_render", 6),
                ("assign_in_render", 7),
                ("assign_in_render", 9)
            ]
        );
    }

    #[test]
    fn the_parts_render_draws_through_are_read_as_render() {
        let text = r#"
impl Rush {
    fn render_stage(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let storm = self.stats.update(cx, |stats, cx| stats.storm_panel(cx));
        self.drawn = true;
        div().child(storm).into_any_element()
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.stats.update(cx, |stats, cx| stats.reset(cx));
        self.drawn = false;
        cx.notify();
    }
}
"#;
        assert_eq!(
            lints_of(text),
            [("update_in_render", 4), ("assign_in_render", 5)]
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
    fn an_xsmall_kit_button_is_found_unless_it_keeps_24_px() {
        let text = r#"
fn row(cx: &mut App) -> Div {
    div()
        .child(Button::new("train").ghost().xsmall().label("Train"))
        .child(Button::new("more").xsmall().text().min_h(px(24.)).label("More"))
        .child(Input::new(&field).xsmall())
        .child(gpui_kit::component::button::Button::new(("accept", 1))
            .when(on, |button| button.primary())
            .xsmall()
            .label("Accept"))
        .child(Button::new("go").small().label("Go"))
        .children(vec![Button::new("ok").xsmall().label("OK"), Button::new("no").label("No")])
}
"#;
        assert_eq!(
            lints_of(text),
            [
                ("small_target", 4),
                ("small_target", 9),
                ("small_target", 12)
            ]
        );
    }

    /// A busy Import grew by its spinner and pushed Cancel beside it; one with an icon swaps it.
    #[test]
    fn a_loading_button_without_an_icon_is_found() {
        let text = r#"
fn footer(busy: bool) -> Div {
    div()
        .child(Button::new("import").primary().label("Import").loading(busy))
        .child(Button::new("sync").icon(icon("refresh-cw")).label("Sync").loading(busy))
        .child(Button::new("go").label("Go").when(on, |b| b.icon(icon("go"))).loading(busy))
        .child(Button::new("save").label("Save"))
}
"#;
        assert_eq!(lints_of(text), [("loading_grows", 4)]);
    }

    #[test]
    fn a_kit_button_without_a_name_is_found() {
        let text = r#"
fn row(cx: &mut App) -> Div {
    div()
        .child(Button::new("next").outline().child(h_flex().child("Next")))
        .child(Button::new("flip").icon(icon("flip")).tooltip("Flip"))
        .child(Button::new("play").when(on, |button| button.label("Play")))
        .child(Button::new("go").icon(icon("go")).accessibility_label("Go").small())
        .child(widgets::text_button("all").label("View all"))
        .child(widgets::trailing_icon(Button::new("on").outline(), "Next", "arrow-right"))
}
"#;
        assert_eq!(
            lints_of(text),
            [("unnamed_button", 4), ("unnamed_button", 5)]
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
    fn a_failure_shown_but_not_logged_is_found() {
        let text = r#"
fn answered(this: &mut Page, answer: Result<Report, CoreError>, cx: &mut App) {
    match answer {
        Ok(report) => this.report = Some(report),
        Err(cause) => this.error = Some(cause.message.into()),
    }
    match answer {
        (Err(cause), _) | (_, Err(cause)) => {
            window.push_notification(Notification::error(cause.message), cx)
        }
        _ => {}
    }
    if let Err(failed) = answer {
        this.error = Some(format!("Could not open it: {}", failed.message));
    }
    match answer {
        Ok(Reply::Done(found)) => this.found = found,
        Err(other) => this.other = Some(cause.message),
    }
    match &answer {
        &Err(ref cause) => this.error = Some(cause.message.clone()),
        _ => {}
    }
    match answer {
        (Err(cause)) => this.error = Some(cause.message),
        _ => {}
    }
}
"#;
        assert_eq!(
            lints_of(text),
            [
                ("unlogged_failure", 5),
                ("unlogged_failure", 9),
                ("unlogged_failure", 14),
                ("unlogged_failure", 21),
                ("unlogged_failure", 25)
            ],
            "only the failures bound in each arm count, behind a reference or parentheses too: \
             `other` is not `cause`"
        );
    }

    #[test]
    fn a_failure_shown_and_logged_is_left_alone() {
        let text = r#"
fn answered(this: &mut Page, answer: Result<Report, CoreError>, cx: &mut App) {
    match answer {
        Err(cause) => this.error = Some(loading::failure("page", &cause).into()),
        _ => {}
    }
    match answer {
        Err(cause) => {
            crate::log::write(Level::Warn, "page", &cause.message);
            this.error = Some(cause.message);
        }
        _ => {}
    }
    match answer {
        Err(cause) => this.loading.failed("page", &cause),
        _ => {}
    }
    if let Err(cause) = answer {
        log(&format!("staying connected failed: {}", cause.message));
    }
    match answer {
        Err(_) => this.failed = true,
        Ok(_) => {}
    }
}
"#;
        assert_eq!(lints_of(text), []);
    }

    /// syn 3 keeps a match arm's guard in its pattern: a guarded arm that shows the failure
    /// without logging it went unread from the move to syn 3 until this lint looked inside it.
    #[test]
    fn a_guarded_arm_that_shows_a_failure_is_read_too() {
        let text = r#"
fn answered(this: &mut Page, answer: Result<Report, CoreError>) {
    match answer {
        Err(cause) if this.current => this.error = Some(cause.message),
        _ => {}
    }
}
"#;
        assert_eq!(lints_of(text), [("unlogged_failure", 4)]);
    }

    #[test]
    fn a_core_failure_thrown_away_is_found() {
        let text = r#"
fn load(&mut self, cx: &mut Context<Self>) {
    let status = cx.global::<AppCore>().request(calls::EngineStatus { variant: None });
    let data = core.request(calls::LoadData {});
    let library = this.update(cx, |_, cx| cx.global::<AppCore>().request(calls::Library {}));
    cx.spawn(async move |this, cx| {
        let status = status.await.ok();
        let Ok(data) = data.await else { return };
        let answer = library.await;
        if let Ok(library) = answer {
            this.library = library;
        }
        if let Ok(table) = core.request(calls::Crosstable { a, b }).await
            && table.nb_games > 0
        {
            this.table = Some(table);
        }
        let joined = joined_read.await.unwrap_or_default();
        let themes = bridge.request_now(CustomThemes {}).unwrap_or_else(|_| empty());
        match core.request(calls::Days { name }).await {
            Ok(days) => this.days = Some(days),
            Err(_) => this.days_error = true,
        }
        let _ = core.request(calls::Save { value }).await;
    });
}
"#;
        assert_eq!(
            lints_of(text),
            [
                ("dropped_failure", 7),
                ("dropped_failure", 8),
                ("dropped_failure", 10),
                ("dropped_failure", 13),
                ("dropped_failure", 19),
                ("dropped_failure", 22),
                ("dropped_failure", 24),
            ],
            "`joined_read` was never bound to a core call: only answers count"
        );
    }

    #[test]
    fn a_core_failure_logged_shown_or_handed_on_is_left_alone() {
        let text = r#"
fn load(&mut self, cx: &mut Context<Self>) {
    let status = core.request(calls::EngineStatus { variant: None });
    let page = core.request(calls::GamePage { query });
    let search = core.request(calls::BestMove { fen });
    let rated = core.request(calls::RecentGames { name });
    let client = reqwest::Client::new();
    cx.spawn(async move |this, cx| {
        let status = status.await.warn_on_err();
        let page = page.await;
        if !this.loading.settle(turn, "history", &page) {
            return;
        }
        if let Ok(page) = page {
            this.games = page.games;
        }
        match search.await {
            Ok(best) => this.best = Some(best),
            Err(cause) if !cause.aborted => this.error = Some(failure("engine", &cause)),
            Err(_) => {}
        }
        let rebuilt = if known {
            None
        } else {
            let games = rated.await.warn_on_err().unwrap_or_default();
            Some(series(games))
        };
        let shown = rebuilt.unwrap_or_default();
        let body = client.request(Method::GET, url).send().await.ok();
        if let Ok(answer) = core.request(calls::Profile { name }).await {
            this.profile = Some(answer);
        } else {
            log_warn("no profile");
        }
    });
}
"#;
        assert_eq!(lints_of(text), []);
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
