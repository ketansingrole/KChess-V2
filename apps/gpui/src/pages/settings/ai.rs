//! Settings → AI: connecting a ChatGPT plan, and the model and effort every AI feature uses.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::Select;
use gpui_kit::component::{Disableable as _, IndexPath, Sizable as _, h_flex};
use gpui_kit::*;

use super::{Rows, SettingsPage};
use crate::ai::{self, USAGE_SETTINGS};
use crate::assets::icon;
use crate::theme::Palette;

/// What the pickers show: the models, and the chosen model and effort.
pub(super) type Shown = (Vec<(String, String)>, Option<String>, String);

/// The efforts on offer, as the core names them, with their labels.
pub const EFFORTS: [(&str, &str); 4] = [
    ("auto", "Auto"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
];

impl SettingsPage {
    /// Puts the account's models and the saved choices into the pickers.
    pub(super) fn sync_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ai = ai::handle(cx);
        let (models, model, effort) = {
            let ai = ai.read(cx);
            (
                ai.models.clone(),
                ai.status["model"].as_str().map(str::to_string),
                ai.status["effort"].as_str().unwrap_or("auto").to_string(),
            )
        };
        let shown = (models.clone(), model.clone(), effort.clone());
        if self.ai_shown.as_ref() == Some(&shown) {
            return;
        }
        self.ai_shown = Some(shown);
        let items: Vec<SharedString> = models
            .iter()
            .map(|(_, name)| SharedString::from(name.clone()))
            .collect();
        let row = model
            .and_then(|model| models.iter().position(|(slug, _)| *slug == model))
            .or((!models.is_empty()).then_some(0));
        self.ai_model.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_index(row.map(IndexPath::new), window, cx);
        });
        let row = EFFORTS
            .iter()
            .position(|(key, _)| *key == effort)
            .unwrap_or(0);
        self.ai_effort.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::new(row)), window, cx);
        });
    }

    /// Saves what the pickers show.
    pub(super) fn save_ai(&mut self, cx: &mut Context<Self>) {
        let ai = ai::handle(cx);
        let model_row = self
            .ai_model
            .read(cx)
            .selected_index(cx)
            .map(|index| index.row);
        let effort_row = self
            .ai_effort
            .read(cx)
            .selected_index(cx)
            .map(|index| index.row);
        let model = model_row
            .and_then(|row| ai.read(cx).models.get(row))
            .map(|(slug, _)| slug.clone());
        let effort = EFFORTS[effort_row.unwrap_or(0).min(EFFORTS.len() - 1)]
            .0
            .to_string();
        ai.update(cx, |ai, cx| ai.choose(model, effort, cx));
    }

    pub(super) fn ai_section(&self, cx: &Context<Self>, palette: &Palette) -> Div {
        let handle = ai::handle(cx);
        let ai = handle.read(cx);
        let connected = ai.connected();
        let email = ai.status["email"].as_str().map(str::to_string);
        let refused = ai.status["effortRefused"] == serde_json::json!(true);
        let busy = ai.busy;
        let connect = handle.clone();
        let disconnect = handle.clone();
        let account = if connected {
            h_flex()
                .gap(px(8.))
                .justify_end()
                .child(
                    Button::new("ai-usage")
                        .ghost()
                        .small()
                        .label("Manage usage")
                        .on_click(|_, _, cx| cx.open_url(USAGE_SETTINGS)),
                )
                .child(
                    Button::new("ai-disconnect")
                        .outline()
                        .small()
                        .label("Disconnect")
                        .on_click(move |_, _, cx| {
                            disconnect.update(cx, |ai, cx| ai.disconnect(cx));
                        }),
                )
                .into_any_element()
        } else {
            Button::new("ai-connect")
                .primary()
                .icon(icon("lightbulb"))
                .label("Continue with ChatGPT")
                .disabled(busy)
                .on_click(move |_, window, cx| {
                    connect.update(cx, |ai, cx| ai.connect(window, cx));
                })
                .into_any_element()
        };
        let hint: SharedString = match email {
            Some(email) if connected => email.into(),
            _ if connected => "Connected".into(),
            _ => "Explain mistakes and puzzles with your Plus or Pro plan".into(),
        };
        let rows = Rows::new(self.narrow()).row("ChatGPT", Some(hint), account, palette);
        if !connected {
            return rows.card(palette);
        }
        rows.row(
            "Model",
            None,
            Select::new(&self.ai_model).accessibility_label("Model"),
            palette,
        )
        .row(
            "Effort",
            refused.then(|| "This model chooses its own effort".into()),
            Select::new(&self.ai_effort)
                .accessibility_label("Effort")
                .disabled(refused),
            palette,
        )
        .card(palette)
    }
}
