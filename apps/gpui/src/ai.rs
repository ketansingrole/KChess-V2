//! ChatGPT in the app: the connection, as the core keeps it, and `Explainer`, the one block every
//! page explains with. It shows nothing until the person asks, and stays out of the way of anyone
//! who has not connected a plan.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::AiModelName;
use kchess_contracts::types::{
    AiConnection, AiEffort, AiExplanation, AiModelList, AiProblem, AiRequest,
};

use crate::assets::icon;
use crate::core::AppCore;
use crate::theme::{Palette, radii};

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
            let Ok(status) = read.await else { return };
            let _ = this.update(cx, |this, cx| this.adopt(status, cx));
        })
        .detach();
    }

    fn load_models(&mut self, cx: &mut Context<Self>) {
        let read = cx.global::<AppCore>().request(calls::AiModels {});
        cx.spawn(async move |this, cx| {
            let Ok(answer) = read.await else { return };
            let models = match answer {
                AiModelList::Listed { models } => models,
                AiModelList::Refused { .. } => Vec::new(),
            };
            let _ = this.update(cx, |this, cx| {
                this.models = models
                    .into_iter()
                    .map(|model| (model.slug, model.name))
                    .collect();
                cx.notify();
            });
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
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                cx.notify();
                match result {
                    Ok(status) => {
                        this.adopt(status, cx);
                        if first {
                            welcome(window, cx);
                        }
                    }
                    Err(cause) => window.push_notification(cause.message, cx),
                }
            });
        })
        .detach();
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let done = cx.global::<AppCore>().request(calls::AiDisconnect {});
        cx.spawn(async move |this, cx| {
            let Ok(status) = done.await else { return };
            let _ = this.update(cx, |this, cx| this.adopt(status, cx));
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
            let Ok(status) = saved.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.status = status;
                cx.notify();
            });
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
pub struct Explainer {
    request: Option<AiRequest>,
    phase: Option<Phase>,
    asked: u64,
}

impl Explainer {
    pub fn new() -> Explainer {
        Explainer {
            request: None,
            phase: None,
            asked: 0,
        }
    }

    pub fn ask(&mut self, request: AiRequest, on_answer: Option<OnAnswer>, cx: &mut Context<Self>) {
        self.asked += 1;
        let asked = self.asked;
        self.request = Some(request.clone());
        self.phase = Some(Phase::Asking);
        cx.notify();
        let answer = cx.global::<AppCore>().request(calls::AiExplain { request });
        cx.spawn(async move |this, cx| {
            let result = answer.await;
            let _ = this.update(cx, |this, cx| {
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
                    Err(cause) => Phase::Failed(cause.message),
                });
                cx.notify();
                handle(cx).update(cx, |ai, cx| ai.refresh(cx));
            });
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
        AiProblem::Unavailable => "ChatGPT is unavailable. Try again.",
    }
}

fn manage_usage(id: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .label("Manage usage")
        .on_click(|_, _, cx| cx.open_url(USAGE_SETTINGS))
}

/// The explaining block for `request`: the button, then the answer. `id` keeps pages apart; with
/// no plan connected it is nothing at all, and `on_answer` runs once an answer arrives.
pub fn explain_block(
    id: &'static str,
    label: &'static str,
    explainer: &Entity<Explainer>,
    request: AiRequest,
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
            .rounded(radii::MD)
            .bg(palette.accented)
            .text_size(px(13.))
            .child(content)
    };
    Some(match phase {
        None => button(label).on_click(ask).into_any_element(),
        Some(Phase::Asking) => button("Explaining…").disabled(true).into_any_element(),
        Some(Phase::Said(text)) => panel(div().child(text).into_any_element())
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
                AiProblem::Unavailable => Some(
                    Button::new("ai-retry")
                        .outline()
                        .small()
                        .label("Try again")
                        .on_click(ask)
                        .into_any_element(),
                ),
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
