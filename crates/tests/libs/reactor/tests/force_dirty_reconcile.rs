//! Regression coverage for nested-dirty reconcile: a component that dirties its
//! OWN state (no prop change) under a structurally-stable ancestor is pruned by
//! `can_skip_update` unless `force_dirty_subtrees()` forces a full descent.
//!
//! Test 1 pins the bug (no force -> pruned); test 2 pins the fix (force ->
//! re-renders). Mirrors the real symptom: a toggle button in a screen nested
//! under the NavigationView shell whose state flips but never repaints.

use std::cell::RefCell;
use std::rc::Rc;

use test_reactor::{Op, RecordingBackend};
use windows_reactor::{
    ControlId, Element, PropValue, Reconciler, RenderCx, SetState, StackPanel, component, text_block,
};

thread_local! {
    /// The most recent `counter` instance's setter, captured during render so
    /// the test can dirty the instance from outside — the headless stand-in for
    /// a click handler mutating internal state.
    static COUNTER_SETTER: RefCell<Option<SetState<i32>>> = RefCell::new(None);
}

/// A stateful leaf whose text is driven purely by internal `use_state`, never by
/// props. Bumping its state dirties the instance without changing any Element in
/// the tree — the exact shape of a self-toggling button.
fn counter(_: &(), cx: &mut RenderCx) -> Element {
    let (n, set_n) = cx.use_state(0_i32);
    COUNTER_SETTER.with(|s| *s.borrow_mut() = Some(set_n));
    Element::TextBlock(text_block(format!("count: {n}")))
}

/// `StackPanel > [static header, StackPanel > counter]`: the stateful component
/// sits under a structurally-stable ancestor beside a static sibling, so the
/// skip fires at an ancestor rather than at the component's own slot.
fn tree() -> Element {
    let mut inner = StackPanel::vertical();
    inner.children = vec![component(counter, ())];
    let mut root = StackPanel::vertical();
    root.children = vec![
        Element::TextBlock(text_block("static header")),
        Element::StackPanel(inner),
    ];
    Element::StackPanel(root)
}

fn reconcile(
    r: &mut Reconciler<RecordingBackend>,
    old: Option<&Element>,
    new: &Element,
    existing: Option<ControlId>,
) -> Option<ControlId> {
    r.reconcile(old, new, existing, Rc::new(|| {}))
}

/// True if a TextBlock's text prop was set to `want` during the last pass.
fn text_was_set_to(r: &Reconciler<RecordingBackend>, want: &str) -> bool {
    r.backend
        .ops
        .iter()
        .any(|op| matches!(op, Op::SetProp { value: PropValue::Str(s), .. } if s == want))
}

#[test]
fn nested_dirty_component_is_pruned_without_force_descent() {
    COUNTER_SETTER.with(|s| *s.borrow_mut() = None);
    let mut r = Reconciler::new(RecordingBackend::new());

    let old = tree();
    let root = reconcile(&mut r, None, &old, None).unwrap();
    assert!(
        text_was_set_to(&r, "count: 0"),
        "initial mount should render count: 0"
    );

    // Internal state change only — no props/Elements change anywhere.
    COUNTER_SETTER.with(|s| s.borrow().as_ref().expect("setter captured at mount").call(1));

    r.backend.clear_ops();
    let new = tree(); // structurally identical to `old`
    // Deliberately NOT calling force_dirty_subtrees(): the unpatched path.
    reconcile(&mut r, Some(&old), &new, Some(root));

    assert!(
        !text_was_set_to(&r, "count: 1"),
        "BUG REPRO: the dirty nested component is pruned at its stable ancestor \
         and never re-renders"
    );
}

#[test]
fn nested_dirty_component_rerenders_with_force_dirty_subtrees() {
    COUNTER_SETTER.with(|s| *s.borrow_mut() = None);
    let mut r = Reconciler::new(RecordingBackend::new());

    let old = tree();
    let root = reconcile(&mut r, None, &old, None).unwrap();

    COUNTER_SETTER.with(|s| s.borrow().as_ref().expect("setter captured at mount").call(1));

    r.backend.clear_ops();
    let new = tree();
    r.force_dirty_subtrees(); // the fix, exactly as render_once now calls it
    reconcile(&mut r, Some(&old), &new, Some(root));

    assert!(
        text_was_set_to(&r, "count: 1"),
        "with forced descent the dirty nested component must re-render to its new state"
    );
}
