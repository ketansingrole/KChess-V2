//! ChatGPT in the app: the connection, as the core keeps it, and `Explainer`, the one block every
//! page explains with. It shows nothing until the person asks, and stays out of the way of anyone
//! who has not connected a plan.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::core::AppCore;
use crate::theme::{Palette, radii};

/// Where a person manages what apps may spend of their plan.
pub const USAGE_SETTINGS: &str = "https://chatgpt.com/#settings";

/// The connection, and the models it may use.
pub struct Ai {
    pub status: Value,
    /// Slug and display name.
    pub models: Vec<(String, String)>,
    pub busy: bool,
}

#[derive(Clone)]
pub struct AiHandle(pub Entity<Ai>);

impl Global for AiHandle {}

pub fn init(cx: &mut App) {
    let ai = cx.new(|_| Ai {
        status: Value::Null,
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
        self.status["connected"] == json!(true)
    }

    fn adopt(&mut self, status: Value, cx: &mut Context<Self>) {
        let connected = status["connected"] == json!(true);
        self.status = status;
        if connected {
            self.load_models(cx);
        } else {
            self.models.clear();
        }
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let read = cx.global::<AppCore>().call("aiStatus", Vec::new());
        cx.spawn(async move |this, cx| {
            let Ok(status) = read.await else { return };
            let _ = this.update(cx, |this, cx| this.adopt(status, cx));
        })
        .detach();
    }

    fn load_models(&mut self, cx: &mut Context<Self>) {
        let read = cx.global::<AppCore>().call("aiModels", Vec::new());
        cx.spawn(async move |this, cx| {
            let Ok(answer) = read.await else { return };
            let models = answer["models"].as_array().cloned().unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.models = models
                    .iter()
                    .filter_map(|model| {
                        let slug = model["slug"].as_str()?.to_string();
                        let name = model["name"].as_str().unwrap_or(&slug).to_string();
                        Some((slug, name))
                    })
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
        let first = self.status["registered"] != json!(true);
        let connected = cx.global::<AppCore>().call("aiConnect", Vec::new());
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
        let done = cx.global::<AppCore>().call("aiDisconnect", Vec::new());
        cx.spawn(async move |this, cx| {
            let Ok(status) = done.await else { return };
            let _ = this.update(cx, |this, cx| this.adopt(status, cx));
        })
        .detach();
    }

    /// The model and effort every AI feature uses.
    pub fn choose(&mut self, model: Option<String>, effort: String, cx: &mut Context<Self>) {
        let saved = cx
            .global::<AppCore>()
            .call("aiSetOptions", vec![json!(model), json!(effort)]);
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
    window.open_alert_dialog(cx, |dialog: AlertDialog, _, _| {
        dialog
            .title("Using your ChatGPT plan")
            .description(
                "Explanations you ask for use your ChatGPT plan. Manage usage in ChatGPT settings.",
            )
            .ok_text("Got it")
            .ok_variant(ButtonVariant::Primary)
    });
}

/// Runs just before a request goes out.
pub type OnAsk = std::rc::Rc<dyn Fn(&mut App)>;

/// How an explanation stands.
#[derive(Clone, PartialEq)]
enum Phase {
    Asking,
    Said(SharedString),
    /// `problem` of a refused answer, or a failure's own message.
    Refused(String),
}

/// One explanation at a time: what was asked and how it stands. A page keeps one and hands
/// `explain_block` the request of what it shows, so moving on simply shows another request.
pub struct Explainer {
    request: Value,
    phase: Option<Phase>,
    asked: u64,
}

impl Explainer {
    pub fn new() -> Explainer {
        Explainer {
            request: Value::Null,
            phase: None,
            asked: 0,
        }
    }

    pub fn ask(&mut self, request: Value, cx: &mut Context<Self>) {
        self.asked += 1;
        let asked = self.asked;
        self.request = request.clone();
        self.phase = Some(Phase::Asking);
        cx.notify();
        let answer = cx.global::<AppCore>().call("aiExplain", vec![request]);
        cx.spawn(async move |this, cx| {
            let result = answer.await;
            let _ = this.update(cx, |this, cx| {
                if this.asked != asked {
                    return;
                }
                this.phase = Some(match result {
                    Ok(answer) => match answer["text"].as_str() {
                        Some(text) => Phase::Said(text.to_string().into()),
                        None => Phase::Refused(
                            answer["problem"]
                                .as_str()
                                .unwrap_or("unavailable")
                                .to_string(),
                        ),
                    },
                    Err(cause) => Phase::Refused(cause.message),
                });
                cx.notify();
                handle(cx).update(cx, |ai, cx| ai.refresh(cx));
            });
        })
        .detach();
    }
}

/// What a refusal says.
fn refusal_text(problem: &str) -> &str {
    match problem {
        "connect" => "Sign in to ChatGPT again.",
        "limit" => "ChatGPT usage limit reached.",
        "ineligible" => "This ChatGPT plan can't be used here.",
        "unsupported" => "This model can't answer here.",
        "unverified" => "No reliable explanation for this one.",
        "unavailable" => "ChatGPT is unavailable. Try again.",
        other => other,
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
/// no plan connected it is nothing at all, and `on_ask` runs just before a request goes out.
pub fn explain_block(
    id: &'static str,
    label: &'static str,
    explainer: &Entity<Explainer>,
    request: Value,
    on_ask: Option<OnAsk>,
    cx: &App,
    palette: &Palette,
) -> Option<AnyElement> {
    let ai = handle(cx);
    let state = explainer.read(cx);
    let phase = state.phase.clone().filter(|_| state.request == request);
    if !ai.read(cx).connected() && phase.is_none() {
        return None;
    }
    let ask = {
        let explainer = explainer.clone();
        let request = request.clone();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            if let Some(on_ask) = &on_ask {
                on_ask(cx);
            }
            explainer.update(cx, |explainer, cx| explainer.ask(request.clone(), cx));
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
            let action = match problem.as_str() {
                "connect" => Some(
                    Button::new("ai-connect-again")
                        .outline()
                        .small()
                        .label("Continue with ChatGPT")
                        .on_click(move |_, window, cx| {
                            ai.update(cx, |ai, cx| ai.connect(window, cx));
                        })
                        .into_any_element(),
                ),
                "limit" => Some(manage_usage("ai-limit-usage").into_any_element()),
                "unavailable" => Some(
                    Button::new("ai-retry")
                        .outline()
                        .small()
                        .label("Try again")
                        .on_click(ask)
                        .into_any_element(),
                ),
                _ => None,
            };
            panel(
                div()
                    .child(refusal_text(&problem).to_string())
                    .into_any_element(),
            )
            .children(action)
            .into_any_element()
        }
    })
}
