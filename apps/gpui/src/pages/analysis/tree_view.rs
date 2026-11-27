//! The move list: the main line as the game pages' scoresheet, its comments and variations in
//! blocks under the row they branch from, as Lichess's analysis board shows them, and the menu on
//! each move.

use super::*;

/// What a move's menu needs to know of it.
#[derive(Clone)]
pub(super) struct MoveMenu {
    commented: bool,
    nag: Option<i64>,
    mainline: bool,
    fen: String,
}

/// What a menu item does to the page.
pub(super) type PageAction =
    Box<dyn Fn(&mut AnalysisPage, &mut Window, &mut Context<AnalysisPage>)>;

/// Right-click a move to comment on it, annotate it, promote or delete it.
pub(super) fn move_menu(
    page: &WeakEntity<AnalysisPage>,
    path: &str,
    state: &MoveMenu,
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let act = |run: PageAction| {
        let page = page.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            page.update(cx, |page, cx| run(page, window, cx)).ok();
        }
    };
    let (at, current) = (path.to_string(), state.nag);
    let annotate = {
        let (page, at) = (page.clone(), at.clone());
        move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            notes::MOVE_GLYPHS
                .iter()
                .fold(menu, |menu, (nag, glyph, label)| {
                    let (page, at, nag) = (page.clone(), at.clone(), *nag);
                    menu.item(
                        PopupMenuItem::new(format!("{glyph}  {label}"))
                            .checked(current == Some(nag))
                            .on_click(move |_, _, cx| {
                                page.update(cx, |page, cx| page.toggle_glyph(&at, nag, cx))
                                    .ok();
                            }),
                    )
                })
        }
    };
    let (promote_at, delete_at, line_at, fen) =
        (at.clone(), at.clone(), at.clone(), state.fen.clone());
    menu.item(
        PopupMenuItem::new(if state.commented {
            "Edit comment"
        } else {
            "Add comment"
        })
        .icon(icon("message-square-text"))
        .on_click(act(Box::new(|page, window, cx| {
            page.focus_notes(window, cx)
        }))),
    )
    .submenu_with_icon(Some(icon("highlighter")), "Annotate", window, cx, annotate)
    .separator()
    .item(
        PopupMenuItem::new("Make main line")
            .icon(icon("arrow-up-to-line"))
            .disabled(state.mainline)
            .on_click(act(Box::new(move |page, _, cx| {
                page.path = promote_at.clone();
                page.promote(cx)
            }))),
    )
    .item(
        PopupMenuItem::new("Delete from here")
            .icon(icon("scissors"))
            .on_click(act(Box::new(move |page, window, cx| {
                page.path = delete_at.clone();
                page.delete_from_here(window, cx)
            }))),
    )
    .separator()
    .item(
        PopupMenuItem::new("Copy FEN")
            .icon(icon("copy"))
            .on_click(act(Box::new(move |page, window, cx| {
                page.copy(fen.clone(), "FEN", window, cx)
            }))),
    )
    .item(
        PopupMenuItem::new("Copy line")
            .icon(icon("list-tree"))
            .on_click(act(Box::new(move |page, window, cx| {
                page.copy_line(&line_at, window, cx)
            }))),
    )
}

pub(super) struct TreeView<'a> {
    pub(super) current: &'a str,
    pub(super) page: WeakEntity<AnalysisPage>,
    pub(super) palette: Palette,
    /// The review's label on each main-line move, by path.
    pub(super) marks: HashMap<String, review::ReviewMark>,
    /// The novelty's move, by path, and what its N says.
    pub(super) novelty: Option<(String, SharedString)>,
    /// Paths on the main line.
    pub(super) mainline: std::collections::HashSet<String>,
    /// Bring the move shown into view after it changes (`block: 'nearest'`).
    pub(super) reveal: std::rc::Rc<std::cell::Cell<bool>>,
    pub(super) scroll: ScrollHandle,
    /// Moves shown with piece symbols.
    pub(super) figurine: crate::notation::Figurine,
}

impl TreeView<'_> {
    /// A move's click, menu, reveal and what it shows after its SAN, on the shell a main-line
    /// cell or a variation's move gives it.
    fn dress(
        &self,
        shell: gpui_kit::base::ObservedElement<Stateful<Div>>,
        node: &TreeNode,
        path: &str,
        active: bool,
    ) -> AnyElement {
        let palette = &self.palette;
        let (page, target) = (self.page.clone(), path.to_string());
        let glyph = move_glyph(node);
        let annotated = glyph.is_some();
        let menu_state = MoveMenu {
            commented: node
                .comments
                .iter()
                .flatten()
                .any(|comment| !comment_text(comment).is_empty()),
            nag: node
                .nags
                .iter()
                .flatten()
                .copied()
                .find(|nag| (1..=6).contains(nag))
                .map(i64::from),
            mainline: self.mainline.contains(path),
            fen: node.fen.clone(),
        };
        let (menu_page, menu_path) = (page.clone(), path.to_string());
        let novelty = self.novelty_at(path);
        shell
            .on_click({
                let (page, target) = (page.clone(), target.clone());
                move |_, _, cx| {
                    page.update(cx, |page, cx| page.go_to(&target, cx)).ok();
                }
            })
            // A right-click shows the move it is about, then its menu.
            .on_mouse_down(MouseButton::Right, {
                let (page, target) = (page.clone(), target.clone());
                move |_, _, cx| {
                    page.update(cx, |page, cx| page.go_to(&target, cx)).ok();
                }
            })
            .when(active, |chip| {
                let reveal = self.reveal.clone();
                chip.relative()
                    .child(widgets::reveal(self.scroll.clone(), move || {
                        reveal.replace(false)
                    }))
            })
            .child(crate::notation::shown(&node.san, &self.figurine))
            .children(glyph.map(|(glyph, _)| {
                div()
                    .ml(px(1.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(glyph)
            }))
            // Without an annotation, the review's label.
            .when(!annotated, |chip| {
                chip.children(
                    self.marks
                        .get(path)
                        .and_then(|mark| Some((mark.classification?, mark.tablebase.clone())))
                        .map(|(classification, tablebase)| {
                            let (label, glyph, color) =
                                review::classification_look(classification.as_str(), palette.dark);
                            let label = SharedString::from(match tablebase {
                                Some(said) => format!("{label}: {said}"),
                                None => label.to_string(),
                            });
                            div()
                                .id(SharedString::from(format!("label-{path}")))
                                .test_support()
                                .aria_label(label.clone())
                                .ml(px(1.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(color)
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(label.clone())
                                        .build(window, cx)
                                })
                                .child(glyph)
                        }),
                )
            })
            // As ChessBase marks a novelty: N, the first move the databases have not seen.
            .children(novelty.map(|said| {
                div()
                    .id(SharedString::from(format!("novelty-{path}")))
                    .test_support()
                    .aria_label(said.clone())
                    .ml(px(2.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.info)
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(said.clone()).build(window, cx)
                    })
                    .child("N")
            }))
            .context_menu(move |menu, window, cx| {
                move_menu(&menu_page, &menu_path, &menu_state, menu, window, cx)
            })
            .into_any_element()
    }

    /// What the novelty's N says, on the novelty's move.
    fn novelty_at(&self, path: &str) -> Option<SharedString> {
        self.novelty
            .as_ref()
            .filter(|(at, _)| at == path)
            .map(|(_, said)| said.clone())
    }

    /// The move's name: its SAN, and whether it is the novelty.
    fn label(&self, node: &TreeNode, path: &str) -> SharedString {
        match self.novelty_at(path) {
            Some(_) => format!("{}, novelty", node.san).into(),
            None => node.san.clone().into(),
        }
    }

    fn id(path: &str) -> SharedString {
        format!("tree-{path}").into()
    }

    /// A main-line move: its cell in the sheet's row.
    fn sheet_move(&self, node: &TreeNode, path: &str) -> AnyElement {
        let active = path == self.current;
        let shell = widgets::sheet_move(
            Self::id(path),
            self.label(node, path),
            active,
            &self.palette,
        );
        self.dress(shell, node, path, active)
    }

    /// A variation's move, after its number when it opens the line or follows a branch.
    fn variation_move(&self, node: &TreeNode, path: &str, number: String) -> AnyElement {
        let palette = &self.palette;
        let active = path == self.current;
        let shell = widgets::move_button(Self::id(path), self.label(node, path), active, palette)
            .px(px(4.))
            .py(px(2.))
            .mx(px(1.))
            .when(!active, |chip| chip.text_color(palette.toned))
            .when(!number.is_empty(), |chip| {
                chip.child(
                    div()
                        .mr(px(2.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(palette.dimmed)
                        .child(number),
                )
            });
        self.dress(shell, node, path, active)
    }

    fn comment(&self, text: String, path: &str, block: bool) -> AnyElement {
        let (page, target) = (self.page.clone(), path.to_string());
        let palette = &self.palette;
        div()
            .id(SharedString::from(format!("comment-{path}")))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(text.clone())
            .text_size(px(13.))
            .line_height(relative(1.5))
            .text_color(palette.text_muted)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                page.update(cx, |page, cx| page.go_to(&target, cx)).ok();
            })
            .when(block, |comment| {
                comment
                    .w_full()
                    .px(px(8.))
                    .py(px(4.))
                    .border_l_2()
                    .border_color(tint(palette.primary, 50.))
                    .rounded_r(radii::BOX)
                    .bg(palette.elevated)
            })
            .when(!block, |comment| comment.mx(px(4.)))
            .child(text)
            .into_any_element()
    }

    fn paren(&self, text: &'static str) -> AnyElement {
        div()
            .text_color(self.palette.dimmed)
            .child(text)
            .into_any_element()
    }

    /// The comments on a move, joined.
    fn comment_of(node: &TreeNode) -> String {
        node.comments
            .iter()
            .flatten()
            .map(|text| comment_text(text))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The main line from `root` as the game pages' scoresheet: numbered rows of White's and
    /// Black's moves. A move's comment and the alternatives to it break the line after it, in a
    /// block under its row; the line resumes on a row of its own, "…" holding White's place.
    pub(super) fn sheet(&self, root: &TreeNode) -> impl IntoElement + use<> {
        let palette = &self.palette;
        let mut out: Vec<AnyElement> = Vec::new();
        let mut row: Vec<AnyElement> = Vec::new();
        // The sheet's rows so far, for its stripes.
        let mut rows = 0;
        let mut end_row = |row: &mut Vec<AnyElement>, out: &mut Vec<AnyElement>| {
            out.push(widgets::sheet_row(std::mem::take(row), rows, palette).into_any_element());
            rows += 1;
        };
        let (mut parent, mut parent_path) = (root, String::new());
        let mut node = root.children.first();
        while let Some(current) = node {
            let path = child_path(&parent_path, &current.uci);
            let black = parent.ply % 2 == 1;
            if !black || row.is_empty() {
                if !row.is_empty() {
                    end_row(&mut row, &mut out);
                }
                let number = parent.ply as usize / 2 + 1;
                row.push(
                    widgets::sheet_number(
                        SharedString::from(format!("tree-number-{path}")),
                        number,
                        palette,
                    )
                    .into_any_element(),
                );
                if black {
                    row.push(widgets::sheet_gap(palette).into_any_element());
                }
            }
            row.push(self.sheet_move(current, &path));
            let comment = Self::comment_of(current);
            let alternatives = &parent.children[1..];
            if !comment.is_empty() || !alternatives.is_empty() {
                if !black {
                    row.push(widgets::sheet_gap(palette).into_any_element());
                }
                end_row(&mut row, &mut out);
                out.push(self.interruption(parent, &parent_path, comment, &path, alternatives));
            }
            parent = current;
            parent_path = path;
            node = current.children.first();
        }
        if !row.is_empty() {
            end_row(&mut row, &mut out);
        }
        widgets::sheet_text(div().id("analysis-sheet").test_support().w_full()).children(out)
    }

    /// Under a main-line move's row: its comment, then each alternative to it as a line of its
    /// own, indented under the row.
    fn interruption(
        &self,
        parent: &TreeNode,
        parent_path: &str,
        comment: String,
        path: &str,
        alternatives: &[TreeNode],
    ) -> AnyElement {
        let palette = &self.palette;
        let lines: Vec<AnyElement> = alternatives
            .iter()
            .map(|alternative| {
                let mut inline = vec![self.paren("(")];
                self.line(parent, parent_path, alternative, &mut inline);
                inline.push(self.paren(")"));
                h_flex().flex_wrap().children(inline).into_any_element()
            })
            .collect();
        v_flex()
            .w_full()
            .gap(px(4.))
            .px(px(8.))
            .py(px(4.))
            .border_b_1()
            .border_color(palette.border)
            .bg(palette.elevated)
            .when(!comment.is_empty(), |block| {
                block.child(self.comment(comment, path, true))
            })
            .when(!lines.is_empty(), |block| {
                block.child(
                    v_flex()
                        .ml(px(8.))
                        .pl(px(8.))
                        .border_l_2()
                        .border_color(palette.border)
                        .children(lines),
                )
            })
            .into_any_element()
    }

    /// A variation from `first`, an alternative to the main move from `parent`: its moves run
    /// on, a comment in their flow, the alternatives to a move in brackets after it.
    fn line(
        &self,
        parent: &TreeNode,
        parent_path: &str,
        first: &TreeNode,
        out: &mut Vec<AnyElement>,
    ) {
        let mut parent = parent;
        let mut parent_path = parent_path.to_string();
        let mut node = Some(first);
        let mut numbered = true;
        while let Some(current) = node {
            let path = child_path(&parent_path, &current.uci);
            out.push(self.variation_move(current, &path, move_number(parent.ply, numbered)));
            let comment = Self::comment_of(current);
            if !comment.is_empty() {
                out.push(self.comment(comment, &path, false));
            }
            numbered = false;
            // The node itself, not its move: a tree with two siblings of one move drew the second
            // as the main line's alternatives again, without end.
            let is_main = parent
                .children
                .first()
                .is_some_and(|main| std::ptr::eq(main, current));
            if is_main && parent.children.len() > 1 {
                for alternative in &parent.children[1..] {
                    out.push(self.paren("("));
                    self.line(parent, &parent_path, alternative, out);
                    out.push(self.paren(")"));
                }
                numbered = true;
            }
            parent = current;
            parent_path = path;
            node = current.children.first();
        }
    }
}
