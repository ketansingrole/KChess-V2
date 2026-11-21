//! Appearance: light or dark, the app's theme, and the pieces and boards, each shown as a
//! preview to pick.

use super::*;

/// The inner width of a picker of `width`: its padding and border.
fn picture_width(width: Pixels) -> Pixels {
    width - px(12.)
}

fn piece_preview(set: &str, width: Pixels) -> impl IntoElement {
    let side = picture_width(width) / 2.;
    // Each square takes the preview's outer corners: GPUI clips to rectangles, not rounded ones.
    let square = |piece: Piece, color: u32, first: bool| {
        crate::widgets::bar_segment(div(), (first, !first), radii::SM)
            .size(side)
            .bg(rgb(color))
            .child(
                img(SharedString::from(looks::piece_path(set, piece)))
                    .size(side)
                    .object_fit(ObjectFit::Cover),
            )
    };
    h_flex()
        .rounded(radii::SM)
        .overflow_hidden()
        .child(square(
            Piece {
                color: Color::White,
                role: Role::Knight,
            },
            0xf0d9b5,
            true,
        ))
        .child(square(
            Piece {
                color: Color::Black,
                role: Role::Queen,
            },
            0xb58863,
            false,
        ))
}

fn board_swatch(surface: &looks::Surface, width: Pixels) -> impl IntoElement {
    div()
        .relative()
        .size(picture_width(width))
        .child(looks::surface(surface, radii::SM))
}

/// The theme's background, a raised card, its text and primary colors. A scale
/// model of a page, so its corners are scaled down with it rather than taken from `radii`.
fn theme_preview(theme: &str, custom: &[AppTheme], dark: bool, width: Pixels) -> impl IntoElement {
    let colors = crate::theme::colors(theme, custom, dark);
    let color = |name: &str| {
        colors
            .get(name)
            .and_then(|value| value.as_str())
            .map(crate::theme::hex)
            .unwrap_or_default()
    };
    let width = picture_width(width);
    let card_width = (width - px(14.)) * 0.7;
    h_flex()
        .w(width)
        .h(width * 10. / 16.)
        .items_end()
        .p(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(color("--ui-border"))
        .bg(color("--ui-bg"))
        .child(
            v_flex()
                .w(card_width)
                .gap(px(4.))
                .p(px(4.))
                .rounded(px(4.))
                .bg(color("--ui-bg-elevated"))
                .child(
                    div()
                        .h(px(4.))
                        .w((card_width - px(10.)) * 0.8)
                        .rounded(px(2.))
                        .bg(color("--ui-text").opacity(0.75)),
                )
                .child(
                    div()
                        .h(px(4.))
                        .w((card_width - px(10.)) * 0.45)
                        .rounded(px(2.))
                        .bg(color("--ui-primary")),
                ),
        )
}

/// The default look, the presets, then the user's own (replacing a preset's id).
pub(super) fn theme_list(custom: &[AppTheme]) -> Vec<AppTheme> {
    let mut themes = vec![AppTheme {
        id: "kchess".into(),
        name: "KChess".into(),
        light: None,
        dark: None,
        custom: None,
    }];
    themes.extend(
        serde_json::from_value::<Vec<AppTheme>>(kchess_domain::theme::presets().clone())
            .unwrap_or_default(),
    );
    for theme in custom {
        let theme = AppTheme {
            custom: Some(true),
            ..theme.clone()
        };
        match themes.iter_mut().find(|known| known.id == theme.id) {
            Some(known) => *known = theme,
            None => themes.push(theme),
        }
    }
    themes
}

fn appearance_items() -> [(Appearance, &'static str, Option<Icon>); 3] {
    APPEARANCES.map(|(appearance, label, glyph)| (appearance, label, Some(icon(glyph))))
}

impl SettingsPage {
    pub(super) fn appearance(
        &self,
        settings: &Settings,
        window: &Window,
        palette: &Palette,
        cx: &App,
    ) -> Div {
        let frontend = &settings.frontend;
        let width = self.inner_width(window);
        let (count, item) = columns(width, OPTION_MIN, OPTION_GAP);
        let pieces = looks::piece_sets()
            .iter()
            .map(|set| {
                let id = set.id.clone();
                option(
                    SharedString::from(format!("piece-set-{}", set.id)),
                    item,
                    frontend.piece_set == set.id,
                    piece_preview(&set.id, item),
                    set.name.clone(),
                    None,
                    move |cx| {
                        let id = id.clone();
                        crate::settings::change(cx, move |s| s.frontend.piece_set = id);
                    },
                    palette,
                )
                .into_any_element()
            })
            .collect();
        let boards = looks::boards()
            .iter()
            .map(|board| {
                let id = board.id.clone();
                option(
                    SharedString::from(format!("board-{}", board.id)),
                    item,
                    frontend.board_theme == board.id,
                    board_swatch(&board.surface, item),
                    board.name.clone(),
                    None,
                    move |cx| {
                        let id = id.clone();
                        crate::settings::change(cx, move |s| s.frontend.board_theme = id);
                    },
                    palette,
                )
                .into_any_element()
            })
            .collect();
        let appearances = APPEARANCES.map(|(value, name, _)| (value, name));
        self.rows()
            .row(
                "Theme",
                None,
                choice(
                    "appearance",
                    &appearance_items(),
                    frontend.appearance,
                    |s, value| s.frontend.appearance = value,
                    palette,
                ),
                Some(defaults::Reset::new("appearance", settings, move |d| {
                    name_of(&appearances, d.frontend.appearance)
                })),
                palette,
            )
            .row(
                "Interface size",
                Some(zoom_hint(cx).into()),
                widgets::setting_field(
                    Select::new(&self.zoom).accessibility_label("Interface size"),
                ),
                None,
                palette,
            )
            .row(
                "Board coordinates",
                None,
                choice(
                    "coordinates",
                    &items(&COORDINATES),
                    frontend.coordinates,
                    |s, value| s.frontend.coordinates = value,
                    palette,
                ),
                Some(defaults::Reset::new("coordinates", settings, |d| {
                    name_of(&COORDINATES, d.frontend.coordinates)
                })),
                palette,
            )
            .row(
                "Piece animation",
                None,
                choice(
                    "animation",
                    &items(&ANIMATIONS),
                    frontend.piece_animation,
                    |s, value| s.frontend.piece_animation = value,
                    palette,
                ),
                Some(defaults::Reset::new("pieceAnimation", settings, |d| {
                    name_of(&ANIMATIONS, d.frontend.piece_animation)
                })),
                palette,
            )
            .row(
                "Piece notation",
                None,
                choice(
                    "notation",
                    &items(&NOTATIONS),
                    frontend.figurine_notation,
                    |s, value| s.frontend.figurine_notation = value,
                    palette,
                ),
                Some(defaults::Reset::new("figurineNotation", settings, |d| {
                    name_of(&NOTATIONS, d.frontend.figurine_notation)
                })),
                palette,
            )
            .stacked(
                "Piece set",
                None,
                grid(count, pieces),
                Some(defaults::Reset::new("pieceSet", settings, |d| {
                    looks::piece_sets()
                        .iter()
                        .find(|set| set.id == d.frontend.piece_set)
                        .map(|set| SharedString::from(set.name.clone()))
                        .unwrap_or_default()
                })),
                palette,
            )
            .stacked(
                "Board theme",
                None,
                grid(count, boards),
                Some(defaults::Reset::new("boardTheme", settings, |d| {
                    looks::boards()
                        .iter()
                        .find(|board| board.id == d.frontend.board_theme)
                        .map(|board| SharedString::from(board.name.clone()))
                        .unwrap_or_default()
                })),
                palette,
            )
            .card(palette)
    }

    pub(super) fn themes(
        &self,
        settings: &Settings,
        custom: &[AppTheme],
        dir: &str,
        problems: &[String],
        window: &Window,
        palette: &Palette,
    ) -> Div {
        let frontend = &settings.frontend;
        let (count, item) = columns(self.inner_width(window), APP_THEME_MIN, OPTION_GAP);
        let themes = theme_list(custom);
        let picker = |dark: bool| {
            let chosen = if dark {
                &frontend.dark_theme
            } else {
                &frontend.light_theme
            };
            let options = themes
                .iter()
                .map(|theme| {
                    let id = theme.id.clone();
                    option(
                        SharedString::from(format!(
                            "{}-theme-{}",
                            if dark { "dark" } else { "light" },
                            theme.id
                        )),
                        item,
                        *chosen == theme.id,
                        theme_preview(&theme.id, custom, dark, item),
                        theme.name.clone(),
                        (theme.custom == Some(true)).then_some("custom"),
                        move |cx| {
                            let id = id.clone();
                            crate::settings::change(cx, move |s| {
                                if dark {
                                    s.frontend.dark_theme = id;
                                } else {
                                    s.frontend.light_theme = id;
                                }
                            });
                        },
                        palette,
                    )
                    .into_any_element()
                })
                .collect();
            grid(count, options)
        };
        let dir = dir.to_string();
        let own = v_flex()
            .gap(px(8.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        Button::new("open-themes")
                            .outline()
                            .icon(icon("folder-open"))
                            .label("Open themes folder")
                            .tooltip(SharedString::from(format!(
                                "Copy _template.json there, edit its colors, then reload. {dir}"
                            )))
                            .on_click(move |_, _, cx| cx.open_with_system(Path::new(&dir))),
                    )
                    .child(
                        Button::new("reload-themes")
                            .outline()
                            .icon(icon("refresh-cw"))
                            .label("Reload themes")
                            .on_click(|_, _, cx| {
                                let handle = cx.global::<SettingsHandle>().0.clone();
                                crate::settings::reload(&handle, cx);
                            }),
                    ),
            )
            .children(problems.iter().map(|problem| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.warning)
                    .child(format!("• {problem}"))
            }));
        let appearances = APPEARANCES.map(|(value, name, _)| (value, name));
        let names: Vec<(String, SharedString)> = themes
            .iter()
            .map(|theme| (theme.id.clone(), theme.name.clone().into()))
            .collect();
        let theme_name = move |id: &str| {
            names
                .iter()
                .find(|(of, _)| of == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_default()
        };
        let dark_name = theme_name.clone();
        self.rows()
            .row(
                "Appearance",
                None,
                choice(
                    "theme-appearance",
                    &appearance_items(),
                    frontend.appearance,
                    |s, value| s.frontend.appearance = value,
                    palette,
                ),
                Some(defaults::Reset::new("appearance", settings, move |d| {
                    name_of(&appearances, d.frontend.appearance)
                })),
                palette,
            )
            .stacked(
                "Light mode theme",
                None,
                picker(false),
                Some(defaults::Reset::new("lightTheme", settings, move |d| {
                    theme_name(&d.frontend.light_theme)
                })),
                palette,
            )
            .stacked(
                "Dark mode theme",
                None,
                picker(true),
                Some(defaults::Reset::new("darkTheme", settings, move |d| {
                    dark_name(&d.frontend.dark_theme)
                })),
                palette,
            )
            .stacked("Your own themes", None, own, None, palette)
            .card(palette)
    }
}
