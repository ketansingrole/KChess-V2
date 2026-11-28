//! The page's parts: an opening's card, the tree of moves, the browser of groups and the detail
//! of the line shown.

use super::*;

impl OpeningsPage {
    pub(super) fn render_card(
        &mut self,
        item: &Entry,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let board = self.preview(&item.fen, true, cx);
        let (open, train) = (item.moves.clone(), item.clone());
        let page = cx.entity().downgrade();
        widgets::StudyCard {
            id: item.name.clone(),
            name: item.name.clone(),
            picture: Some(widgets::CardPicture::Board(board.into())),
            location: item.eco.clone(),
            description: Some(item.san.clone()),
            edited: None,
            rows: 2,
            menu: None,
            found: Vec::new(),
            action: Button::new(SharedString::from(format!("train-{}", item.name)))
                .outline()
                .small()
                .h(px(28.))
                .w_full()
                .icon(icon("graduation-cap"))
                // gpui-lints: allow(icon_per_label) trains the opening; Train on Puzzles starts puzzles
                .label("Train")
                .accessibility_label(format!("Train {}", item.name))
                .on_click(move |_, window, cx| {
                    Self::train(
                        OpeningChoice {
                            name: train.name.clone(),
                            moves: train.moves.clone(),
                        },
                        window,
                        cx,
                    )
                })
                .into_any_element(),
            width,
            on_open: std::rc::Rc::new(move |window, cx| {
                page.update(cx, |page, cx| {
                    page.show_line(Some(open.clone()), window, cx)
                })
                .ok();
            }),
        }
        .into_any_element()
    }

    /// One entry, with its variations below it once opened.
    pub(super) fn render_node(
        &self,
        node: &OpeningNode,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let open = self.expanded.contains(&node.name);
        let name = node.name.clone();
        let toggle = {
            let name = name.clone();
            cx.listener(
                move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                    if !this.expanded.remove(&name) {
                        this.expanded.insert(name.clone());
                    }
                    cx.notify();
                },
            )
        };
        let row = h_flex()
            .gap(px(8.))
            .min_h(px(28.))
            .when(!node.children.is_empty(), |row| {
                row.child(
                    Button::new(SharedString::from(format!("toggle-{name}")))
                        .ghost()
                        .small()
                        .icon(if open {
                            icon("chevron-down")
                        } else {
                            icon("chevron-right")
                        })
                        .accessibility_label(format!(
                            "{} {} variations",
                            if open { "Hide" } else { "Show" },
                            node.name
                        ))
                        .on_click(toggle),
                )
            })
            .when(node.children.is_empty(), |row| {
                row.child(div().w(px(24.)).flex_none())
            })
            .child({
                let link = div()
                    .id(SharedString::from(format!("open-{name}")))
                    .test_support()
                    .role(Role::Link)
                    .keyboard_focus(palette.primary)
                    .aria_label(node.label.clone())
                    .min_w_0()
                    .truncate()
                    .cursor_pointer()
                    .pressable_link()
                    .child(node.label.clone());
                match &node.moves {
                    Some(moves) => {
                        let moves = moves.clone();
                        link.on_click(cx.listener(move |this, _, window, cx| {
                            this.show_line(Some(moves.clone()), window, cx)
                        }))
                    }
                    None => {
                        let name = name.clone();
                        link.on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&name) {
                                this.expanded.insert(name.clone());
                            }
                            cx.notify();
                        }))
                    }
                }
            })
            .children(node.eco.clone().map(|eco| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(eco)
            }));
        v_flex()
            .child(row)
            .when(open, |entry| {
                entry.child(
                    v_flex()
                        .ml(px(14.))
                        .pl(px(12.))
                        .border_l_1()
                        .border_color(palette.border)
                        .children(
                            node.children
                                .iter()
                                .map(|child| self.render_node(child, cx, palette))
                                .collect::<Vec<_>>(),
                        ),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_browser(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let section = |title: &'static str| {
            div()
                .mb(px(8.))
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(title)
        };
        let folded = fold(&self.query);
        let words: Vec<&str> = folded.split_whitespace().collect();
        let mut body = v_flex().gap(px(24.)).w_full().max_w(BROWSER_MAX).child(
            Input::new(&self.search)
                .prefix(icon("search"))
                .aria_label("Search openings"),
        );
        if !words.is_empty() {
            let catalog = self.catalog.clone();
            let matches: Vec<_> = catalog
                .named
                .iter()
                .filter(|(_, _, _, key)| words.iter().all(|word| key.contains(word)))
                .collect();
            let total = matches.len();
            let rows: Vec<AnyElement> = matches
                .into_iter()
                .take(LIMIT)
                .map(|(name, eco, moves, _)| {
                    let moves = moves.clone();
                    h_flex()
                        .id(SharedString::from(format!("result-{name}")))
                        .test_support()
                        .role(Role::Link)
                        .aria_label(name.clone())
                        .justify_between()
                        .gap(px(12.))
                        .py(px(4.))
                        .child(
                            div()
                                .id(SharedString::from(format!("result-name-{name}")))
                                .min_w_0()
                                .cursor_pointer()
                                .pressable_link()
                                .child(widgets::highlighted(
                                    name.clone(),
                                    &quick_pick::found(name, &words, fold_char),
                                    palette,
                                )),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(eco.clone()),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_line(Some(moves.clone()), window, cx)
                        }))
                        .into_any_element()
                })
                .collect();
            body = body.child(
                widgets::card(palette)
                    .when(total == 0, |card| {
                        card.child(
                            div()
                                .text_size(px(14.))
                                .text_color(palette.text_muted)
                                .child("No openings found"),
                        )
                    })
                    .children(rows)
                    .when(total > LIMIT, |card| {
                        card.child(
                            div()
                                .mt(px(8.))
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(format!("{LIMIT} of {total} openings")),
                        )
                    }),
            );
            return body.into_any_element();
        }
        let width = crate::play_layout::column_width(window, BROWSER_MAX, self.area.size().width);
        let recent: Vec<Entry> = Self::recent(cx)
            .iter()
            .filter_map(|choice| entry(&choice.moves, Some(&choice.name)))
            .collect();
        // Pictures of openings that left Recent are let go; the popular ones are always shown.
        let catalog = self.catalog.clone();
        self.previews.retain(|(fen, in_card)| match in_card {
            true => catalog.popular.iter().any(|item| &item.fen == fen),
            false => recent.iter().any(|item| &item.fen == fen),
        });
        if !recent.is_empty() {
            let (count, item) = widgets::columns(width, px(220.), px(8.));
            let mut rows = Vec::new();
            let items: Vec<AnyElement> = recent
                .iter()
                .map(|recent| {
                    let board = self.preview(&recent.fen, false, cx);
                    let moves = recent.moves.clone();
                    h_flex()
                        .id(SharedString::from(format!("recent-{}", recent.name)))
                        .test_support()
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label(recent.name.clone())
                        .w(item)
                        .gap(px(8.))
                        .pl(px(6.))
                        .pr(px(12.))
                        .py(px(6.))
                        .border_1()
                        .border_color(palette.border)
                        .rounded(radii::BOX)
                        .bg(palette.elevated)
                        .cursor_pointer()
                        .pressable(palette)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_line(Some(moves.clone()), window, cx)
                        }))
                        .child(
                            div()
                                .w(px(44.))
                                .flex_none()
                                .child(crate::board::cached(&board)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(recent.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(recent.eco.clone()),
                        )
                        .into_any_element()
                })
                .collect();
            let mut items = items.into_iter().peekable();
            while items.peek().is_some() {
                rows.push(h_flex().gap(px(8.)).children(items.by_ref().take(count)));
            }
            body = body.child(
                v_flex()
                    .child(section("Recent"))
                    .child(v_flex().gap(px(8.)).children(rows)),
            );
        }
        let (count, card_width) = widgets::columns(width, CARD_MIN, CARD_GAP);
        let cards: Vec<AnyElement> = catalog
            .popular
            .iter()
            .map(|item| self.render_card(item, card_width, cx))
            .collect();
        let mut rows = Vec::new();
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            rows.push(
                h_flex()
                    .items_stretch()
                    .gap(CARD_GAP)
                    .children(cards.by_ref().take(count)),
            );
        }
        body = body.child(
            v_flex()
                .child(section("Popular"))
                .child(v_flex().gap(CARD_GAP).children(rows)),
        );
        let group = crate::prefs::get::<String>(cx, GROUP, "e4".into());
        let families: Vec<AnyElement> = catalog
            .tree
            .iter()
            .filter(|node| group_of(node) == group)
            .map(|node| self.render_node(node, cx, palette))
            .collect();
        body = body.child(
            v_flex()
                .child(section("All openings"))
                .child(
                    div().mb(px(12.)).child(widgets::header_pills(
                        "openings-group",
                        GROUPS
                            .iter()
                            .map(|(_, label, _)| Pill {
                                icon: None,
                                label: (*label).into(),
                                badge: None,
                                disabled: false,
                            })
                            .collect(),
                        GROUPS
                            .iter()
                            .position(|(key, _, _)| *key == group)
                            .unwrap_or(0),
                        |index, window, cx| {
                            crate::prefs::set(cx, GROUP, GROUPS[index].0);
                            window.refresh();
                        },
                        palette,
                    )),
                )
                .child(widgets::card(palette).children(families)),
        );
        body.into_any_element()
    }

    pub(super) fn render_detail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let line = self.line.clone().unwrap_or_default();
        let steps = line_of(&line);
        let opening = opening_at(&line);
        let title = opening
            .as_ref()
            .map_or_else(|| "Starting position".to_string(), |(name, _)| name.clone());
        let board_size = self.layout.board_size(window);
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(crate::board::cached(&self.board))
            .child(self.move_box.lone(true, palette));
        let figurine = crate::notation::figurine(cx);
        let moves = h_flex()
            .flex_wrap()
            .gap_x(px(8.))
            .gap_y(px(2.))
            .text_size(px(14.))
            .children(steps.iter().enumerate().map(|(index, (_, san, _))| {
                let upto = line[..=index].to_vec();
                let current = index + 1 == steps.len();
                h_flex()
                    .gap(px(4.))
                    .when(index % 2 == 0, |row| {
                        row.child(
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("{}.", index / 2 + 1)),
                        )
                    })
                    .child(
                        div()
                            .id(("opening-move", index))
                            .test_support()
                            .role(Role::Button)
                            .keyboard_focus(palette.primary)
                            .aria_label(san.clone())
                            .px(px(4.))
                            .rounded(radii::SMALL)
                            .cursor_pointer()
                            .when(current, |chip| chip.font_weight(FontWeight::MEDIUM))
                            .pressable(palette)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_line(Some(upto.clone()), window, cx)
                            }))
                            .child(crate::notation::shown(san, &figurine)),
                    )
            }));
        let train_choice = OpeningChoice {
            name: title.clone(),
            moves: line.clone(),
        };
        let analyse_line = line.clone();
        let puzzles = opening.clone();
        let description = self.description.clone().map(|page| {
            let blocks = page.blocks;
            let shown: Vec<DescriptionBlock> = if self.full_description {
                blocks.clone()
            } else {
                blocks
                    .iter()
                    .find(|block| !block.heading)
                    .cloned()
                    .into_iter()
                    .collect()
            };
            let more = blocks.len() > shown.len() || self.full_description;
            let url = page.url;
            v_flex()
                .gap(px(8.))
                .children(shown.into_iter().map(|block| {
                    div()
                        .text_size(px(14.))
                        .when(block.heading, |text| text.font_weight(FontWeight::MEDIUM))
                        .child(block.text)
                }))
                .when(more, |section| {
                    section.child(
                        h_flex().child(
                            Button::new("description-more")
                                .link()
                                .small()
                                .label(if self.full_description {
                                    "Less"
                                } else {
                                    "More"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.full_description = !this.full_description;
                                    cx.notify();
                                })),
                        ),
                    )
                })
                .child(
                    h_flex()
                        .gap(px(4.))
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child("Excerpt from")
                        .child(
                            div()
                                .id("wikibooks")
                                .role(Role::Link)
                                .keyboard_focus(palette.primary)
                                .aria_label("Wikibooks")
                                .underline()
                                .cursor_pointer()
                                .on_click(move |_, _, cx| cx.open_url(&url))
                                .child("Wikibooks"),
                        )
                        .child("·")
                        .child(
                            div()
                                .id("cc-by-sa")
                                .role(Role::Link)
                                .keyboard_focus(palette.primary)
                                .aria_label("CC BY-SA 4.0")
                                .underline()
                                .cursor_pointer()
                                .on_click(|_, _, cx| {
                                    cx.open_url("https://creativecommons.org/licenses/by-sa/4.0/")
                                })
                                .child("CC BY-SA 4.0"),
                        ),
                )
        });
        let panel = widgets::side_panel(palette)
            .h(board_size + px(36.) + widgets::STACK_GAP)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        Button::new("all-openings")
                            .ghost()
                            .small()
                            .icon(icon("arrow-left"))
                            .label("All openings")
                            .on_click(cx.listener(|this, _, window, cx| this.show_line(None, window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("openings-flip")
                            .ghost()
                            .small()
                            .icon(icon("arrow-up-down"))
                            .tooltip(crate::menu::flip_label(cx))
                            .accessibility_label("Flip board")
                            .on_click(cx.listener(|this, _, _, cx| this.flip(cx))),
                    ),
            )
            .child(
                v_flex()
                    .child(
                        div()
                            .id("opening-title")
                            .test_support()
                            .aria_label(title.clone())
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.highlighted)
                            .child(title),
                    )
                    .children(opening.as_ref().map(|(_, eco)| {
                        div()
                            .text_size(px(14.))
                            .text_color(palette.text_muted)
                            .child(eco.clone())
                    })),
            )
            .when(!steps.is_empty(), |panel| panel.child(moves))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("train-line")
                            .primary()
                            .icon(icon("graduation-cap"))
                            .label("Train this line")
                            .disabled(steps.is_empty())
                            .on_click(move |_, window, cx| {
                                Self::train(train_choice.clone(), window, cx)
                            }),
                    )
                    .child(
                        Button::new("analyse-line")
                            .outline()
                            .icon(icon("microscope"))
                            .label("Analyse")
                            .on_click(move |_, window, cx| {
                                kchess_board::analysis::open(kchess_board::analysis::Open::Line(analyse_line.clone()), window, cx)
                            }),
                    )
                    .children(puzzles.map(|(name, _)| {
                        Button::new("train-puzzles")
                            .outline()
                            .icon(icon("puzzle"))
                            .label("Train with puzzles")
                            .on_click(move |_, window, cx| {
                                crate::pages::puzzles::open_opening(&name, window, cx)
                            })
                    })),
            )
            .child(
                v_flex()
                    .id("opening-more")
                    .flex_1()
                    .min_h_0()
                    .gap(px(14.))
                    .overflow_y_scroll()
                    // The explorer comes first: the description and the popularity come from
                    // the web after each move, and above it they pushed the next move you were
                    // about to click down as they came (and up again at the next move).
                    .child(self.explorer.clone())
                    .when(self.popularity_months > 0, |more| {
                        more.child(
                            v_flex()
                                .pt(px(14.))
                                .border_t_1()
                                .border_color(palette.border)
                                .gap(px(4.))
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.text_muted)
                                        .child(format!("Popularity of {}", self.popularity_san))
                                        .child(
                                            div()
                                                .id("popularity-about")
                                                .tooltip(|window, cx| {
                                                    gpui_kit::component::tooltip::Tooltip::new(
                                                        "Share of Lichess blitz, rapid and classical \
                                                         games from the previous position that \
                                                         continued with this move, by month.",
                                                    )
                                                    .build(window, cx)
                                                })
                                                .child(icon("info").size(px(13.))),
                                        ),
                                )
                                .child(self.popularity.clone()),
                        )
                    })
                    .children(description.map(|description| {
                        div()
                            .pt(px(14.))
                            .border_t_1()
                            .border_color(palette.border)
                            .child(description)
                    })),
            );
        self.layout
            .render("opening-detail", board_column, panel, window, cx)
            .into_any_element()
    }
}
