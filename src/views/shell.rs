// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Main screen: sidebar navigation, top bar, page content, now-playing bar.

use gpui_icons::LucideIcon;
use gpui_kit::{
    Context, Div, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::{
    app::{Jellyui, Page},
    jellyfin::format_duration,
    player::PlayState,
    ui::{
        button::{Button, ButtonSize, ButtonVariant},
        input::Input,
        menu::MenuItem,
        progress::Progress,
        sidebar::{Sidebar, SidebarItem, sidebar_dropdown, sidebar_group_label},
        theme::UiTheme,
    },
    views::cards::icon,
};

const SIDEBAR_W: f32 = 236.;
const RAIL_W: f32 = 60.;

impl Jellyui {
    pub fn render_shell(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = UiTheme::read(cx).clone();
        let page = match &self.page {
            Page::Home(_) => self.render_home(cx).into_any_element(),
            Page::Library(_) => self.render_library(cx).into_any_element(),
            Page::Detail(_) => self.render_detail(cx).into_any_element(),
            Page::Search(_) => self.render_search(cx).into_any_element(),
        };
        let width = if self.sidebar_collapsed {
            RAIL_W
        } else {
            SIDEBAR_W
        };
        if self.player_open {
            return div()
                .flex()
                .size_full()
                .child(self.render_player(window, cx))
                .into_any_element();
        }
        div()
            .flex()
            .size_full()
            .child(
                div()
                    .w(px(width))
                    .h_full()
                    .flex_shrink_0()
                    .border_r_1()
                    .border_color(t.colors.sidebar_border)
                    .child(self.render_sidebar(window, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.render_topbar(cx))
                    .child(div().flex_1().min_h_0().w_full().child(page))
                    .children(self.render_now_playing(cx)),
            )
            .into_any_element()
    }

    fn render_sidebar(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = UiTheme::read(cx).clone();
        let collapsed = self.sidebar_collapsed;
        let fg = t.colors.sidebar_foreground;
        let is_home = matches!(self.page, Page::Home(_)) && self.history.is_empty();
        let current_library = match &self.page {
            Page::Library(data) if self.history.is_empty() => {
                data.view.as_ref().map(|view| view.id.clone())
            }
            _ => None,
        };

        let header = div()
            .pt(px(30.))
            .flex()
            .items_center()
            .gap(px(8.))
            .when(!collapsed, |el| el.pl(px(6.)))
            .when(collapsed, |el| el.justify_center())
            .child(
                div()
                    .size(px(28.))
                    .rounded(px(8.))
                    .bg(t.colors.primary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(
                        LucideIcon::Clapperboard,
                        16.,
                        t.colors.primary_foreground,
                    )),
            )
            .when(!collapsed, |el| {
                el.child(
                    div()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child("Jellyui"),
                )
            });

        let mut sidebar = Sidebar::new("nav", "Navigation")
            .header(header)
            .footer(self.render_profile(cx));

        sidebar = sidebar.child(
            SidebarItem::new("nav.home", "Home")
                .collapsed(collapsed)
                .selected(is_home)
                .icon(icon(LucideIcon::House, 16., fg))
                .on_activate(cx.listener(|this, _, _, cx| this.open_home(cx))),
        );
        sidebar = sidebar.child(
            SidebarItem::new("nav.search", "Search")
                .collapsed(collapsed)
                .selected(matches!(self.page, Page::Search(_)))
                .icon(icon(LucideIcon::Search, 16., fg))
                .on_activate(cx.listener(|this, _, window, cx| {
                    let query = this.search_input.read(cx).value().to_string();
                    this.open_search(query, cx);
                    let focus = this.search_input.read(cx).focus_handle(cx);
                    window.focus(&focus, cx);
                })),
        );

        sidebar = sidebar.child(
            SidebarItem::new("nav.favorites", "Favorites")
                .collapsed(collapsed)
                .selected(
                    matches!(&self.page, Page::Library(data) if data.view.is_none())
                        && self.history.is_empty(),
                )
                .icon(icon(LucideIcon::Heart, 16., fg))
                .on_activate(cx.listener(|this, _, _, cx| this.open_favorites(cx))),
        );

        if !collapsed {
            sidebar = sidebar.child(
                div()
                    .mt(px(12.))
                    .child(sidebar_group_label("Libraries", cx)),
            );
        } else {
            sidebar = sidebar.child(div().h(px(12.)));
        }
        for view in &self.views {
            let glyph = match view.collection_type.as_deref() {
                Some("movies") => LucideIcon::Film,
                Some("tvshows") => LucideIcon::Tv,
                Some("music") => LucideIcon::Music,
                Some("homevideos") | Some("photos") => LucideIcon::Image,
                Some("boxsets") => LucideIcon::Boxes,
                _ => LucideIcon::Folder,
            };
            let target = view.clone();
            sidebar = sidebar.child(
                SidebarItem::new(
                    SharedString::from(format!("nav.lib.{}", view.id)),
                    view.name.clone(),
                )
                .collapsed(collapsed)
                .selected(current_library.as_deref() == Some(view.id.as_str()))
                .icon(icon(glyph, 16., fg))
                .on_activate(
                    cx.listener(move |this, _, _, cx| this.open_library(target.clone(), cx)),
                ),
            );
        }
        sidebar
    }

    fn render_profile(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = UiTheme::read(cx).clone();
        let collapsed = self.sidebar_collapsed;
        let (name, server, image) = match &self.session {
            Some(s) => (
                s.user_name.clone(),
                s.server_name.clone(),
                s.user_image.clone(),
            ),
            None => ("Not signed in".to_string(), String::new(), None),
        };
        let initial: String = name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        let avatar = crate::ui::avatar::Avatar::new("profile.avatar")
            .fallback(initial)
            .aria_label(name.clone())
            .when_some(
                image.and_then(|url| crate::images::image(&url, cx)),
                |a, image| a.image(image),
            );
        let trigger = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .min_w_0()
            .when(collapsed, |el| el.justify_center())
            .child(avatar)
            .when(!collapsed, |el| {
                el.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .child(
                            div()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .truncate()
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.colors.muted_foreground)
                                .truncate()
                                .child(server),
                        ),
                )
                .child(icon(
                    LucideIcon::ChevronDown,
                    14.,
                    t.colors.muted_foreground,
                ))
            });
        sidebar_dropdown(&self.profile_menu, "Switch profile", collapsed, cx).trigger(trigger)
    }

    /// Rebuilds the profile dropdown entries from the current config.
    pub fn rebuild_menu(&mut self, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let mut items = Vec::new();
        let active = self.config.active.clone();
        for server in &self.config.servers {
            items.push(MenuItem::label(
                SharedString::from(format!("menu.server.{}", server.id)),
                server.name.clone(),
            ));
            for profile in &server.profiles {
                let server_id = server.id.clone();
                let user_id = profile.user_id.clone();
                let selected = active
                    .as_ref()
                    .is_some_and(|(s, u)| s == &server_id && u == &user_id);
                let handle = this.clone();
                items.push(
                    MenuItem::new(
                        SharedString::from(format!("menu.profile.{server_id}.{user_id}")),
                        profile.name.clone(),
                    )
                    .radio(selected)
                    .on_click(move |_, _, cx| {
                        let (server_id, user_id) = (server_id.clone(), user_id.clone());
                        handle
                            .update(cx, move |this, cx| {
                                this.open_session(&server_id, &user_id, cx);
                            })
                            .ok();
                    }),
                );
            }
        }
        if !items.is_empty() {
            items.push(MenuItem::separator());
        }
        let handle = this.clone();
        items.push(
            MenuItem::new("menu.manage", "Servers & profiles…").on_click(move |_, _, cx| {
                handle.update(cx, |this, cx| this.show_connect(cx)).ok();
            }),
        );
        let dark = self.config.dark.unwrap_or(true);
        let handle = this.clone();
        items.push(
            MenuItem::new(
                "menu.theme",
                if dark {
                    "Light appearance"
                } else {
                    "Dark appearance"
                },
            )
            .on_click(move |_, _, cx| {
                handle.update(cx, |this, cx| this.toggle_theme(cx)).ok();
            }),
        );
        if self.session.is_some() {
            let handle = this.clone();
            items.push(
                MenuItem::new("menu.signout", "Sign out").on_click(move |_, _, cx| {
                    handle.update(cx, |this, cx| this.sign_out(cx)).ok();
                }),
            );
        }
        self.profile_menu
            .update(cx, |menu, cx| menu.set_items(items, cx));
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = UiTheme::read(cx).clone();
        let title: String = match &self.page {
            Page::Home(_) => "Home".into(),
            Page::Library(data) => data.title.clone(),
            Page::Detail(data) => data.item.name.clone(),
            Page::Search(_) => "Search".into(),
        };
        let can_go_back = !self.history.is_empty();
        div()
            .h(px(56.))
            .flex_shrink_0()
            .pt(px(8.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                Button::new("top.sidebar")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .aria_label("Toggle sidebar")
                    .child(icon(
                        if self.sidebar_collapsed {
                            LucideIcon::PanelLeftOpen
                        } else {
                            LucideIcon::PanelLeftClose
                        },
                        16.,
                        t.colors.muted_foreground,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sidebar_collapsed = !this.sidebar_collapsed;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("top.back")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .aria_label("Back")
                    .disabled(!can_go_back)
                    .child(icon(
                        LucideIcon::ChevronLeft,
                        18.,
                        if can_go_back {
                            t.colors.foreground
                        } else {
                            t.colors.muted_foreground
                        },
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(16.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title),
            )
            .child(
                Input::new(&self.search_input)
                    .aria_label("Search")
                    .w(px(320.))
                    .h(px(34.)),
            )
            .child(
                Button::new("top.refresh")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .aria_label("Refresh")
                    .child(icon(LucideIcon::RefreshCw, 15., t.colors.muted_foreground))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.load_page(cx);
                        cx.notify();
                    })),
            )
    }

    fn render_now_playing(&self, cx: &mut Context<Self>) -> Option<Div> {
        let s = &self.player_status;
        if s.state == PlayState::Idle {
            return None;
        }
        let t = UiTheme::read(cx).clone();
        let starting = s.state == PlayState::Starting;
        let progress = if s.duration > 0. {
            (s.position / s.duration).clamp(0., 1.)
        } else {
            0.
        };
        let time = if starting {
            "Starting mpv…".to_string()
        } else if s.duration > 0. {
            format!(
                "{} / {}",
                format_duration(s.position as i64),
                format_duration(s.duration as i64)
            )
        } else {
            format_duration(s.position as i64)
        };
        Some(
            div()
                .h(px(68.))
                .flex_shrink_0()
                .border_t_1()
                .border_color(t.colors.border)
                .bg(t.colors.card)
                .px(px(20.))
                .flex()
                .items_center()
                .gap(px(14.))
                .child(
                    Button::new("np.seek-back")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .aria_label("Back 10 seconds")
                        .child(icon(LucideIcon::RotateCcw, 16., t.colors.foreground))
                        .on_click(cx.listener(|this, _, _, _| this.player.seek_relative(-10.))),
                )
                .child(
                    Button::new("np.pause")
                        .size(ButtonSize::Icon)
                        .aria_label(if s.paused { "Resume" } else { "Pause" })
                        .child(icon(
                            if s.paused {
                                LucideIcon::Play
                            } else {
                                LucideIcon::Pause
                            },
                            16.,
                            t.colors.primary_foreground,
                        ))
                        .on_click(cx.listener(|this, _, _, _| this.player.toggle_pause())),
                )
                .child(
                    Button::new("np.seek-fwd")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .aria_label("Forward 30 seconds")
                        .child(icon(LucideIcon::RotateCw, 16., t.colors.foreground))
                        .on_click(cx.listener(|this, _, _, _| this.player.seek_relative(30.))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(12.))
                                .child(
                                    div()
                                        .id("np.title")
                                        .cursor_pointer()
                                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                                        .truncate()
                                        .hover(|s| s.text_color(t.colors.primary))
                                        .child(s.title.clone())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_player_view(window, cx)
                                        })),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(12.))
                                        .font_family(t.fonts.mono.clone())
                                        .text_color(t.colors.muted_foreground)
                                        .child(time),
                                ),
                        )
                        .child(
                            Progress::new("np.progress")
                                .value(progress * 100.)
                                .when(starting, |p| p.indeterminate())
                                .w_full(),
                        ),
                )
                .child(
                    Button::new("np.stop")
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .aria_label("Stop playback")
                        .child(icon(LucideIcon::Square, 13., t.colors.foreground))
                        .label("Stop")
                        .on_click(cx.listener(|this, _, _, _| this.player.stop())),
                ),
        )
    }
}
