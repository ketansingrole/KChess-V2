//! Every page's controls at the minimum, a mid and the default window size: each has an accessible
//! name, lies inside the window across (a page scrolls only down), and overlaps no other control.
//! Controls under 24 px (WCAG 2.5.8) are counted and may not grow in number.

use std::fmt::Write as _;

use gpui_kit::test::{ElementSnapshot, TestQueryExt as _};
use gpui_kit::{Role, px, size};

use super::{Harness, SIZES};
use crate::pages::Page;

/// Controls under 24 px today; lower it as they grow, never raise it.
const SMALL_TARGETS: usize = 73;

const CONTROLS: [Role; 7] = [
    Role::Button,
    Role::Link,
    Role::Tab,
    Role::TextInput,
    Role::ComboBox,
    Role::Switch,
    Role::CheckBox,
];

#[derive(Default)]
struct Findings {
    problems: Vec<String>,
    small: Vec<String>,
}

#[gpui_kit::test]
fn every_page_lays_out_its_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let mut findings = Findings::default();
    let mut pages: Vec<(Page, &str)> = crate::shell::nav()
        .iter()
        .map(|(page, label, _)| (*page, *label))
        .collect();
    pages.push((Page::Settings, "Settings"));
    for (width, height) in SIZES {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        for &(page, label) in &pages {
            app.with_window(|window, cx| crate::shell::open(page, window, cx));
            for _ in 0..20 {
                app.tick();
            }
            let controls = app.with_window(|window, _| window.elements());
            check(
                &format!("{label} at {width}×{height}"),
                width,
                &controls,
                &mut findings,
            );
        }
    }
    let mut report = String::new();
    for problem in &findings.problems {
        let _ = writeln!(report, "  {problem}");
    }
    assert!(
        findings.problems.is_empty(),
        "controls out of place:\n{report}"
    );
    assert!(
        findings.small.len() <= SMALL_TARGETS,
        "{} controls under 24 px, more than {SMALL_TARGETS}:\n  {}",
        findings.small.len(),
        findings.small.join("\n  ")
    );
}

fn check(page: &str, width: f32, elements: &[ElementSnapshot], findings: &mut Findings) {
    let controls: Vec<&ElementSnapshot> = elements
        .iter()
        .filter(|e| e.visible() && e.role().is_some_and(|role| CONTROLS.contains(&role)))
        .collect();
    for (index, control) in controls.iter().enumerate() {
        let name = describe(control);
        let bounds = control.bounds();
        if control.label().is_none_or(str::is_empty) {
            findings
                .problems
                .push(format!("{page}: {name} has no name"));
        }
        if f32::from(bounds.left()) < -0.5 || f32::from(bounds.right()) > width + 0.5 {
            findings.problems.push(format!(
                "{page}: {name} is cut off at the side ({bounds:?})"
            ));
        }
        if f32::from(bounds.size.width.min(bounds.size.height)) < 24. {
            findings
                .small
                .push(format!("{page}: {name} {:?}", bounds.size));
        }
        for other in &controls[index + 1..] {
            let (a, b) = (bounds, other.bounds());
            let across = f32::from(a.right().min(b.right()) - a.left().max(b.left()));
            let down = f32::from(a.bottom().min(b.bottom()) - a.top().max(b.top()));
            let nested = contains(a, b) || contains(b, a);
            if across > 1. && down > 1. && !nested {
                findings
                    .problems
                    .push(format!("{page}: {name} overlaps {}", describe(other)));
            }
        }
    }
}

fn contains(
    outer: gpui_kit::Bounds<gpui_kit::Pixels>,
    inner: gpui_kit::Bounds<gpui_kit::Pixels>,
) -> bool {
    outer.left() <= inner.left()
        && outer.top() <= inner.top()
        && outer.right() >= inner.right()
        && outer.bottom() >= inner.bottom()
}

fn describe(control: &ElementSnapshot) -> String {
    match control.label() {
        Some(label) => format!("{:?} {label:?}", control.role()),
        None => format!("{:?} {:?}", control.role(), control.path().last()),
    }
}
