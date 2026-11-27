//! ChatGPT in the app: the connection, as the core keeps it, and `Explainer`, the one block every
//! page explains with. It shows nothing until the person asks, and stays out of the way of anyone
//! who has not connected a plan.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::AiModelName;
use kchess_contracts::types::{
    AiConnection, AiEffort, AiExplanation, AiModelList, AiProblem, AiRequest,
};
use shakmaty::Color;

use crate::core::AppCore;
use crate::theme::{Palette, radii};
use kchess_ui::icon;

/// Where a person manages what apps may spend of their plan.
pub const USAGE_SETTINGS: &str = "https://chatgpt.com/#settings";

/// The connection, and the models it may use.
pub struct Ai {
    pub status: AiConnection,
    /// Slug and display name.
    pub models: Vec<(String, String)>,
    pub busy: bool,
}

#[derive(Clone)]
pub struct AiHandle(pub Entity<Ai>);

impl Global for AiHandle {}

pub fn init(cx: &mut App) {
    let ai = cx.new(|_| Ai {
        status: AiConnection::default(),
        models: Vec::new(),
        busy: false,
    });
    cx.set_global(AiHandle(ai.clone()));
    ai.update(cx, |ai, cx| ai.refresh(cx));
}

pub fn handle(cx: &App) -> Entity<Ai> {
    cx.global::<AiHandle>().0.clone()
}

impl Ai {
    pub fn connected(&self) -> bool {
        self.status.connected
    }

    fn adopt(&mut self, status: AiConnection, cx: &mut Context<Self>) {
        let connected = status.connected;
        self.status = status;
        if connected {
            self.load_models(cx);
        } else {
            self.models.clear();
        }
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let read = cx.global::<AppCore>().request(calls::AiStatus {});
        cx.spawn(async move |this, cx| {
            let Some(status) = read.await.warn_on_err() else {
                return;
            };
            this.update(cx, |this, cx| this.adopt(status, cx)).ok();
        })
        .detach();
    }

    fn load_models(&mut self, cx: &mut Context<Self>) {
        let read = cx.global::<AppCore>().request(calls::AiModels {});
        cx.spawn(async move |this, cx| {
            let Some(answer) = read.await.warn_on_err() else {
                return;
            };
            let models = match answer {
                AiModelList::Listed { models } => models,
                AiModelList::Refused { .. } => Vec::new(),
            };
            this.update(cx, |this, cx| {
                this.models = models
                    .into_iter()
                    .map(|model| (model.slug, model.name))
                    .collect();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Signs in through the browser. The first time, says what it means for the plan.
    pub fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let first = !self.status.registered;
        let connected = cx.global::<AppCore>().request(calls::AiConnect {});
        cx.spawn_in(window, async move |this, cx| {
            let result = connected.await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                cx.notify();
                match result {
                    Ok(status) => {
                        this.adopt(status, cx);
                        if first {
                            welcome(window, cx);
                        }
                    }
                    Err(cause) => {
                        window.push_notification(kchess_app::loading::failure("ai", &cause), cx)
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let done = cx.global::<AppCore>().request(calls::AiDisconnect {});
        cx.spawn(async move |this, cx| {
            let Some(status) = done.await.warn_on_err() else {
                return;
            };
            this.update(cx, |this, cx| this.adopt(status, cx)).ok();
        })
        .detach();
    }

    /// The model and effort every AI feature uses.
    pub fn choose(&mut self, model: Option<String>, effort: AiEffort, cx: &mut Context<Self>) {
        let saved = cx.global::<AppCore>().request(calls::AiSetOptions {
            model: model.map(AiModelName),
            effort,
        });
        cx.spawn(async move |this, cx| {
            let Some(status) = saved.await.warn_on_err() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.status = status;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn welcome(window: &mut Window, cx: &mut App) {
    crate::widgets::open_alert_dialog(window, cx, |dialog: AlertDialog, _, _| {
        dialog
            .title("Using your ChatGPT plan")
            .description(
                "Explanations you ask for use your ChatGPT plan. Manage usage in ChatGPT settings.",
            )
            .ok_text("Got it")
            .ok_variant(ButtonVariant::Primary)
    });
}

/// Runs when an answer is shown, not before: a failed request costs the player nothing.
pub type OnAnswer = std::rc::Rc<dyn Fn(&mut App)>;

/// How an explanation stands.
#[derive(Clone, PartialEq)]
enum Phase {
    Asking,
    Said(SharedString),
    Refused(AiProblem),
    /// A failure's own message.
    Failed(String),
}

/// One explanation at a time: what was asked and how it stands. A page keeps one and hands
/// `explain_block` the request of what it shows, so moving on simply shows another request.
#[derive(Default)]
pub struct Explainer {
    request: Option<AiRequest>,
    phase: Option<Phase>,
    asked: u64,
}

impl Explainer {
    pub fn ask(&mut self, request: AiRequest, on_answer: Option<OnAnswer>, cx: &mut Context<Self>) {
        self.asked += 1;
        let asked = self.asked;
        self.request = Some(request.clone());
        self.phase = Some(Phase::Asking);
        cx.notify();
        let answer = cx.global::<AppCore>().request(calls::AiExplain { request });
        cx.spawn(async move |this, cx| {
            let result = answer.await;
            this.update(cx, |this, cx| {
                if this.asked != asked {
                    return;
                }
                this.phase = Some(match result {
                    Ok(AiExplanation::Told { text }) => {
                        if let Some(on_answer) = &on_answer {
                            on_answer(cx);
                        }
                        Phase::Said(text.into())
                    }
                    Ok(AiExplanation::Refused { problem }) => Phase::Refused(problem),
                    Err(cause) => Phase::Failed(kchess_app::loading::failure("ai", &cause)),
                });
                cx.notify();
                handle(cx).update(cx, |ai, cx| ai.refresh(cx));
            })
            .ok();
        })
        .detach();
    }
}

/// What a refusal says.
fn refusal_text(problem: AiProblem) -> &'static str {
    match problem {
        AiProblem::Connect => "Sign in to ChatGPT again.",
        AiProblem::Limit => "ChatGPT usage limit reached.",
        AiProblem::Ineligible => "This ChatGPT plan can't be used here.",
        AiProblem::Unsupported => "This model can't answer here.",
        AiProblem::Unverified => "No reliable explanation for this one.",
        AiProblem::Unavailable => "ChatGPT is unavailable.",
    }
}

fn manage_usage(id: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .label("Manage usage")
        .on_click(|_, _, cx| cx.open_url(USAGE_SETTINGS))
}

/// The lines an explanation was given, each from `request.fen`: the better line first, as the
/// moves a reply names alone are most often the ones it recommends.
fn given_lines(request: &AiRequest) -> Vec<Vec<String>> {
    let played = request
        .played
        .iter()
        .chain(&request.refutation)
        .cloned()
        .collect();
    vec![request.best.clone(), request.moves.clone(), played]
}

/// An answer's text, each move it names from `request`'s lines showing the position it reaches
/// on hover, seen from `orientation`.
fn said(text: &str, request: &AiRequest, orientation: Color, palette: &Palette) -> AnyElement {
    let named = kchess_domain::coach::named_moves(text, &request.fen, &given_lines(request));
    if named.is_empty() {
        return div().child(text.to_string()).into_any_element();
    }
    let variant = kchess_domain::variant::Variant::Standard;
    let mut named = named.into_iter().enumerate().peekable();
    let mut words: Vec<AnyElement> = Vec::new();
    let mut at = 0;
    // Word by word, so a move can be an element of its own and the text still wraps.
    for word in text.split_inclusive(char::is_whitespace) {
        let end = at + word.len();
        let mut parts: Vec<AnyElement> = Vec::new();
        let mut from = at;
        while let Some((index, move_named)) = named.next_if(|(_, named)| named.range.end <= end) {
            let range = move_named.range.clone();
            if range.start > from {
                parts.push(text[from..range.start].to_string().into_any_element());
            }
            parts.push(
                div()
                    .id(("ai-move", index))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.toned)
                    .tooltip(crate::board::preview_tip(
                        move_named.fen,
                        move_named.uci,
                        variant,
                        orientation,
                    ))
                    .child(text[range.clone()].to_string())
                    .into_any_element(),
            );
            from = range.end;
        }
        if from < end {
            parts.push(text[from..end].to_string().into_any_element());
        }
        words.push(h_flex().flex_none().children(parts).into_any_element());
        at = end;
    }
    h_flex()
        .flex_wrap()
        .items_baseline()
        .children(words)
        .into_any_element()
}

/// The explaining block for `request`: the button, then the answer, its moves previewed from
/// `orientation`. `id` keeps pages apart; with no plan connected it is nothing at all, and
/// `on_answer` runs once an answer arrives.
#[allow(clippy::too_many_arguments)]
pub fn explain_block(
    id: &'static str,
    label: &'static str,
    explainer: &Entity<Explainer>,
    request: AiRequest,
    orientation: Color,
    on_answer: Option<OnAnswer>,
    cx: &App,
    palette: &Palette,
) -> Option<AnyElement> {
    let ai = handle(cx);
    let state = explainer.read(cx);
    let phase = state
        .phase
        .clone()
        .filter(|_| state.request.as_ref() == Some(&request));
    if !ai.read(cx).connected() && phase.is_none() {
        return None;
    }
    let ask = {
        let explainer = explainer.clone();
        let request = request.clone();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            explainer.update(cx, |explainer, cx| {
                explainer.ask(request.clone(), on_answer.clone(), cx);
            });
        }
    };
    let button = |text: &'static str| {
        Button::new(SharedString::from(format!("{id}-explain")))
            .outline()
            .w_full()
            .icon(icon("lightbulb"))
            .label(text)
    };
    let muted = palette.text_muted;
    let panel = |content: AnyElement| {
        v_flex()
            .gap(px(6.))
            .p(px(10.))
            .rounded(radii::BOX)
            .bg(palette.accented)
            .text_size(px(13.))
            .child(content)
    };
    Some(match phase {
        None => button(label).on_click(ask).into_any_element(),
        Some(Phase::Asking) => button("Explaining…").disabled(true).into_any_element(),
        Some(Phase::Said(text)) => panel(said(&text, &request, orientation, palette))
            .child(
                h_flex()
                    .gap(px(8.))
                    .justify_between()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child("Using ChatGPT plan")
                    .child(manage_usage("ai-manage-usage")),
            )
            .into_any_element(),
        Some(Phase::Refused(problem)) => {
            let action = match problem {
                AiProblem::Connect => Some(
                    Button::new("ai-connect-again")
                        .outline()
                        .small()
                        .label("Continue with ChatGPT")
                        .on_click(move |_, window, cx| {
                            ai.update(cx, |ai, cx| ai.connect(window, cx));
                        })
                        .into_any_element(),
                ),
                AiProblem::Limit => Some(manage_usage("ai-limit-usage").into_any_element()),
                // Try again goes beside what went wrong, as on every card that failed to load.
                AiProblem::Unavailable => {
                    return Some(
                        panel(
                            crate::widgets::retry_line(
                                "ai-retry",
                                refusal_text(problem),
                                palette.text,
                                ask,
                            )
                            .into_any_element(),
                        )
                        .into_any_element(),
                    );
                }
                _ => None,
            };
            panel(div().child(refusal_text(problem)).into_any_element())
                .children(action)
                .into_any_element()
        }
        Some(Phase::Failed(message)) => {
            panel(div().child(message).into_any_element()).into_any_element()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::given_lines;
    use kchess_contracts::types::{AiKind, AiRequest};

    /// An explanation's lines start where it does: the better line first, the puzzle's
    /// solution, then the move played followed by its answer.
    #[test]
    fn an_explanation_s_lines_put_the_better_one_first() {
        let request = AiRequest {
            kind: AiKind::Move,
            fen: String::new(),
            played: Some("e2e4".into()),
            moves: vec!["d2d4".into()],
            best: vec!["g1f3".into(), "d7d5".into()],
            refutation: vec!["e7e5".into()],
            mate_before: None,
            mate_after: None,
        };
        assert_eq!(
            given_lines(&request),
            [
                vec!["g1f3".to_string(), "d7d5".into()],
                vec!["d2d4".into()],
                vec!["e2e4".into(), "e7e5".into()],
            ]
        );
    }

    /// A refusal says what went wrong and leaves its action to its button: "ChatGPT is
    /// unavailable. Try again." stood over a Try again.
    #[test]
    fn a_refusal_leaves_its_action_to_its_button() {
        use kchess_contracts::types::AiProblem;
        for (problem, action) in [
            (AiProblem::Connect, "Continue with ChatGPT"),
            (AiProblem::Limit, "Manage usage"),
            (AiProblem::Unavailable, "Try again"),
        ] {
            assert!(!super::refusal_text(problem).contains(action), "{action}");
        }
    }

    fn started_ai(cx: &mut gpui_kit::TestAppContext) -> crate::testing::Started {
        let mut app = crate::testing::Started::new(cx);
        app.with_window(|_, cx| super::init(cx));
        app
    }

    /// An explanation the core cannot give (a position it cannot read) shows its message and logs it.
    #[gpui_kit::test]
    fn an_explanation_the_core_refuses_is_shown_and_logged(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = started_ai(cx);
        let before = kchess_app::tests::logged_in("ai");
        let explainer = app.with_window(|_, cx| {
            let explainer = cx.new(|_| super::Explainer::default());
            explainer.update(cx, |explainer, cx| {
                explainer.ask(
                    AiRequest {
                        kind: AiKind::Nudge,
                        fen: "not a position".into(),
                        played: None,
                        moves: Vec::new(),
                        best: Vec::new(),
                        refutation: Vec::new(),
                        mate_before: None,
                        mate_after: None,
                    },
                    None,
                    cx,
                );
            });
            explainer
        });
        app.wait_for("the failure shown", |_, cx| {
            matches!(explainer.read(cx).phase, Some(super::Phase::Failed(_)))
        });
        assert!(kchess_app::tests::logged_in("ai") > before);
    }

    /// Signing in that the core cannot start ends the busy state and logs why.
    #[gpui_kit::test]
    fn a_sign_in_that_fails_is_logged_and_ends(cx: &mut gpui_kit::TestAppContext) {
        let mut app = started_ai(cx);
        let before = kchess_app::tests::logged_in("ai");
        let ai = app.with_window(|window, cx| {
            let ai = super::handle(cx);
            ai.update(cx, |ai, cx| ai.connect(window, cx));
            assert!(ai.read(cx).busy);
            ai
        });
        app.wait_for("the failure logged", |_, _| {
            kchess_app::tests::logged_in("ai") > before
        });
        app.wait_for("the sign-in to end", |_, cx| !ai.read(cx).busy);
    }
}
