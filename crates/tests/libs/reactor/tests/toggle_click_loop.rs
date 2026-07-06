//! End-to-end reproduction of the "toggle button sticks on" symptom, headless.
//!
//! A component owns a `bool` and renders a button whose `on_click` flips it,
//! nested under a structurally-stable ancestor (like a screen under the
//! NavigationView). We fire real Click events through the RecordingBackend and
//! simulate the render pass (`force_dirty_subtrees()` + reconcile) the host runs
//! after each state change — then assert the button toggles OFF again on the
//! second click. If the reconciler keeps a stale click handler (capturing the
//! old value) the second click is a no-op and the state sticks at `true`.

use std::rc::Rc;

use test_reactor::{Op, RecordingBackend};
use windows_reactor::{
    ControlId, ControlKind, Element, Event, PropValue, Reconciler, RenderCx, StackPanel, button,
    component, text_block,
};

/// A component owning a `bool`, rendering `[button, "state: {on}"]`. The button
/// flips the bool; the text lets us observe it via recorded SetProp ops.
fn toggle(_: &(), cx: &mut RenderCx) -> Element {
    let (on, set) = cx.use_state(false);
    let s = set.clone();
    let btn: Element = button("T").on_click(move || s.call(!on)).into();
    let mut sp = StackPanel::vertical();
    sp.children = vec![btn, Element::TextBlock(text_block(format!("state: {on}")))];
    Element::StackPanel(sp)
}

/// `StackPanel > [static header, StackPanel > component(toggle)]`.
fn tree() -> Element {
    let mut inner = StackPanel::vertical();
    inner.children = vec![component(toggle, ())];
    let mut root = StackPanel::vertical();
    root.children = vec![Element::TextBlock(text_block("hdr")), Element::StackPanel(inner)];
    Element::StackPanel(root)
}

fn first_button(r: &Reconciler<RecordingBackend>) -> Option<ControlId> {
    r.backend.ops.iter().find_map(|op| match op {
        Op::Create { id, kind } if *kind == ControlKind::Button => Some(*id),
        _ => None,
    })
}

fn text_was_set_to(r: &Reconciler<RecordingBackend>, want: &str) -> bool {
    r.backend
        .ops
        .iter()
        .any(|op| matches!(op, Op::SetProp { value: PropValue::Str(s), .. } if s == want))
}

/// One host render pass: force descent past stable ancestors, then reconcile a
/// structurally-identical new tree (re-runs dirty components).
fn render_pass(r: &mut Reconciler<RecordingBackend>, old: &Element, root: ControlId) -> Element {
    let new = tree();
    r.force_dirty_subtrees();
    r.reconcile(Some(old), &new, Some(root), Rc::new(|| {}));
    new
}

#[test]
fn toggle_button_flips_back_off_on_second_click() {
    let mut r = Reconciler::new(RecordingBackend::new());
    let t0 = tree();
    let root = r.reconcile(None, &t0, None, Rc::new(|| {})).unwrap();
    assert!(text_was_set_to(&r, "state: false"), "mounts in the off state");
    let btn = first_button(&r).expect("a Button control was created");

    // Click 1: fire -> handler s.call(true) -> dirty; then the host's render pass.
    r.backend.fire(btn, Event::Click);
    r.backend.clear_ops();
    let t1 = render_pass(&mut r, &t0, root);
    assert!(
        text_was_set_to(&r, "state: true"),
        "after click 1 the button turns ON"
    );

    // Click 2: this is the crux — the re-rendered button must carry a handler
    // that captures `on == true`, so this click flips it back to false.
    r.backend.fire(btn, Event::Click);
    r.backend.clear_ops();
    let _t2 = render_pass(&mut r, &t1, root);
    assert!(
        text_was_set_to(&r, "state: false"),
        "STUCK-ON REPRO: after click 2 the button must turn OFF again"
    );
}
