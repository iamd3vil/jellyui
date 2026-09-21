// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Item detail: hero, actions, and season/episode browsing for series.

use gpui_icons::LucideIcon;
use gpui_kit::{
    Context, Div, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, StyledImage as _, div, img,
    linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};

use crate::{
    app::{DetailData, Jellyui, Page},
    jellyfin::{Client, Item, format_duration, format_runtime},
    ui::{
        button::{Button, ButtonSize, ButtonVariant},
        scroll_area::ScrollArea,
        tabs::{Tab, Tabs, TabsVariant},
        theme::UiTheme,
    },
    views::cards::{artwork, card_id, icon, meta_line},
};

impl Jellyui {
    pub fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Page::Detail(data) = &self.page else {
            unreachable!()
        };
        let Some(client) = self.session.as_ref().map(|s| s.client.clone()) else {
            return div();
        };
        let t = UiTheme::read(cx).clone();
        let item = &data.item;

        let backdrop = item
            .backdrop_url(&client, 1600)
            .or_else(|| item.wide_url(&client, 1600));
        let hero = div()
            .relative()
            .w_full()
            .h(px(300.))
            .overflow_hidden()
            .bg(t.colors.muted)
            .when_some(
                backdrop.and_then(|url| crate::images::image(&url, cx)),
                |el, image| {
                    el.child(
                        img(image)
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    )
                },
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(t.colors.background.opacity(0.0), 0.2),
                        linear_color_stop(t.colors.background, 1.0),
                    )),
            );

        let poster = item.poster_url(&client, 400);
        let info = div()
            .px(px(28.))
            .mt(px(-120.))
            .relative()
            .flex()
            .gap(px(24.))
            .items_end()
            .child(
                div()
                    .flex_shrink_0()
                    .rounded(t.radius.md)
                    .shadow(t.shadows.lg.clone())
                    .child(artwork(
                        poster,
                        180.,
                        270.,
                        t.radius.md,
                        LucideIcon::Film,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .pb(px(6.))
                    .when_some(item.series_name.clone(), |el, series| {
                        let series_id = item.series_id.clone();
                        el.child(
                            div()
                                .id("detail.series-link")
                                .cursor_pointer()
                                .text_color(t.colors.primary)
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .child(series.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(id) = &series_id {
                                        let mut series_item = Item {
                                            id: id.clone(),
                                            kind: "Series".into(),
                                            ..Default::default()
                                        };
                                        series_item.name = series.clone();
                                        this.open_item(series_item, cx);
                                    }
                                })),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(30.))
                            .line_height(px(36.))
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .child(match item.episode_code() {
                                Some(code) if item.kind == "Episode" => {
                                    format!("{code} · {}", item.name)
                                }
                                _ => item.name.clone(),
                            }),
                    )
                    .child(meta_line(item, cx))
                    .child(self.render_actions(item, cx)),
            );

        let overview = item.overview.clone().filter(|o| !o.is_empty()).map(|text| {
            div()
                .px(px(28.))
                .max_w(px(860.))
                .text_color(t.colors.foreground.opacity(0.85))
                .line_height(px(22.))
                .child(text)
        });

        let seasons = data
            .item
            .is_series()
            .then(|| self.render_seasons(data, &client, cx));

        div().size_full().child(
            ScrollArea::new("detail.scroll").size_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(24.))
                    .pb(px(40.))
                    .child(hero)
                    .child(info)
                    .children(overview)
                    .children(seasons),
            ),
        )
    }

    fn render_actions(&self, item: &Item, cx: &mut Context<Self>) -> Div {
        let t = UiTheme::read(cx).clone();
        let mut row = div().flex().items_center().gap(px(10.)).mt(px(4.));
        if item.is_playable() {
            let resume_at = item.resume_secs();
            if resume_at > 0 {
                let target = item.clone();
                row = row.child(
                    Button::new("detail.resume")
                        .size(ButtonSize::Lg)
                        .child(icon(LucideIcon::Play, 16., t.colors.primary_foreground))
                        .label(format!("Resume from {}", format_duration(resume_at)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.play(&target, true, window, cx)
                        })),
                );
                let target = item.clone();
                row = row.child(
                    Button::new("detail.restart")
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Lg)
                        .child(icon(
                            LucideIcon::RotateCcw,
                            16.,
                            t.colors.secondary_foreground,
                        ))
                        .label("Play from start")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.play(&target, false, window, cx)
                        })),
                );
            } else {
                let target = item.clone();
                row = row.child(
                    Button::new("detail.play")
                        .size(ButtonSize::Lg)
                        .child(icon(LucideIcon::Play, 16., t.colors.primary_foreground))
                        .label("Play")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.play(&target, false, window, cx)
                        })),
                );
            }
        } else if item.is_series() {
            let target = item.clone();
            row = row.child(
                Button::new("detail.play-series")
                    .size(ButtonSize::Lg)
                    .child(icon(LucideIcon::Play, 16., t.colors.primary_foreground))
                    .label("Play")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.play_series(&target, window, cx)
                    })),
            );
        }

        let played = item.user_data.played;
        row = row.child(
            Button::new("detail.watched")
                .variant(ButtonVariant::Secondary)
                .size(ButtonSize::Lg)
                .aria_label(if played {
                    "Mark as unwatched"
                } else {
                    "Mark as watched"
                })
                .child(if played {
                    icon(LucideIcon::CircleCheck, 16., t.colors.primary)
                } else {
                    icon(LucideIcon::Check, 16., t.colors.secondary_foreground)
                })
                .label(if played { "Watched" } else { "Mark watched" })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_played(cx))),
        );

        let favorite = item.user_data.is_favorite;
        row = row.child(
            Button::new("detail.favorite")
                .variant(ButtonVariant::Secondary)
                .size(ButtonSize::IconLg)
                .aria_label(if favorite {
                    "Remove from favorites"
                } else {
                    "Add to favorites"
                })
                .child(icon(
                    LucideIcon::Heart,
                    16.,
                    if favorite {
                        t.colors.destructive
                    } else {
                        t.colors.secondary_foreground
                    },
                ))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_favorite(cx))),
        );
        row
    }

    fn render_seasons(&self, data: &DetailData, client: &Client, cx: &mut Context<Self>) -> Div {
        let t = UiTheme::read(cx).clone();
        let mut tabs = Tabs::new("detail.seasons")
            .aria_label("Seasons")
            .variant(TabsVariant::default())
            .when_some(data.season_id.clone(), |tabs, id| tabs.selected(id));
        for season in &data.seasons {
            tabs = tabs.item(Tab::new(
                SharedString::from(format!("detail.season.{}", season.id)),
                season.id.clone(),
                season.name.clone(),
            ));
        }
        let tabs = tabs.on_change({
            let this = cx.weak_entity();
            move |value, _, cx| {
                this.update(cx, |this, cx| this.select_season(value.to_string(), cx))
                    .ok();
            }
        });

        let episodes: Vec<Stateful<Div>> = data
            .episodes
            .iter()
            .map(|ep| episode_row(ep, client, cx))
            .collect();
        div()
            .px(px(28.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .when(!data.seasons.is_empty(), |el| el.child(div().child(tabs)))
            .when(data.loading && episodes.is_empty(), |el| {
                el.child(
                    div()
                        .text_color(t.colors.muted_foreground)
                        .child("Loading episodes…"),
                )
            })
            .when(
                !data.loading && episodes.is_empty() && !data.seasons.is_empty(),
                |el| {
                    el.child(
                        div()
                            .text_color(t.colors.muted_foreground)
                            .child("No episodes in this season."),
                    )
                },
            )
            .child(div().flex().flex_col().gap(px(8.)).children(episodes))
    }
}

fn episode_row(ep: &Item, client: &Client, cx: &mut Context<Jellyui>) -> Stateful<Div> {
    let t = UiTheme::read(cx).clone();
    let still = ep.wide_url(client, 400);
    let code = ep.episode_code().unwrap_or_default();
    let runtime = ep.runtime_secs().map(format_runtime);
    let open = ep.clone();
    let play = ep.clone();
    let resume = ep.resume_secs() > 0;
    div()
        .id(card_id("episode", ep))
        .flex()
        .items_start()
        .gap(px(16.))
        .p(px(10.))
        .rounded(t.radius.md)
        .cursor_pointer()
        .hover(|s| s.bg(t.colors.accent))
        .on_click(cx.listener(move |this, _, _, cx| this.open_item(open.clone(), cx)))
        .child(
            div()
                .relative()
                .flex_shrink_0()
                .child(artwork(still, 176., 99., t.radius.sm, LucideIcon::Tv, cx))
                .when_some(ep.progress(), |el, p| {
                    el.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(px(4.))
                            .rounded_b(t.radius.sm)
                            .bg(gpui_kit::black().alpha(0.5))
                            .child(div().h_full().w(gpui_kit::relative(p)).bg(t.colors.primary)),
                    )
                })
                .when(ep.user_data.played, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top(px(6.))
                            .right(px(6.))
                            .size(px(20.))
                            .rounded_full()
                            .bg(t.colors.primary)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(LucideIcon::Check, 13., t.colors.primary_foreground)),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_color(t.colors.muted_foreground)
                                .font_family(t.fonts.mono.clone())
                                .text_size(px(12.))
                                .child(code),
                        )
                        .child(
                            div()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .truncate()
                                .child(ep.name.clone()),
                        )
                        .when_some(runtime, |el, r| {
                            el.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.colors.muted_foreground)
                                    .child(r),
                            )
                        }),
                )
                .when_some(ep.overview.clone().filter(|o| !o.is_empty()), |el, text| {
                    el.child(
                        div()
                            .text_color(t.colors.muted_foreground)
                            .line_clamp(2)
                            .child(text),
                    )
                }),
        )
        .child(
            Button::new(card_id("episode.play", ep))
                .variant(ButtonVariant::Secondary)
                .size(ButtonSize::Icon)
                .aria_label(if resume {
                    "Resume episode"
                } else {
                    "Play episode"
                })
                .child(icon(LucideIcon::Play, 15., t.colors.secondary_foreground))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.play(&play, resume, window, cx)),
                ),
        )
}
