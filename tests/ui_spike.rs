//! Recipe proof: headless keystrokes -> key bindings -> actions, with kit Input coexisting.

use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable, IntoElement, KeyBinding,
    ParentElement, Point, Render, Styled, TestAppContext, Window, WindowBounds, WindowOptions,
    base::Root,
    component::input::{Input, InputState},
    div, px, size,
};
use gpui_kit::prelude::*;

gpui_kit::actions!(spike, [Bump, Focus]);

struct Spike {
    focus: FocusHandle,
    input: Entity<InputState>,
    bumps: usize,
}

impl Render for Spike {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Spike")
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(|this, _: &Bump, _, cx| {
                this.bumps += 1;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Focus, window, cx| {
                let handle = this.input.focus_handle(cx);
                window.focus(&handle, cx);
            }))
            .child(Input::new(&self.input))
    }
}

fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Spike>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("j", Bump, Some("Spike && !Input")),
            KeyBinding::new("cmd-k", Focus, Some("Spike && !Input")),
        ]);
        let (window, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(600.), px(400.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| Spike {
                    focus: cx.focus_handle(),
                    input: cx.new(|cx| InputState::new(window, cx)),
                    bumps: 0,
                });
                let handle = view.read(cx).focus.clone();
                window.focus(&handle, cx);
                view
            },
        )
        .expect("open window");
        (window.downcast::<Root>().expect("root").into(), view)
    })
}

#[gpui_kit::test]
fn keystrokes_reach_actions_and_inputs_own_typing(cx: &mut TestAppContext) {
    let (window, view) = open(cx);
    cx.run_until_parked();
    cx.simulate_keystrokes(window, "j j");
    assert_eq!(view.read_with(cx, |v, _| v.bumps), 2);

    // Move focus into the Input: bare letters now type instead of firing "j".
    cx.simulate_keystrokes(window, "cmd-k");
    cx.simulate_keystrokes(window, "j a");
    assert_eq!(view.read_with(cx, |v, _| v.bumps), 2);
    let text = view.read_with(cx, |v, cx| v.input.read(cx).value().to_string());
    assert_eq!(text, "ja");
}
