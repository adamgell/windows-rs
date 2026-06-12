//! Regression coverage for the nested-dirty reconcile prune and the
//! `force_dirty_subtrees()` fix.
//!
//! `update()` skips a structurally-identical subtree via `can_skip_update`
//! and only consults `is_component_state_dirty` for the *exact* node it is
//! visiting. When a component dirties itself via `SetState` without changing
//! any props (e.g. a timer-backed status bar), nothing in the rebuilt Element
//! tree differs, so the skip fires at a stable ancestor and the dirty
//! component is never reached. `render_host` now calls `force_dirty_subtrees()`
//! before `reconcile`, which disables skip-on-equality for the pass whenever
//! any mounted instance is dirty, guaranteeing descent reaches it.
//!
//! These two tests pin both halves: the bug (no force call → pruned) and the
//! fix (force call → re-renders), toggling exactly the one call site that
//! `render_host::render_once` adds.

use std::cell::RefCell;
use std::rc::Rc;

use crate::core::backend::{Op, PropValue, RecordingBackend};
use crate::core::component_element::component;
use crate::core::element::{Element, StackPanel, TextBlock};
use crate::core::reconciler::Reconciler;
use crate::core::render_context::{RenderCx, SetState};

thread_local! {
    /// The most recent `counter` instance's state setter, captured during
    /// render so the test can dirty the instance from the outside — the
    /// headless stand-in for a timer tick mutating internal state.
    static COUNTER_SETTER: RefCell<Option<SetState<i32>>> = RefCell::new(None);
}

/// A stateful leaf whose text is driven purely by internal `use_state`, never
/// by props. Bumping its state dirties the instance without changing any
/// Element in the tree — the exact shape of a self-refreshing status bar.
fn counter(_: &(), cx: &mut RenderCx) -> Element {
    let (n, set_n) = cx.use_state(0_i32);
    COUNTER_SETTER.with(|s| *s.borrow_mut() = Some(set_n));
    Element::TextBlock(TextBlock::new(format!("count: {n}")))
}

/// Mirrors `NavigationView > vstack > status_bar`: the stateful component is
/// nested under a structurally-stable StackPanel, beside a static sibling, so
/// the skip fires at an ancestor rather than at the component's own slot.
fn tree() -> Element {
    let mut inner = StackPanel::vertical();
    inner.children = vec![component(counter, ())];
    let mut root = StackPanel::vertical();
    root.children = vec![
        Element::TextBlock(TextBlock::new("static header")),
        Element::StackPanel(inner),
    ];
    Element::StackPanel(root)
}

/// True if any mounted component instance still carries an unconsumed dirty
/// flag. `update_component` consumes the flag via `take_state_dirty` only when
/// it actually re-renders, so this flips false exactly when descent reached it.
fn any_instance_dirty(r: &Reconciler<RecordingBackend>) -> bool {
    r.component_instances
        .values()
        .any(|inst| inst.render_cx.peek_state_dirty())
}

/// True if the recorded ops include a string-valued `SetProp` equal to `want`
/// (i.e. a TextBlock whose `Prop::Text` was updated to that content).
fn text_was_set_to(r: &Reconciler<RecordingBackend>, want: &str) -> bool {
    r.backend.ops.iter().any(|op| {
        matches!(op, Op::SetProp { value: PropValue::Str(s), .. } if s == want)
    })
}

#[test]
fn nested_dirty_component_is_pruned_without_force_descent() {
    COUNTER_SETTER.with(|s| *s.borrow_mut() = None);
    let mut r = Reconciler::new(RecordingBackend::new());
    let noop: Rc<dyn Fn()> = Rc::new(|| {});

    let old = tree();
    let root = r.reconcile(None, &old, None, Rc::clone(&noop)).unwrap();
    assert!(
        text_was_set_to(&r, "count: 0"),
        "counter should render its initial state on mount"
    );

    // Internal state change only — no props/Elements change anywhere.
    COUNTER_SETTER.with(|s| s.borrow().as_ref().expect("setter captured at mount").call(1));
    assert!(any_instance_dirty(&r), "SetState must mark the instance dirty");

    r.backend.clear_ops();
    let new = tree(); // structurally identical to `old`
    // Deliberately NOT calling r.force_dirty_subtrees(): this is the unpatched
    // path that render_host used to take.
    r.reconcile(Some(&old), &new, Some(root), Rc::clone(&noop));

    assert!(
        !text_was_set_to(&r, "count: 1"),
        "BUG REPRO: the dirty nested component is pruned at its stable ancestor \
         and never re-renders, so its text is never updated"
    );
    assert!(
        any_instance_dirty(&r),
        "the dirty flag is never consumed because update_component never ran"
    );
}

#[test]
fn nested_dirty_component_rerenders_with_force_dirty_subtrees() {
    COUNTER_SETTER.with(|s| *s.borrow_mut() = None);
    let mut r = Reconciler::new(RecordingBackend::new());
    let noop: Rc<dyn Fn()> = Rc::new(|| {});

    let old = tree();
    let root = r.reconcile(None, &old, None, Rc::clone(&noop)).unwrap();

    COUNTER_SETTER.with(|s| s.borrow().as_ref().expect("setter captured at mount").call(1));
    assert!(any_instance_dirty(&r));

    r.backend.clear_ops();
    let new = tree();
    // The fix, exactly as render_host::render_once now calls it before reconcile.
    r.force_dirty_subtrees();
    r.reconcile(Some(&old), &new, Some(root), Rc::clone(&noop));

    assert!(
        text_was_set_to(&r, "count: 1"),
        "with forced descent the dirty nested component must re-render to its new state"
    );
    assert!(
        !any_instance_dirty(&r),
        "re-rendering consumes the dirty flag via take_state_dirty"
    );
}
