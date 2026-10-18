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
    jellyfin::{Item, format_duration},
    player::PlayState,
    ui::{
        button::{Button, ButtonSize, ButtonVariant},
        menu::Menu,
        slider::Slider,
        theme::UiTheme,
    },
    views::cards::{artwork, icon},
};

/// How long before the end of an episode the next one is offered.
const UP_NEXT_LEAD_SECS: f64 = 20.;

// Hitboxes don't block by default, so overlay clicks would reach the surface and
// toggle pause too. Swallow them here, keeping keyboard focus on the player.
fn swallow(this: &mut Jellyui, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Jellyui>) {
    window.focus(&this.player_focus, cx);
    cx.stop_propagation();
}

impl Jellyui {
    pub fn render_player(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = UiTheme::read(cx).clone();
        let s = &self.player_status;
        // The view only stays open on an idle player after an episode finished.
        if s.state == PlayState::Idle
            && let Some(next) = &self.up_next
        {
            return self.render_finished(next, window, cx);
        }
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
                    .child(
                        ghost(
                            "player.mute",
                            if s.muted || s.volume == 0. {
                                LucideIcon::VolumeX
                            } else {
                                LucideIcon::Volume2
                            },
                            if s.muted { "Unmute" } else { "Mute" },
                            16.,
                        )
                        .on_click(cx.listener(|this, _, _, _| this.player.toggle_mute())),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .child(Slider::new(&self.volume_slider).aria_label("Volume")),
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

        let close = close_button(fullscreen, cx).opacity(visibility);

        let up_next = self
            .up_next
            .as_ref()
            .filter(|_| {
                s.state == PlayState::Playing
                    && s.duration > 0.
                    && s.duration - s.position <= UP_NEXT_LEAD_SECS
            })
            .map(|next| {
                let still = self
                    .session
                    .as_ref()
                    .and_then(|session| next.wide_url(&session.client, 400));
                div()
                    .absolute()
                    .right(px(20.))
                    // Clear of the controls while they show.
                    .bottom(px(if self.controls_visible { 116. } else { 20. }))
                    .on_mouse_down(MouseButton::Left, cx.listener(swallow))
                    .child(
                        div()
                            .id("player.up-next")
                            .w(px(340.))
                            .p(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .rounded(t.radius.lg)
                            .bg(gpui_kit::black().alpha(0.8))
                            .border_1()
                            .border_color(gpui_kit::white().alpha(0.15))
                            .cursor_pointer()
                            .hover(|s| s.bg(gpui_kit::black().alpha(0.95)))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.play_up_next(window, cx)),
                            )
                            .child(artwork(still, 128., 72., t.radius.sm, LucideIcon::Tv, cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div().text_size(px(11.)).text_color(dim).child("Up next"),
                                    )
                                    .child(
                                        div()
                                            .text_color(white)
                                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                                            .truncate()
                                            .child(next.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_family(t.fonts.mono.clone())
                                            .text_color(dim)
                                            .child(next.episode_code().unwrap_or_default()),
                                    ),
                            )
                            .child(icon(LucideIcon::Play, 16., white)),
                    )
            });

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
                        "m" => this.player.toggle_mute(),
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
            .children(up_next)
    }

    /// Shown after an episode plays to its end: the next one, ready to start.
    fn render_finished(
        &self,
        next: &Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = UiTheme::read(cx).clone();
        let white: gpui_kit::Rgba = gpui_kit::white().into();
        let dim: gpui_kit::Rgba = gpui_kit::white().alpha(0.7).into();
        let still = self
            .session
            .as_ref()
            .and_then(|session| next.wide_url(&session.client, 1600))
            .and_then(|url| crate::images::image(&url, cx));
        let title = match next.episode_code() {
            Some(code) => format!("{code} · {}", next.name),
            None => next.name.clone(),
        };
        div()
            .id("player.root")
            .track_focus(&self.player_focus)
            .relative()
            .size_full()
            .bg(gpui_kit::black())
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "enter" | "space" => this.play_up_next(window, cx),
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
            .when_some(still, |el, image| {
                el.child(
                    img(image)
                        .absolute()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .opacity(0.45),
                )
            })
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(gpui_kit::black().alpha(0.9), 0.0),
                linear_color_stop(gpui_kit::black().alpha(0.1), 1.0),
            )))
            .child(
                div()
                    .absolute()
                    .left(px(48.))
                    .bottom(px(48.))
                    .max_w(px(560.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().text_color(dim).child("Up next"))
                    .when_some(next.series_name.clone(), |el, series| {
                        el.child(
                            div()
                                .text_color(t.colors.primary)
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .child(series),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(28.))
                            .line_height(px(34.))
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_color(white)
                            .child(title),
                    )
                    .when_some(
                        next.overview.clone().filter(|o| !o.is_empty()),
                        |el, text| el.child(div().text_color(dim).line_clamp(3).child(text)),
                    )
                    .child(
                        div().mt(px(6.)).flex().gap(px(10.)).child(
                            Button::new("player.play-next")
                                .size(ButtonSize::Lg)
                                .child(icon(LucideIcon::Play, 16., t.colors.primary_foreground))
                                .label(if next.resume_secs() > 0 {
                                    format!("Resume from {}", format_duration(next.resume_secs()))
                                } else {
                                    "Play".into()
                                })
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.play_up_next(window, cx)
                                    }),
                                ),
                        ),
                    ),
            )
            .child(close_button(window.is_fullscreen(), cx))
    }
}

/// Top-left button that returns to the library.
fn close_button(fullscreen: bool, cx: &mut Context<Jellyui>) -> Div {
    div()
        .absolute()
        .top(px(12.))
        .left(px(if fullscreen { 12. } else { 84. }))
        .on_mouse_down(MouseButton::Left, cx.listener(swallow))
        .child(
            Button::new("player.close")
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Icon)
                .aria_label("Back to library")
                .child(icon(LucideIcon::ChevronDown, 20., gpui_kit::white().into()))
                .on_click(cx.listener(|this, _, _, cx| this.close_player_view(cx))),
        )
}
