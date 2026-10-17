//! Single-value slider using Kit state and pointer primitives.
use super::theme::{UiTheme, apply_style};
pub use gpui_kit::base::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::base::{SliderIndicator, SliderThumb, SliderTrack};
use gpui_kit::{
    AccessibleAction, App, ElementId, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _, relative,
};

#[derive(IntoElement)]
/// A Nova presentation of retained Kit slider state.
pub struct Slider {
    state: Entity<SliderState>,
    disabled: bool,
    label: Option<SharedString>,
    style: StyleRefinement,
}
impl Slider {
    /// Creates a Nova slider using the caller's retained Kit state.
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            disabled: false,
            label: None,
            style: Default::default(),
        }
    }
    /// Disables interaction and applies the disabled appearance.
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
    /// Sets the accessible name of the control.
    pub fn aria_label(mut self, value: impl Into<SharedString>) -> Self {
        self.label = Some(value.into());
        self
    }
}
impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = UiTheme::read(cx).clone();
        let colors = theme.colors;
        let state = self.state.read(cx);
        assert!(
            state.value().is_single(),
            "Slider expects single-value state"
        );
        let (min, max, step, value) = (
            state.min_value(),
            state.max_value(),
            state.step_value(),
            state.value().end(),
        );
        assert!(
            min.is_finite()
                && max.is_finite()
                && min < max
                && step.is_finite()
                && step > 0.
                && value.is_finite()
        );
        let percent = state.percentage().end;
        let focus = window
            .use_keyed_state(
                ElementId::from(("slider-focus", self.state.entity_id())),
                cx,
                |_, cx| cx.focus_handle(),
            )
            .read(cx)
            .clone()
            .tab_stop(!self.disabled);
        let focused = focus.is_focused(window);
        let click_focus = focus.clone();
        let thumb_focus = focus.clone();
        let input_state = self.state.clone();
        let increment = self.state.clone();
        let decrement = self.state.clone();
        let set_value = self.state.clone();
        let release = self.state.clone();
        let release_out = self.state.clone();
        let thumb = SliderThumb::new(&self.state)
            .disabled(self.disabled)
            .absolute()
            .left(relative(percent))
            .ml(-theme.space(1.5))
            .top(-theme.space(1.))
            .size(theme.space(3.))
            .rounded_full()
            .border_1()
            .border_color(colors.ring)
            .bg(colors.background)
            .when(focused && !self.disabled, |t| {
                t.border_color(colors.foreground)
            })
            .when(!self.disabled, |t| {
                t.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    thumb_focus.focus(window, cx)
                })
            });
        let root = div()
            .id(("slider", self.state.entity_id()))
            .role(Role::Slider)
            .aria_numeric_value(value as f64)
            .aria_min_numeric_value(min as f64)
            .aria_max_numeric_value(max as f64)
            .aria_numeric_value_step(step as f64)
            .aria_orientation(gpui_kit::Orientation::Horizontal)
            .when_some(self.label, |root, label| root.aria_label(label))
            .relative()
            .w_full()
            .h(theme.space(5.))
            .flex()
            .items_center()
            .when(self.disabled, |root| root.opacity(0.5).cursor_not_allowed())
            .when(!self.disabled, |root| {
                root.track_focus(&focus)
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        click_focus.focus(window, cx)
                    })
                    .on_key_down(move |event, window, cx| {
                        if event.keystroke.modifiers.modified() {
                            return;
                        }
                        let state = input_state.read(cx);
                        let value = match event.keystroke.key.as_str() {
                            "right" | "up" => state.value().end() + state.step_value(),
                            "left" | "down" => state.value().end() - state.step_value(),
                            "pageup" => state.value().end() + state.step_value() * 10.,
                            "pagedown" => state.value().end() - state.step_value() * 10.,
                            "home" => state.min_value(),
                            "end" => state.max_value(),
                            _ => return,
                        };
                        update_value(&input_state, value, window, cx);
                        cx.stop_propagation();
                    })
                    .on_a11y_action(AccessibleAction::Increment, move |_, window, cx| {
                        let state = increment.read(cx);
                        let value = state.value().end() + state.step_value();
                        update_value(&increment, value, window, cx);
                    })
                    .on_a11y_action(AccessibleAction::Decrement, move |_, window, cx| {
                        let state = decrement.read(cx);
                        let value = state.value().end() - state.step_value();
                        update_value(&decrement, value, window, cx);
                    })
                    .on_a11y_action(AccessibleAction::SetValue, move |data, window, cx| {
                        if let Some(gpui_kit::accesskit::ActionData::NumericValue(value)) = data {
                            update_value(&set_value, *value as f32, window, cx);
                        }
                    })
                    .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                        release.update(cx, |state, cx| state.handle_release(cx))
                    })
                    .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                        release_out.update(cx, |state, cx| state.handle_release(cx))
                    })
            })
            .child(
                // The track owns the pointer mapping, so keep it as tall as the
                // whole control; the visible bar is the thin indicator inside it.
                // Otherwise clicks land in the padding around the bar and do
                // nothing.
                SliderTrack::new(&self.state)
                    .disabled(self.disabled)
                    .relative()
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(
                        SliderIndicator::new(&self.state)
                            .relative()
                            .w_full()
                            .h(theme.space(1.))
                            .rounded_full()
                            .bg(colors.muted)
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(percent))
                                    .rounded_full()
                                    .bg(colors.primary),
                            )
                            .child(thumb),
                    ),
            );
        apply_style(root, &self.style)
    }
}
/// Sets the range end, step and value in place. Replacing the state would drop
/// the track bounds recorded at paint, so a click before the next frame would
/// map onto a zero-width track and yield NaN.
pub fn set_range(state: &mut SliderState, max: f32, step: f32, value: f32) {
    let old = std::mem::replace(state, SliderState::new());
    *state = old.max(max).step(step).default_value(value);
}

fn update_value(state: &Entity<SliderState>, value: f32, window: &mut Window, cx: &mut App) {
    if !value.is_finite() {
        return;
    }
    state.update(cx, |state, cx| {
        let min = state.min_value();
        let value = (min + ((value - min) / state.step_value()).round() * state.step_value())
            .clamp(min, state.max_value());
        state.set_value(value, window, cx);
        state.handle_release(cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        AppContext as _, Context, IntoElement, Modifiers, Render, Subscription, TestAppContext,
        VisualTestContext, Window, WindowHandle, div, point, px,
    };
    use std::{cell::RefCell, rc::Rc};

    struct Harness {
        state: Entity<SliderState>,
        releases: Rc<RefCell<Vec<f32>>>,
        changes: Rc<RefCell<usize>>,
        swallow: bool,
        _sub: Subscription,
    }

    impl Harness {
        fn new(cx: &mut Context<Self>, swallow: bool) -> Self {
            let state = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.));
            let releases = Rc::new(RefCell::new(Vec::new()));
            let changes = Rc::new(RefCell::new(0));
            let (r, c) = (releases.clone(), changes.clone());
            let _sub = cx.subscribe(&state, move |_, _, event: &SliderEvent, _| match event {
                SliderEvent::Change(_) => *c.borrow_mut() += 1,
                SliderEvent::Release(value) => r.borrow_mut().push(value.end()),
            });
            Self {
                state,
                releases,
                changes,
                swallow,
                _sub,
            }
        }
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(100.))
                .h(px(24.))
                // The player wraps its controls in a handler that swallows the
                // mouse down, so make sure that doesn't eat the seek either.
                .when(self.swallow, |el| {
                    el.on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation()
                    })
                })
                .child(Slider::new(&self.state))
        }
    }

    fn harness(cx: &mut TestAppContext, swallow: bool) -> WindowHandle<Harness> {
        cx.update(super::super::theme::init);
        cx.add_window(move |_, cx| Harness::new(cx, swallow))
    }

    #[gpui_kit::test]
    fn click_on_track_seeks_to_the_pointer(cx: &mut TestAppContext) {
        let window = harness(cx, true);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));

        // Click in the middle of the track, away from the thumb at value 0.
        visual.simulate_click(point(px(50.), px(10.)), Modifiers::default());

        window
            .update(cx, |harness, _, cx| {
                assert!(
                    !harness.releases.borrow().is_empty(),
                    "clicking the track should emit a Release event (changes: {})",
                    harness.changes.borrow()
                );
                let released = *harness.releases.borrow().last().unwrap();
                assert!(
                    (harness.state.read(cx).value().end() - 50.).abs() < 1.,
                    "released value {released} should be near 50"
                );
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn releasing_after_a_drag_still_seeks(cx: &mut TestAppContext) {
        let window = harness(cx, true);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));

        // A real click almost always moves the pointer a pixel or two, which
        // starts the track's drag before the button is released.
        visual.simulate_mouse_down(
            point(px(60.), px(10.)),
            gpui_kit::MouseButton::Left,
            Modifiers::default(),
        );
        visual.simulate_mouse_move(point(px(62.), px(10.)), None, Modifiers::default());
        visual.simulate_mouse_move(point(px(64.), px(10.)), None, Modifiers::default());
        visual.simulate_mouse_up(
            point(px(64.), px(10.)),
            gpui_kit::MouseButton::Left,
            Modifiers::default(),
        );

        window
            .update(cx, |harness, _, _| {
                assert!(
                    !harness.releases.borrow().is_empty(),
                    "releasing a drag on the track should emit Release (changes: {})",
                    harness.changes.borrow()
                );
            })
            .unwrap();
    }

    /// Mirrors the player: a poll rebuilds the retained [`SliderState`] on a
    /// short interval unless the slider reports that the user is scrubbing, and
    /// a subscriber turns `Change` into "scrubbing" and `Release` into a seek.
    struct PollHarness {
        state: Entity<SliderState>,
        scrubbing: bool,
        seeks: Rc<RefCell<Vec<f32>>>,
        resets: Rc<std::cell::Cell<usize>>,
        _sub: Subscription,
        _poll: gpui_kit::Task<()>,
    }

    impl PollHarness {
        fn new(cx: &mut Context<Self>) -> Self {
            let state = cx.new(|_| SliderState::new().min(0.).max(100.).step(1.));
            let seeks = Rc::new(RefCell::new(Vec::new()));
            let resets = Rc::new(std::cell::Cell::new(0));
            let (seek_sink, reset_sink) = (seeks.clone(), resets.clone());
            let _sub = cx.subscribe(&state, move |this, _, event: &SliderEvent, _| match event {
                SliderEvent::Change(_) => this.scrubbing = true,
                SliderEvent::Release(value) => {
                    this.scrubbing = false;
                    seek_sink.borrow_mut().push(value.end());
                }
            });
            let poll_state = state.clone();
            let _poll = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(8))
                        .await;
                    let keep_going = this
                        .update(cx, |this, cx| {
                            if !this.scrubbing {
                                reset_sink.set(reset_sink.get() + 1);
                                poll_state.update(cx, |s, cx| {
                                    set_range(s, 100., 1., 0.);
                                    cx.notify();
                                });
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !keep_going {
                        break;
                    }
                }
            });
            Self {
                state,
                scrubbing: false,
                seeks,
                resets,
                _sub,
                _poll,
            }
        }
    }

    impl Render for PollHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(100.)).h(px(24.)).child(Slider::new(&self.state))
        }
    }

    #[gpui_kit::test]
    fn click_near_the_track_still_seeks(cx: &mut TestAppContext) {
        let window = harness(cx, false);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));

        // The control is 20px tall, but the visible bar is only 4px, centred.
        // Click a few pixels above it, as a user aiming at the bar often does.
        visual.simulate_click(point(px(50.), px(3.)), Modifiers::default());

        window
            .update(cx, |harness, _, _| {
                let released = harness.releases.borrow().last().copied();
                assert!(
                    released.is_some_and(|v| (v - 50.).abs() < 5.),
                    "clicking near the bar should seek to the pointer, got {released:?}"
                );
            })
            .unwrap();
    }

    /// The player poll updates the state between frames. A click that lands
    /// before the next paint must still map onto the track painted last.
    #[gpui_kit::test]
    fn click_right_after_a_poll_update_seeks(cx: &mut TestAppContext) {
        let window = harness(cx, false);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));

        window
            .update(cx, |harness, _, cx| {
                harness.state.update(cx, |s, _| set_range(s, 100., 1., 10.))
            })
            .unwrap();
        visual.simulate_click(point(px(50.), px(10.)), Modifiers::default());
        visual.update(|window, cx| window.draw(cx).clear(cx));

        window
            .update(cx, |harness, _, _| {
                let released = harness.releases.borrow().last().copied();
                assert!(
                    released.is_some_and(|v| (v - 50.).abs() < 5.),
                    "should seek near the click, got {released:?}"
                );
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn click_seeks_while_the_player_poll_is_running(cx: &mut TestAppContext) {
        cx.update(super::super::theme::init);
        let window = cx.add_window(|_, cx| PollHarness::new(cx));
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));

        visual.simulate_mouse_down(
            point(px(50.), px(10.)),
            gpui_kit::MouseButton::Left,
            Modifiers::default(),
        );
        // Let several poll ticks elapse while the button is held down.
        visual
            .dispatcher
            .advance_clock(std::time::Duration::from_millis(40));
        visual.run_until_parked();
        visual.simulate_mouse_up(
            point(px(50.), px(10.)),
            gpui_kit::MouseButton::Left,
            Modifiers::default(),
        );
        visual.run_until_parked();

        window
            .update(cx, |harness, _, _| {
                assert!(
                    !harness.seeks.borrow().is_empty(),
                    "clicking the track should seek even while the poll runs (resets: {})",
                    harness.resets.get()
                );
                let last = *harness.seeks.borrow().last().unwrap();
                assert!(
                    (last - 50.).abs() < 5.,
                    "should seek near the click, got {last}"
                );
            })
            .unwrap();
    }
}
