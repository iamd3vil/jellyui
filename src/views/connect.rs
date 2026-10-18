// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server and profile management: add servers, sign in, pick a saved profile.

use gpui_icons::LucideIcon;
use gpui_kit::{
    AppContext as _, Context, Div, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::{
    app::Jellyui,
    config::{Profile, Server, normalize_url},
    jellyfin::{Client, User},
    ui::{
        avatar::{Avatar, AvatarSize},
        button::{Button, ButtonSize, ButtonVariant},
        input::{Input, InputEvent, InputState},
        scroll_area::ScrollArea,
        theme::UiTheme,
    },
    views::cards::icon,
};

pub struct ConnectState {
    pub url: gpui_kit::Entity<InputState>,
    pub username: gpui_kit::Entity<InputState>,
    pub password: gpui_kit::Entity<InputState>,
    pub selected_server: Option<String>,
    pub public_users: Vec<User>,
    pub probing: bool,
    pub signing_in: bool,
    pub error: Option<String>,
}

impl ConnectState {
    pub fn new(window: &mut Window, cx: &mut Context<Jellyui>) -> Self {
        Self {
            url: cx
                .new(|cx| InputState::new(window, cx).placeholder("https://jellyfin.example.com")),
            username: cx.new(|cx| InputState::new(window, cx).placeholder("Username")),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Password")
                    .masked(true)
            }),
            selected_server: None,
            public_users: Vec::new(),
            probing: false,
            signing_in: false,
            error: None,
        }
    }

    /// Enter submits the relevant form.
    pub fn subscribe(&self, window: &mut Window, cx: &mut Context<Jellyui>) -> Vec<Subscription> {
        vec![
            cx.subscribe_in(&self.url, window, |this, _, event, _, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.add_server(cx);
                }
            }),
            cx.subscribe_in(&self.username, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let focus = this.connect.password.read(cx).focus_handle(cx);
                    window.focus(&focus, cx);
                }
            }),
            cx.subscribe_in(&self.password, window, |this, _, event, _, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.sign_in(cx);
                }
            }),
        ]
    }
}

impl Jellyui {
    pub fn select_server(&mut self, server_id: String, cx: &mut Context<Self>) {
        self.connect.selected_server = Some(server_id.clone());
        self.connect.public_users.clear();
        self.connect.error = None;
        let Some(server) = self.config.server(&server_id) else {
            return;
        };
        let client = Client::new(&server.url, &self.config.device_id);
        self.fetch_with(
            client,
            cx,
            |client| client.public_users(),
            move |this, result, cx| {
                if this.connect.selected_server.as_deref() != Some(server_id.as_str()) {
                    return;
                }
                if let Ok(users) = result {
                    this.connect.public_users = users;
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    pub fn add_server(&mut self, cx: &mut Context<Self>) {
        let raw = self.connect.url.read(cx).value().to_string();
        if raw.trim().is_empty() || self.connect.probing {
            return;
        }
        let url = normalize_url(&raw);
        self.connect.probing = true;
        self.connect.error = None;
        let client = Client::new(&url, &self.config.device_id);
        let probe_url = url.clone();
        self.fetch_with(
            client,
            cx,
            |client| client.public_info(),
            move |this, result, cx| {
                this.connect.probing = false;
                match result {
                    Ok(info) => {
                        let name = if info.server_name.is_empty() {
                            probe_url.clone()
                        } else {
                            info.server_name.clone()
                        };
                        this.config.upsert_server(Server {
                            id: info.id.clone(),
                            name,
                            url: probe_url.clone(),
                            profiles: Vec::new(),
                        });
                        this.save_config(cx);
                        this.rebuild_menu(cx);
                        this.select_server(info.id, cx);
                        this.toast("Server added", format!("Connected to {probe_url}"), cx);
                    }
                    Err(err) => {
                        this.connect.error = Some(format!("Could not reach {probe_url}: {err:#}"))
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    pub fn remove_server(&mut self, server_id: String, cx: &mut Context<Self>) {
        self.config.remove_server(&server_id);
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.server_id == server_id)
        {
            self.session = None;
        }
        self.connect.selected_server = self.config.servers.first().map(|s| s.id.clone());
        self.connect.public_users.clear();
        self.save_config(cx);
        self.rebuild_menu(cx);
        cx.notify();
    }

    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.connect.signing_in {
            return;
        }
        let Some(server_id) = self.connect.selected_server.clone() else {
            return;
        };
        let Some(server) = self.config.server(&server_id) else {
            return;
        };
        let username = self.connect.username.read(cx).value().to_string();
        let password = self.connect.password.read(cx).value().to_string();
        if username.trim().is_empty() {
            self.connect.error = Some("Enter a username.".into());
            cx.notify();
            return;
        }
        self.connect.signing_in = true;
        self.connect.error = None;
        let client = Client::new(&server.url, &self.config.device_id);
        self.fetch_with(
            client,
            cx,
            move |client| client.authenticate(username.trim(), &password),
            move |this, result, cx| {
                this.connect.signing_in = false;
                match result {
                    Ok(auth) => {
                        let profile = Profile {
                            user_id: auth.user.id.clone(),
                            name: auth.user.name.clone(),
                            token: auth.access_token.clone(),
                            image_tag: auth.user.primary_image_tag.clone(),
                            autoplay_next: false,
                        };
                        this.add_profile(&server_id, profile, cx);
                    }
                    Err(err) => {
                        let text = format!("{err:#}");
                        this.connect.error = Some(if text.contains("401") {
                            "Wrong username or password.".to_string()
                        } else {
                            text
                        });
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    pub fn render_connect(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = UiTheme::read(cx).clone();
        let selected = self
            .connect
            .selected_server
            .as_ref()
            .and_then(|id| self.config.server(id))
            .cloned();

        div()
            .size_full()
            .flex()
            .child(
                div()
                    .w(px(320.))
                    .h_full()
                    .flex_shrink_0()
                    .bg(t.colors.sidebar)
                    .border_r_1()
                    .border_color(t.colors.sidebar_border)
                    .flex()
                    .flex_col()
                    .child(self.render_server_list(cx)),
            )
            .child(
                div().flex_1().min_w_0().h_full().child(match selected {
                    Some(server) => self
                        .render_server_panel(&server, window, cx)
                        .into_any_element(),
                    None => self.render_welcome(cx).into_any_element(),
                }),
            )
    }

    fn render_server_list(&self, cx: &mut Context<Self>) -> Div {
        let t = UiTheme::read(cx).clone();
        let mut rows: Vec<Stateful<Div>> = Vec::new();
        for server in &self.config.servers {
            let selected = self.connect.selected_server.as_deref() == Some(server.id.as_str());
            let id = server.id.clone();
            rows.push(
                div()
                    .id(SharedString::from(format!("server.{}", server.id)))
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(t.radius.md)
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .when(selected, |el| el.bg(t.colors.sidebar_accent))
                    .hover(|s| s.bg(t.colors.sidebar_accent))
                    .child(
                        div()
                            .size(px(36.))
                            .rounded(px(10.))
                            .bg(if selected {
                                t.colors.primary
                            } else {
                                t.colors.secondary
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(
                                LucideIcon::Server,
                                16.,
                                if selected {
                                    t.colors.primary_foreground
                                } else {
                                    t.colors.secondary_foreground
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                                    .truncate()
                                    .child(server.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.colors.muted_foreground)
                                    .truncate()
                                    .child(format!(
                                        "{} · {} profile{}",
                                        server
                                            .url
                                            .trim_start_matches("https://")
                                            .trim_start_matches("http://"),
                                        server.profiles.len(),
                                        if server.profiles.len() == 1 { "" } else { "s" }
                                    )),
                            ),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_server(id.clone(), cx)),
                    ),
            );
        }

        let add_form = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .text_color(t.colors.muted_foreground)
                    .child("ADD A SERVER"),
            )
            .child(
                Input::new(&self.connect.url)
                    .aria_label("Server address")
                    .w_full(),
            )
            .child(
                Button::new("connect.add")
                    .w_full()
                    .disabled(self.connect.probing)
                    .child(icon(LucideIcon::Plus, 15., t.colors.primary_foreground))
                    .label(if self.connect.probing {
                        "Connecting…"
                    } else {
                        "Connect"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.add_server(cx))),
            );

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .pt(px(44.))
                    .px(px(20.))
                    .pb(px(12.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child("Servers"),
                    )
                    .when(self.session.is_some(), |el| {
                        el.child(
                            Button::new("connect.back")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .label("Done")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.screen = crate::app::Screen::Main;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                div().flex_1().min_h_0().child(
                    ScrollArea::new("connect.servers").size_full().child(
                        div()
                            .px(px(12.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .children(rows),
                    ),
                ),
            )
            .child(
                div()
                    .p(px(20.))
                    .border_t_1()
                    .border_color(t.colors.sidebar_border)
                    .child(add_form),
            )
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> Div {
        let t = UiTheme::read(cx).clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .child(
                div()
                    .size(px(64.))
                    .rounded(px(18.))
                    .bg(t.colors.primary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(LucideIcon::Clapperboard, 30., t.colors.primary_foreground)),
            )
            .child(div().text_size(px(26.)).font_weight(gpui_kit::FontWeight::BOLD).child("Welcome to Jellyui"))
            .child(
                div()
                    .text_color(t.colors.muted_foreground)
                    .max_w(px(380.))
                    .text_center()
                    .child("Add your Jellyfin server on the left to get started. You can keep several servers and switch between profiles at any time."),
            )
    }

    fn render_server_panel(
        &self,
        server: &Server,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = UiTheme::read(cx).clone();
        let client = Client::new(&server.url, &self.config.device_id);
        let server_id = server.id.clone();

        // Saved profiles: one click opens the session.
        let mut saved: Vec<Stateful<Div>> = Vec::new();
        for profile in &server.profiles {
            let active = self
                .config
                .active
                .as_ref()
                .is_some_and(|(s, u)| s == &server.id && u == &profile.user_id);
            let image = profile
                .image_tag
                .as_ref()
                .map(|tag| client.user_image_url(&profile.user_id, tag));
            let initial: String = profile
                .name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            let (sid, uid) = (server.id.clone(), profile.user_id.clone());
            let (sid2, uid2) = (server.id.clone(), profile.user_id.clone());
            saved.push(
                div()
                    .id(SharedString::from(format!(
                        "profile.{}.{}",
                        server.id, profile.user_id
                    )))
                    .w(px(150.))
                    .p(px(14.))
                    .rounded(t.radius.lg)
                    .border_1()
                    .border_color(if active {
                        t.colors.primary
                    } else {
                        t.colors.border
                    })
                    .bg(t.colors.card)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.colors.accent))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Avatar::new(SharedString::from(format!("avatar.{}", profile.user_id)))
                            .size(AvatarSize::Lg)
                            .fallback(initial)
                            .aria_label(profile.name.clone())
                            .when_some(
                                image.and_then(|url| crate::images::image(&url, cx)),
                                |a, image| a.image(image),
                            ),
                    )
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .truncate()
                            .child(profile.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if active {
                                t.colors.primary
                            } else {
                                t.colors.muted_foreground
                            })
                            .child(if active { "Active" } else { "Saved" }),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "forget.{}.{}",
                            server.id, profile.user_id
                        )))
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Xs)
                        .label("Forget")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.config.remove_profile(&sid2, &uid2);
                            if this.session.as_ref().is_some_and(|s| s.user_id == uid2) {
                                this.session = None;
                            }
                            this.save_config(cx);
                            this.rebuild_menu(cx);
                            cx.notify();
                        })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_session(&sid, &uid, cx);
                    })),
            );
        }

        // Public users not yet saved: click prefills the sign-in form.
        let mut public: Vec<Stateful<Div>> = Vec::new();
        for user in &self.connect.public_users {
            if server.profiles.iter().any(|p| p.user_id == user.id) {
                continue;
            }
            let image = user
                .primary_image_tag
                .as_ref()
                .map(|tag| client.user_image_url(&user.id, tag));
            let initial: String = user
                .name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            let name = user.name.clone();
            let needs_password = user.has_password;
            public.push(
                div()
                    .id(SharedString::from(format!("public.{}", user.id)))
                    .w(px(150.))
                    .p(px(14.))
                    .rounded(t.radius.lg)
                    .border_1()
                    .border_color(t.colors.border)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.colors.accent))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Avatar::new(SharedString::from(format!("avatar.public.{}", user.id)))
                            .size(AvatarSize::Lg)
                            .fallback(initial)
                            .aria_label(user.name.clone())
                            .when_some(
                                image.and_then(|url| crate::images::image(&url, cx)),
                                |a, image| a.image(image),
                            ),
                    )
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .truncate()
                            .child(user.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.colors.muted_foreground)
                            .child("Sign in"),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.connect
                            .username
                            .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
                        this.connect
                            .password
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        if needs_password {
                            let focus = this.connect.password.read(cx).focus_handle(cx);
                            window.focus(&focus, cx);
                        } else {
                            this.sign_in(cx);
                        }
                    })),
            );
        }

        let form = div()
            .w(px(360.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                Input::new(&self.connect.username)
                    .aria_label("Username")
                    .w_full(),
            )
            .child(
                Input::new(&self.connect.password)
                    .aria_label("Password")
                    .w_full(),
            )
            .when_some(self.connect.error.clone(), |el, err| {
                el.child(
                    div()
                        .text_color(t.colors.destructive)
                        .text_size(px(13.))
                        .child(err),
                )
            })
            .child(
                Button::new("connect.signin")
                    .disabled(self.connect.signing_in)
                    .child(icon(LucideIcon::LogOut, 15., t.colors.primary_foreground))
                    .label(if self.connect.signing_in {
                        "Signing in…"
                    } else {
                        "Sign in"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
            );

        let heading = |text: &str| {
            div()
                .text_size(px(12.))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(t.colors.muted_foreground)
                .child(text.to_uppercase())
        };

        ScrollArea::new("connect.panel")
            .size_full()
            .child(
                div()
                    .pt(px(44.))
                    .px(px(40.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap(px(16.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .text_size(px(26.))
                                            .font_weight(gpui_kit::FontWeight::BOLD)
                                            .child(server.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_color(t.colors.muted_foreground)
                                            .child(server.url.clone()),
                                    ),
                            )
                            .child(
                                Button::new("connect.remove-server")
                                    .variant(ButtonVariant::Outline)
                                    .size(ButtonSize::Sm)
                                    .child(icon(LucideIcon::Trash, 14., t.colors.foreground))
                                    .label("Remove server")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_server(server_id.clone(), cx)
                                    })),
                            ),
                    )
                    .when(!saved.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(12.))
                                .child(heading("Profiles"))
                                .child(div().flex().flex_wrap().gap(px(12.)).children(saved)),
                        )
                    })
                    .when(!public.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(12.))
                                .child(heading("Users on this server"))
                                .child(div().flex().flex_wrap().gap(px(12.)).children(public)),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(heading("Add a profile"))
                            .child(form),
                    ),
            )
            .into_any_element()
            .into_div()
    }
}

/// Helper so panel functions can return a `Div` uniformly.
trait IntoDiv {
    fn into_div(self) -> Div;
}
impl IntoDiv for gpui_kit::AnyElement {
    fn into_div(self) -> Div {
        div().size_full().child(self)
    }
}
