// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Library grid and search results.

use gpui_kit::{
    Context, Div, IntoElement, ParentElement as _, Styled, div, prelude::FluentBuilder as _, px,
};

use crate::{
    app::{Jellyui, Page},
    jellyfin::{Client, Item},
    ui::{
        button::{Button, ButtonVariant},
        scroll_area::ScrollArea,
        tabs::{Tab, Tabs},
        theme::UiTheme,
    },
    views::cards::{empty_state, poster_card, skeleton_row},
};

impl Jellyui {
    pub fn render_library(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Page::Library(data) = &self.page else {
            unreachable!()
        };
        let Some(client) = self.session.as_ref().map(|s| s.client.clone()) else {
            return div();
        };
        let t = UiTheme::read(cx).clone();
        let is_media_library = data.view.as_ref().is_none_or(|view| {
            matches!(
                view.collection_type.as_deref(),
                Some("movies") | Some("tvshows")
            )
        });

        let header = div()
            .px(px(28.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .text_color(t.colors.muted_foreground)
                    .child(if data.total > 0 {
                        format!("{} titles", data.total)
                    } else {
                        String::new()
                    }),
            )
            .when(is_media_library, |el| {
                el.child(
                    Tabs::new("library.sort")
                        .aria_label("Sort order")
                        .selected(data.sort)
                        .item(Tab::new("library.sort.name", "SortName", "A–Z"))
                        .item(Tab::new(
                            "library.sort.added",
                            "DateCreated",
                            "Recently added",
                        ))
                        .item(Tab::new(
                            "library.sort.premiere",
                            "PremiereDate",
                            "Release date",
                        ))
                        .item(Tab::new("library.sort.rating", "CommunityRating", "Rating"))
                        .on_change({
                            let this = cx.weak_entity();
                            move |value, _, cx| {
                                let sort: &'static str = match value.as_ref() {
                                    "DateCreated" => "DateCreated",
                                    "PremiereDate" => "PremiereDate",
                                    "CommunityRating" => "CommunityRating",
                                    _ => "SortName",
                                };
                                this.update(cx, |this, cx| this.set_sort(sort, cx)).ok();
                            }
                        }),
                )
            });

        let body = if data.loading && data.items.is_empty() {
            skeleton_row(
                "sk.library",
                8,
                crate::app::POSTER_W,
                crate::app::POSTER_H,
                cx,
            )
        } else if data.items.is_empty() {
            empty_state(
                if data.view.is_none() {
                    "No favorites yet"
                } else {
                    "Empty library"
                },
                if data.view.is_none() {
                    "Favorite a movie, show or episode to find it here."
                } else {
                    "No items were returned for this library."
                },
                cx,
            )
        } else {
            grid(&data.items, &client, cx)
        };

        let has_more = data.items.len() < data.total;
        div().size_full().child(
            ScrollArea::new("library.scroll").size_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .py(px(12.))
                    .pb(px(32.))
                    .child(header)
                    .child(body)
                    .when(has_more, |el| {
                        el.child(
                            div().px(px(28.)).child(
                                Button::new("library.more")
                                    .variant(ButtonVariant::Secondary)
                                    .disabled(data.loading)
                                    .label(if data.loading {
                                        "Loading…"
                                    } else {
                                        "Load more"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                            ),
                        )
                    }),
            ),
        )
    }

    pub fn render_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Page::Search(data) = &self.page else {
            unreachable!()
        };
        let Some(client) = self.session.as_ref().map(|s| s.client.clone()) else {
            return div();
        };
        let body = if data.query.is_empty() {
            empty_state(
                "Search your libraries",
                "Type in the search box above and press Enter.",
                cx,
            )
        } else if data.loading && data.results.is_empty() {
            skeleton_row(
                "sk.search",
                8,
                crate::app::POSTER_W,
                crate::app::POSTER_H,
                cx,
            )
        } else if data.results.is_empty() {
            empty_state(
                "No results",
                &format!("Nothing matched “{}”.", data.query),
                cx,
            )
        } else {
            grid(&data.results, &client, cx)
        };
        div().size_full().child(
            ScrollArea::new("search.scroll").size_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .py(px(12.))
                    .pb(px(32.))
                    .child(body),
            ),
        )
    }
}

fn grid(items: &[Item], client: &Client, cx: &mut Context<Jellyui>) -> Div {
    div()
        .px(px(28.))
        .flex()
        .flex_wrap()
        .gap(px(16.))
        .children(items.iter().map(|item| poster_card(item, client, cx)))
}
