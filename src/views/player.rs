// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Embedded video view: frame surface, transport controls, track menus.

use gpui_icons::LucideIcon;
use gpui_kit::{
    Context, Div, InteractiveElement as _, MouseButton, MouseDownEvent, ObjectFit,
    ParentElement as _, Stateful, StatefulInteractiveElement as _, Styled, StyledImage as _,
    Window, canvas, div, img, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};

use crate::{
    app::Jellyui,
    jellyfin::format_duration,
    player::PlayState,
    ui::{
        button::{Button, ButtonSize, ButtonVariant},
        menu::Menu,
        slider::Slider,
        theme::UiTheme,
    },
    views::cards::icon,
};

impl Jellyui {
    pub fn render_player(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = UiTheme::read(cx).clone();
        let s = &self.player_status;
        let white: gpui_kit::Rgba = gpui_kit::white().into();
        let dim: gpui_kit::Rgba = gpui_kit::white().alpha(0.7).into();
        let starting = s.state == PlayState::Starting;
        let fullscreen = window.is_fullscreen();

        let player = self.player.clone();
        let scale = window.scale_factor();
        // Reports the video area to the renderer so frames are sized to fit.
        let measure = canvas(
            move |bounds, _, _| {
                player.set_target_size(
                    (f32::from(bounds.size.width) * scale) as u32,
                    (f32::from(bounds.size.height) * scale) as u32,
                );
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let surface = div()
            .id("player.surface")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .child(measure)
            .on_click(cx.listener(|this, _, _, _| this.player.toggle_pause()))
            .when_some(self.current_frame.clone(), |el, frame| {
                el.child(img(frame).size_full().object_fit(ObjectFit::Contain))
            })
            .when(
                self.current_frame.is_none() || starting || s.buffering,
                |el| {
                    el.child(
                        div()
                            .absolute()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(10.))
                            .text_color(dim)
                            .child(icon(LucideIcon::LoaderCircle, 28., dim))
                            .child(if starting {
                                "Opening stream…"
                            } else {
                                "Buffering…"
                            }),
                    )
                },
            );

        let time = if s.duration > 0. {
            format!(
                "{} / {}",
                format_duration(s.position as i64),
                format_duration(s.duration as i64)
            )
        } else {
            format_duration(s.position as i64)
        };

        let ghost = |id: &'static str, glyph: LucideIcon, label: &'static str, size: f32| {
            Button::new(id)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Icon)
                .aria_label(label)
                .child(icon(glyph, size, white))
        };

        let has_subs = s.tracks.iter().any(|tr| tr.kind == "sub");
        let audio_menu = Menu::new(&self.audio_menu, "Audio track").trigger(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(white)
                .child(icon(LucideIcon::Volume2, 15., white))
                .child("Audio"),
        );
        let subtitle_menu = Menu::new(&self.subtitle_menu, "Subtitles").trigger(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(if has_subs { white } else { dim })
                .child(icon(
                    LucideIcon::Captions,
                    15.,
                    if has_subs { white } else { dim },
                ))
                .child("Subtitles"),
        );

        // Hitboxes don't block by default, so overlay clicks would reach the surface and
        // toggle pause too. Swallow them here, keeping keyboard focus on the player.
        fn swallow(
            this: &mut Jellyui,
            _: &MouseDownEvent,
            window: &mut Window,
            cx: &mut Context<Jellyui>,
        ) {
            window.focus(&this.player_focus, cx);
            cx.stop_propagation();
        }

        // Controls overlay the video and fade with pointer activity.
        let visibility = crate::ui::theme::transition_value(
            "player.controls-opacity",
            if self.controls_visible { 1. } else { 0. },
            t.motion.normal,
            window,
            cx,
        );
        let controls = div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .px(px(20.))
            .pt(px(28.))
            .pb(px(14.))
            .opacity(visibility)
            .on_mouse_down(MouseButton::Left, cx.listener(swallow))
            .bg(linear_gradient(
                180.,
                linear_color_stop(gpui_kit::black().alpha(0.0), 0.0),
                linear_color_stop(gpui_kit::black().alpha(0.85), 1.0),
            ))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(Slider::new(&self.seek_slider).aria_label("Seek").w_full())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        ghost(
                            "player.back10",
                            LucideIcon::RotateCcw,
                            "Back 10 seconds",
                            18.,
                        )
                        .on_click(cx.listener(|this, _, _, _| this.player.seek_relative(-10.))),
                    )
                    .child(
                        Button::new("player.pause")
                            .size(ButtonSize::IconLg)
                            .aria_label(if s.paused { "Play" } else { "Pause" })
                            .child(icon(
                                if s.paused {
                                    LucideIcon::Play
                                } else {
                                    LucideIcon::Pause
                                },
                                18.,
                                t.colors.primary_foreground,
                            ))
                            .on_click(cx.listener(|this, _, _, _| this.player.toggle_pause())),
                    )
                    .child(
                        ghost(
                            "player.fwd30",
                            LucideIcon::RotateCw,
                            "Forward 30 seconds",
                            18.,
                        )
                        .on_click(cx.listener(|this, _, _, _| this.player.seek_relative(30.))),
                    )
                    .child(
                        div()
                            .ml(px(8.))
                            .text_size(px(12.))
                            .font_family(t.fonts.mono.clone())
                            .text_color(dim)
                            .child(time),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px(px(16.))
                            .text_color(white)
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .truncate()
                            .text_center()
                            .child(s.title.clone()),
                    )
                    .child(audio_menu)
                    .child(subtitle_menu)
                    .child(
                        ghost(
                            "player.fullscreen",
                            if fullscreen {
                                LucideIcon::Minimize
                            } else {
                                LucideIcon::Maximize
                            },
                            "Toggle fullscreen",
                            16.,
                        )
                        .on_click(|_, window, _| window.toggle_fullscreen()),
                    )
                    .child(
                        ghost("player.stop", LucideIcon::Square, "Stop", 14.)
                            .on_click(cx.listener(|this, _, _, _| this.player.stop())),
                    ),
            );

        let close = div()
            .absolute()
            .opacity(visibility)
            .top(px(12.))
            .left(px(if fullscreen { 12. } else { 84. }))
            .on_mouse_down(MouseButton::Left, cx.listener(swallow))
            .child(
                Button::new("player.close")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .aria_label("Back to library")
                    .child(icon(LucideIcon::ChevronDown, 20., white))
                    .on_click(cx.listener(|this, _, _, cx| this.close_player_view(cx))),
            );

        div()
            .id("player.root")
            .track_focus(&self.player_focus)
            .relative()
            .size_full()
            .bg(gpui_kit::black())
            .flex()
            .flex_col()
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "space" | "k" => this.player.toggle_pause(),
                        "left" => this.player.seek_relative(-5.),
                        "right" => this.player.seek_relative(5.),
                        "j" => this.player.seek_relative(-10.),
                        "l" => this.player.seek_relative(10.),
                        "f" => window.toggle_fullscreen(),
                        "escape" => {
                            if window.is_fullscreen() {
                                window.toggle_fullscreen();
                            } else {
                                this.close_player_view(cx);
                            }
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                window.focus(&this.player_focus, cx);
            }))
            .on_mouse_move(cx.listener(|this, _, _, cx| {
                let was_hidden = !this.controls_visible;
                this.show_controls();
                if was_hidden {
                    cx.notify();
                }
            }))
            .child(surface)
            .when(self.controls_visible || visibility > 0., |el| {
                el.child(controls).child(close)
            })
    }
}
