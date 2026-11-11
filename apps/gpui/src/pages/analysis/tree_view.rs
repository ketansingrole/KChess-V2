//! The move list: the tree as moves, comments and variations, and the menu on each move.

use super::*;
use crate::widgets::Pressable as _;

/// One piece of the move list: moves and inline comments flow; main-line comments and the
/// variations of the main line stand on their own lines.
pub(super) enum Piece {
    Inline(AnyElement),
    Block(AnyElement),
}

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
            let _ = page.update(cx, |page, cx| run(page, window, cx));
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
                                let _ = page.update(cx, |page, cx| page.toggle_glyph(&at, nag, cx));
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
    /// Paths on the main line.
    pub(super) mainline: std::collections::HashSet<String>,
    /// Bring the move shown into view after it changes (`block: 'nearest'`).
    pub(super) reveal: std::rc::Rc<std::cell::Cell<bool>>,
    pub(super) scroll: ScrollHandle,
    /// Moves shown with piece symbols.
    pub(super) figurine: crate::notation::Figurine,
}

impl TreeView<'_> {
    fn move_chip(&self, node: &TreeNode, path: &str, number: String, main: bool) -> AnyElement {
        let active = path == self.current;
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
        h_flex()
            .id(SharedString::from(format!("tree-{path}")))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(node.san.clone())
            .aria_selected(active)
            .px(px(4.))
            .py(px(2.))
            .mx(px(1.))
            .rounded(radii::XS)
            .cursor_pointer()
            .text_color(if main { palette.text } else { palette.toned })
            .when(main, |chip| chip.font_weight(FontWeight::MEDIUM))
            .when(active, |chip| {
                chip.bg(tint(palette.primary, 30.))
                    .text_color(palette.highlighted)
            })
            .when(!active, |chip| chip.pressable(palette))
            .on_click({
                let (page, target) = (page.clone(), target.clone());
                move |_, _, cx| {
                    let _ = page.update(cx, |page, cx| page.go_to(&target, cx));
                }
            })
            // A right-click shows the move it is about, then its menu.
            .on_mouse_down(MouseButton::Right, {
                let (page, target) = (page.clone(), target.clone());
                move |_, _, cx| {
                    let _ = page.update(cx, |page, cx| page.go_to(&target, cx));
                }
            })
            .when(active, |chip| {
                let (reveal, scroll) = (self.reveal.clone(), self.scroll.clone());
                chip.relative().child(
                    canvas(
                        move |bounds, window, _| {
                            if !reveal.replace(false) {
                                return;
                            }
                            let view = scroll.bounds();
                            let mut offset = scroll.offset();
                            let margin = px(8.);
                            if bounds.bottom() + margin > view.bottom() {
                                offset.y -= bounds.bottom() + margin - view.bottom();
                            } else if bounds.top() - margin < view.top() {
                                offset.y += view.top() - (bounds.top() - margin);
                            } else {
                                return;
                            }
                            scroll.set_offset(offset);
                            window.refresh();
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .when(!number.is_empty(), |chip| {
                chip.child(
                    div()
                        .mr(px(2.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(palette.dimmed)
                        .child(number),
                )
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
                        .and_then(|mark| mark.classification.as_deref())
                        .map(|classification| {
                            let (label, glyph, color) =
                                review::classification_look(classification, palette.dark);
                            div()
                                .id(SharedString::from(format!("label-{path}")))
                                .ml(px(1.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(color)
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(label)
                                        .build(window, cx)
                                })
                                .child(glyph)
                        }),
                )
            })
            .context_menu(move |menu, window, cx| {
                move_menu(&menu_page, &menu_path, &menu_state, menu, window, cx)
            })
            .into_any_element()
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
                let _ = page.update(cx, |page, cx| page.go_to(&target, cx));
            })
            .when(block, |comment| {
                comment
                    .w_full()
                    .my(px(2.))
                    .px(px(8.))
                    .py(px(4.))
                    .border_l_2()
                    .border_color(tint(palette.primary, 50.))
                    .rounded_r(radii::SM)
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

    /// A line's moves run on, the alternatives to a move follow it in
    /// brackets before the line resumes. Off the main line everything stays inline.
    pub(super) fn line(
        &self,
        parent: &TreeNode,
        parent_path: &str,
        first: Option<&TreeNode>,
        depth: usize,
        out: &mut Vec<Piece>,
    ) {
        let mut parent = parent;
        let mut parent_path = parent_path.to_string();
        let mut node = first.or_else(|| parent.children.first());
        let mut numbered = true;
        while let Some(current) = node {
            let path = child_path(&parent_path, &current.uci);
            out.push(Piece::Inline(self.move_chip(
                current,
                &path,
                move_number(parent.ply, numbered),
                depth == 0,
            )));
            let comment = current
                .comments
                .iter()
                .flatten()
                .map(|text| comment_text(text))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !comment.is_empty() {
                let element = self.comment(comment, &path, depth == 0);
                out.push(if depth == 0 {
                    Piece::Block(element)
                } else {
                    Piece::Inline(element)
                });
            }
            numbered = false;
            let is_main = parent
                .children
                .first()
                .is_some_and(|main| main.uci == current.uci);
            if is_main && parent.children.len() > 1 {
                if depth == 0 {
                    let rows: Vec<AnyElement> = parent.children[1..]
                        .iter()
                        .map(|alternative| {
                            let mut inline = vec![Piece::Inline(self.paren("("))];
                            self.line(parent, &parent_path, Some(alternative), 1, &mut inline);
                            inline.push(Piece::Inline(self.paren(")")));
                            flow(inline).into_any_element()
                        })
                        .collect();
                    out.push(Piece::Block(
                        v_flex()
                            .w_full()
                            .my(px(2.))
                            .ml(px(8.))
                            .pl(px(8.))
                            .border_l_2()
                            .border_color(self.palette.border)
                            .text_size(px(13.))
                            .children(rows)
                            .into_any_element(),
                    ));
                } else {
                    for alternative in &parent.children[1..] {
                        out.push(Piece::Inline(self.paren("(")));
                        self.line(parent, &parent_path, Some(alternative), depth + 1, out);
                        out.push(Piece::Inline(self.paren(")")));
                    }
                }
                numbered = true;
            }
            parent = current;
            parent_path = path;
            node = current.children.first();
        }
    }
}

/// Moves and inline comments in wrapping rows, broken by the pieces that stand alone.
pub(super) fn flow(pieces: Vec<Piece>) -> Div {
    let mut column = v_flex().w_full();
    let mut row: Vec<AnyElement> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Inline(element) => row.push(element),
            Piece::Block(element) => {
                if !row.is_empty() {
                    column = column.child(h_flex().flex_wrap().children(std::mem::take(&mut row)));
                }
                column = column.child(element);
            }
        }
    }
    if !row.is_empty() {
        column = column.child(h_flex().flex_wrap().children(row));
    }
    column
}
