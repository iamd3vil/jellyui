// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Root view: session, navigation, background fetching, player polling.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, IntoElement, ParentElement as _, Render,
    RenderImage, Styled, Task, Window, div, px, rgb, rgba,
};

use crate::{
    config::{Config, Profile},
    jellyfin::{Client, Item, ItemQuery},
    player::{PlayRequest, PlayState, Player, PlayerStatus},
    ui::menu::MenuItem,
    ui::{
        input::InputState,
        menu::MenuState,
        slider::{SliderEvent, SliderState},
        theme::{UiRadius, UiTheme},
        toast::ToastState,
    },
    views::connect::ConnectState,
};

pub const POSTER_W: f32 = 148.;
pub const POSTER_H: f32 = 222.;
pub const WIDE_W: f32 = 264.;
pub const WIDE_H: f32 = 148.;
pub const PAGE_SIZE: usize = 100;
/// Pointer idle time before the player controls fade out.
const CONTROLS_HIDE_AFTER: Duration = Duration::from_millis(2500);

pub fn theme(dark: bool) -> UiTheme {
    let mut t = if dark {
        UiTheme::neutral_dark()
    } else {
        UiTheme::neutral_light()
    };
    if dark {
        t.colors.background = rgb(0x0d0d10);
        t.colors.foreground = rgb(0xf2f2f5);
        t.colors.card = rgb(0x16161b);
        t.colors.card_foreground = rgb(0xf2f2f5);
        t.colors.popover = rgb(0x18181e);
        t.colors.popover_foreground = rgb(0xf2f2f5);
        t.colors.secondary = rgb(0x24242c);
        t.colors.secondary_foreground = rgb(0xf2f2f5);
        t.colors.muted = rgb(0x1c1c22);
        t.colors.muted_foreground = rgb(0x9a9aa6);
        t.colors.accent = rgb(0x26262f);
        t.colors.accent_foreground = rgb(0xf2f2f5);
        t.colors.border = rgba(0xffffff14);
        t.colors.input = rgba(0xffffff1f);
        t.colors.sidebar = rgb(0x111115);
        t.colors.sidebar_foreground = rgb(0xe6e6ea);
        t.colors.sidebar_accent = rgb(0x1f1f27);
        t.colors.sidebar_accent_foreground = rgb(0xffffff);
        t.colors.sidebar_border = rgba(0xffffff12);
    } else {
        t.colors.background = rgb(0xfafafa);
        t.colors.sidebar = rgb(0xf3f3f6);
    }
    let primary = rgb(0x8b5cf6);
    t.colors.primary = primary;
    t.colors.primary_foreground = rgb(0xffffff);
    t.colors.ring = primary;
    t.colors.sidebar_primary = primary;
    t.colors.sidebar_primary_foreground = rgb(0xffffff);
    t.colors.sidebar_ring = primary;
    t.radius = UiRadius::new(px(8.));
    t
}

pub struct Session {
    pub server_id: String,
    pub server_name: String,
    pub user_id: String,
    pub user_name: String,
    pub user_image: Option<String>,
    pub client: Client,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Connect,
    Main,
}

#[derive(Default)]
pub struct HomeData {
    pub loading: bool,
    pub resume: Vec<Item>,
    pub next_up: Vec<Item>,
    pub latest: Vec<(Item, Vec<Item>)>,
}

pub struct LibraryData {
    pub view: Item,
    pub title: String,
    pub loading: bool,
    pub items: Vec<Item>,
    pub total: usize,
    pub sort: &'static str,
}

pub struct DetailData {
    pub item: Item,
    pub loading: bool,
    pub seasons: Vec<Item>,
    pub season_id: Option<String>,
    pub episodes: Vec<Item>,
}

#[derive(Default)]
pub struct SearchData {
    pub query: String,
    pub loading: bool,
    pub results: Vec<Item>,
}

pub enum Page {
    Home(HomeData),
    Library(LibraryData),
    Detail(DetailData),
    Search(SearchData),
}

pub struct Jellyui {
    pub config: Config,
    pub screen: Screen,
    pub session: Option<Session>,
    pub toasts: Entity<ToastState>,
    pub player: Player,
    pub player_status: PlayerStatus,
    player_poll: Option<Task<()>>,
    pub connect: ConnectState,
    pub views: Vec<Item>,
    pub page: Page,
    pub history: Vec<Page>,
    pub profile_menu: Entity<MenuState>,
    pub search_input: Entity<InputState>,
    pub sidebar_collapsed: bool,
    /// Whether the embedded video view covers the content area.
    pub player_open: bool,
    pub current_frame: Option<Arc<RenderImage>>,
    frame_seq: u64,
    pub seek_slider: Entity<SliderState>,
    pub scrubbing: bool,
    pub audio_menu: Entity<MenuState>,
    pub subtitle_menu: Entity<MenuState>,
    tracks_version: u64,
    pub player_focus: FocusHandle,
    /// Whether the player overlay controls are shown; hidden after pointer idle.
    pub controls_visible: bool,
    last_pointer_activity: std::time::Instant,
    generation: u64,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

impl Jellyui {
    pub fn new(config: Config, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let toasts = cx.new(ToastState::new);
        let profile_menu = cx.new(|cx| MenuState::new([], cx));
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search movies, shows, episodes…"));
        let connect = ConnectState::new(window, cx);
        let seek_slider = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.1));
        let audio_menu = cx.new(|cx| MenuState::new([], cx));
        let subtitle_menu = cx.new(|cx| MenuState::new([], cx));

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe_in(&search_input, window, |this, _, event, _, cx| {
                if let crate::ui::input::InputEvent::PressEnter { .. } = event {
                    let query = this.search_input.read(cx).value().to_string();
                    this.open_search(query, cx);
                }
            }),
        );
        subscriptions.extend(connect.subscribe(window, cx));
        subscriptions.push(cx.subscribe(
            &seek_slider,
            |this, _, event: &SliderEvent, cx| match event {
                SliderEvent::Change(_) => this.scrubbing = true,
                SliderEvent::Release(value) => {
                    this.scrubbing = false;
                    this.player.seek_absolute(value.end() as f64);
                    cx.notify();
                }
            },
        ));

        let mut this = Self {
            config,
            screen: Screen::Connect,
            session: None,
            toasts,
            player: Player::default(),
            player_status: PlayerStatus::default(),
            player_poll: None,
            connect,
            views: Vec::new(),
            page: Page::Home(HomeData::default()),
            history: Vec::new(),
            profile_menu,
            search_input,
            sidebar_collapsed: false,
            player_open: false,
            current_frame: None,
            frame_seq: 0,
            seek_slider,
            scrubbing: false,
            audio_menu,
            subtitle_menu,
            tracks_version: 0,
            player_focus: cx.focus_handle(),
            controls_visible: true,
            last_pointer_activity: std::time::Instant::now(),
            generation: 0,
            _subscriptions: subscriptions,
        };

        if let Some((server_id, user_id)) = this.config.active.clone() {
            this.open_session(&server_id, &user_id, cx);
        }
        this.connect.selected_server = this
            .config
            .active
            .as_ref()
            .map(|(s, _)| s.clone())
            .or_else(|| this.config.servers.first().map(|s| s.id.clone()));
        this.rebuild_menu(cx);
        this
    }

    // ----- infrastructure ---------------------------------------------------

    pub fn toast(&self, title: impl Into<String>, description: impl Into<String>, cx: &mut App) {
        let title = title.into();
        let description = description.into();
        let id = format!("toast-{}", uuid::Uuid::new_v4());
        self.toasts.update(cx, |toasts, cx| {
            toasts.push(id, title, description, Some(Duration::from_secs(6)), cx);
        });
    }

    pub fn save_config(&self, cx: &mut App) {
        if let Err(err) = self.config.save() {
            log::error!("saving config failed: {err:#}");
            self.toast("Could not save settings", format!("{err:#}"), cx);
        }
    }

    /// Runs blocking work with a client on a background thread and hands the
    /// result back to this view.
    pub fn fetch_with<T, W, D>(&self, client: Client, cx: &mut Context<Self>, work: W, done: D)
    where
        T: Send + 'static,
        W: FnOnce(Client) -> Result<T> + Send + 'static,
        D: FnOnce(&mut Self, Result<T>, &mut Context<Self>) + 'static,
    {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { work(client) })
                .await;
            this.update(cx, |this, cx| done(this, result, cx)).ok();
        })
        .detach();
    }

    pub fn fetch<T, W, D>(&self, cx: &mut Context<Self>, work: W, done: D)
    where
        T: Send + 'static,
        W: FnOnce(Client) -> Result<T> + Send + 'static,
        D: FnOnce(&mut Self, Result<T>, &mut Context<Self>) + 'static,
    {
        let Some(client) = self.session.as_ref().map(|s| s.client.clone()) else {
            return;
        };
        self.fetch_with(client, cx, work, done);
    }

    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    // ----- sessions ---------------------------------------------------------

    pub fn open_session(&mut self, server_id: &str, user_id: &str, cx: &mut Context<Self>) -> bool {
        let Some(server) = self.config.server(server_id) else {
            return false;
        };
        let Some(profile) = server.profiles.iter().find(|p| p.user_id == user_id) else {
            return false;
        };
        let client = Client::new(&server.url, &self.config.device_id)
            .with_session(&profile.token, &profile.user_id);
        let user_image = profile
            .image_tag
            .as_ref()
            .map(|tag| client.user_image_url(&profile.user_id, tag));
        self.session = Some(Session {
            server_id: server.id.clone(),
            server_name: server.name.clone(),
            user_id: profile.user_id.clone(),
            user_name: profile.name.clone(),
            user_image,
            client,
        });
        self.config.active = Some((server_id.to_string(), user_id.to_string()));
        self.save_config(cx);
        self.screen = Screen::Main;
        self.history.clear();
        self.views.clear();
        self.page = Page::Home(HomeData::default());
        self.load_views(cx);
        self.load_page(cx);
        self.rebuild_menu(cx);
        cx.notify();
        true
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session.take() {
            self.config
                .remove_profile(&session.server_id, &session.user_id);
            self.connect.selected_server = Some(session.server_id.clone());
        }
        self.config.active = None;
        self.save_config(cx);
        self.screen = Screen::Connect;
        self.rebuild_menu(cx);
        cx.notify();
    }

    pub fn show_connect(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            self.connect.selected_server = Some(session.server_id.clone());
        }
        self.connect.error = None;
        self.screen = Screen::Connect;
        cx.notify();
    }

    pub fn add_profile(&mut self, server_id: &str, profile: Profile, cx: &mut Context<Self>) {
        self.config.upsert_profile(server_id, profile.clone());
        self.save_config(cx);
        self.open_session(server_id, &profile.user_id, cx);
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let dark = !self.config.dark.unwrap_or(true);
        self.config.dark = Some(dark);
        self.save_config(cx);
        UiTheme::set(cx, theme(dark));
        self.rebuild_menu(cx);
        cx.notify();
    }

    fn load_views(&mut self, cx: &mut Context<Self>) {
        self.fetch(
            cx,
            |client| client.views(),
            |this, result, cx| {
                match result {
                    Ok(views) => {
                        this.views = views
                            .into_iter()
                            .filter(|v| {
                                !matches!(
                                    v.collection_type.as_deref(),
                                    Some("playlists")
                                        | Some("livetv")
                                        | Some("books")
                                        | Some("music")
                                )
                            })
                            .collect();
                        if let Page::Home(_) = this.page {
                            this.load_page(cx);
                        }
                    }
                    Err(err) => this.toast("Could not load libraries", format!("{err:#}"), cx),
                }
                cx.notify();
            },
        );
    }

    // ----- navigation -------------------------------------------------------

    pub fn navigate(&mut self, page: Page, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.page, page);
        self.history.push(previous);
        if self.history.len() > 40 {
            self.history.remove(0);
        }
        self.load_page(cx);
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.history.pop() {
            self.page = page;
            self.next_generation();
            cx.notify();
        }
    }

    pub fn open_home(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        self.page = Page::Home(HomeData::default());
        self.load_page(cx);
        cx.notify();
    }

    pub fn open_library(&mut self, view: Item, cx: &mut Context<Self>) {
        self.history.clear();
        let title = view.name.clone();
        self.page = Page::Library(LibraryData {
            view,
            title,
            loading: true,
            items: Vec::new(),
            total: 0,
            sort: "SortName",
        });
        self.load_page(cx);
        cx.notify();
    }

    pub fn open_folder(&mut self, folder: Item, cx: &mut Context<Self>) {
        let title = folder.name.clone();
        self.navigate(
            Page::Library(LibraryData {
                view: folder,
                title,
                loading: true,
                items: Vec::new(),
                total: 0,
                sort: "SortName",
            }),
            cx,
        );
    }

    pub fn open_item(&mut self, item: Item, cx: &mut Context<Self>) {
        match item.kind.as_str() {
            "Season" => {
                let Some(series_id) = item.series_id.clone() else {
                    return;
                };
                let season_id = item.id.clone();
                let mut series = Item {
                    id: series_id,
                    kind: "Series".into(),
                    ..Default::default()
                };
                series.name = item.series_name.clone().unwrap_or_default();
                self.navigate(
                    Page::Detail(DetailData {
                        item: series,
                        loading: true,
                        seasons: Vec::new(),
                        season_id: Some(season_id),
                        episodes: Vec::new(),
                    }),
                    cx,
                );
            }
            "CollectionFolder" | "UserView" | "Folder" | "BoxSet" | "Playlist" => {
                self.open_folder(item, cx)
            }
            _ => self.navigate(
                Page::Detail(DetailData {
                    item,
                    loading: true,
                    seasons: Vec::new(),
                    season_id: None,
                    episodes: Vec::new(),
                }),
                cx,
            ),
        }
    }

    pub fn open_search(&mut self, query: String, cx: &mut Context<Self>) {
        let query = query.trim().to_string();
        if let Page::Search(data) = &mut self.page {
            data.query = query;
        } else {
            self.navigate(
                Page::Search(SearchData {
                    query,
                    ..Default::default()
                }),
                cx,
            );
            return;
        }
        self.load_page(cx);
        cx.notify();
    }

    pub fn select_season(&mut self, season_id: String, cx: &mut Context<Self>) {
        if let Page::Detail(data) = &mut self.page {
            data.season_id = Some(season_id.clone());
            data.episodes.clear();
            data.loading = true;
            let series_id = data.item.id.clone();
            let generation = self.next_generation();
            self.fetch(
                cx,
                move |client| client.episodes(&series_id, &season_id),
                move |this, result, cx| {
                    if this.generation != generation {
                        return;
                    }
                    if let Page::Detail(data) = &mut this.page {
                        data.loading = false;
                        match result {
                            Ok(episodes) => data.episodes = episodes,
                            Err(err) => {
                                this.toast("Could not load episodes", format!("{err:#}"), cx)
                            }
                        }
                    }
                    cx.notify();
                },
            );
            cx.notify();
        }
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        let Page::Library(data) = &mut self.page else {
            return;
        };
        if data.loading || data.items.len() >= data.total {
            return;
        }
        data.loading = true;
        let query = library_query(&data.view, data.sort, data.items.len());
        let generation = self.generation;
        self.fetch(
            cx,
            move |client| client.items(&query),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                if let Page::Library(data) = &mut this.page {
                    data.loading = false;
                    match result {
                        Ok(page) => {
                            data.total = page.total_record_count;
                            data.items.extend(page.items);
                        }
                        Err(err) => this.toast("Could not load more", format!("{err:#}"), cx),
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    pub fn set_sort(&mut self, sort: &'static str, cx: &mut Context<Self>) {
        if let Page::Library(data) = &mut self.page {
            data.sort = sort;
            data.items.clear();
            data.total = 0;
            self.load_page(cx);
            cx.notify();
        }
    }

    /// (Re)loads the data for the current page.
    pub fn load_page(&mut self, cx: &mut Context<Self>) {
        let generation = self.next_generation();
        match &mut self.page {
            Page::Home(data) => {
                data.loading = true;
                let libraries: Vec<Item> = self
                    .views
                    .iter()
                    .filter(|v| {
                        matches!(
                            v.collection_type.as_deref(),
                            Some("movies") | Some("tvshows") | Some("homevideos") | None
                        )
                    })
                    .cloned()
                    .collect();
                self.fetch(
                    cx,
                    move |client| {
                        let resume = client.resume(20)?;
                        let next_up = client.next_up(20)?;
                        let mut latest = Vec::new();
                        for library in libraries {
                            let items = client.latest(&library.id, 16)?;
                            if !items.is_empty() {
                                latest.push((library, items));
                            }
                        }
                        Ok(HomeData {
                            loading: false,
                            resume,
                            next_up,
                            latest,
                        })
                    },
                    move |this, result, cx| {
                        if this.generation != generation {
                            return;
                        }
                        if let Page::Home(data) = &mut this.page {
                            match result {
                                Ok(fresh) => *data = fresh,
                                Err(err) => {
                                    data.loading = false;
                                    this.toast("Could not load home", format!("{err:#}"), cx);
                                }
                            }
                        }
                        cx.notify();
                    },
                );
            }
            Page::Library(data) => {
                data.loading = true;
                let query = library_query(&data.view, data.sort, 0);
                self.fetch(
                    cx,
                    move |client| client.items(&query),
                    move |this, result, cx| {
                        if this.generation != generation {
                            return;
                        }
                        if let Page::Library(data) = &mut this.page {
                            data.loading = false;
                            match result {
                                Ok(page) => {
                                    data.total = page.total_record_count;
                                    data.items = page.items;
                                }
                                Err(err) => {
                                    this.toast("Could not load library", format!("{err:#}"), cx)
                                }
                            }
                        }
                        cx.notify();
                    },
                );
            }
            Page::Detail(data) => {
                data.loading = true;
                let item_id = data.item.id.clone();
                let is_series = data.item.is_series();
                let wanted_season = data.season_id.clone();
                self.fetch(
                    cx,
                    move |client| {
                        let item = client.item(&item_id)?;
                        if !is_series {
                            return Ok((item, Vec::new(), None, Vec::new()));
                        }
                        let seasons = client.seasons(&item_id)?;
                        let season = wanted_season
                            .and_then(|id| seasons.iter().find(|s| s.id == id))
                            .or_else(|| {
                                seasons.iter().find(|s| {
                                    s.user_data.unplayed_item_count.unwrap_or(0) > 0
                                        && s.index_number != Some(0)
                                })
                            })
                            .or_else(|| seasons.iter().find(|s| s.index_number != Some(0)))
                            .or_else(|| seasons.first())
                            .map(|s| s.id.clone());
                        let episodes = match &season {
                            Some(season_id) => client.episodes(&item_id, season_id)?,
                            None => Vec::new(),
                        };
                        Ok((item, seasons, season, episodes))
                    },
                    move |this, result, cx| {
                        if this.generation != generation {
                            return;
                        }
                        if let Page::Detail(data) = &mut this.page {
                            data.loading = false;
                            match result {
                                Ok((item, seasons, season_id, episodes)) => {
                                    data.item = item;
                                    data.seasons = seasons;
                                    data.season_id = season_id;
                                    data.episodes = episodes;
                                }
                                Err(err) => {
                                    this.toast("Could not load details", format!("{err:#}"), cx)
                                }
                            }
                        }
                        cx.notify();
                    },
                );
            }
            Page::Search(data) => {
                if data.query.is_empty() {
                    data.results.clear();
                    data.loading = false;
                    return;
                }
                data.loading = true;
                let query = ItemQuery {
                    search: Some(data.query.clone()),
                    include_types: vec!["Movie".into(), "Series".into(), "Episode".into()],
                    recursive: Some(true),
                    limit: Some(60),
                    ..Default::default()
                };
                self.fetch(
                    cx,
                    move |client| client.items(&query),
                    move |this, result, cx| {
                        if this.generation != generation {
                            return;
                        }
                        if let Page::Search(data) = &mut this.page {
                            data.loading = false;
                            match result {
                                Ok(page) => data.results = page.items,
                                Err(err) => this.toast("Search failed", format!("{err:#}"), cx),
                            }
                        }
                        cx.notify();
                    },
                );
            }
        }
    }

    // ----- user data --------------------------------------------------------

    /// Flips the watched state of the item shown on the detail page.
    pub fn toggle_played(&mut self, cx: &mut Context<Self>) {
        let Page::Detail(data) = &self.page else {
            return;
        };
        let item_id = data.item.id.clone();
        let played = !data.item.user_data.played;
        let generation = self.generation;
        self.fetch(
            cx,
            move |client| client.set_played(&item_id, played),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(user_data) => {
                        if let Page::Detail(data) = &mut this.page {
                            data.item.user_data = user_data;
                            if data.item.is_series() {
                                // Episodes and season counts changed server-side too.
                                this.load_page(cx);
                            }
                        }
                    }
                    Err(err) => {
                        this.toast("Could not update watched state", format!("{err:#}"), cx)
                    }
                }
                cx.notify();
            },
        );
    }

    /// Flips the favorite state of the item shown on the detail page.
    pub fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        let Page::Detail(data) = &self.page else {
            return;
        };
        let item_id = data.item.id.clone();
        let favorite = !data.item.user_data.is_favorite;
        let generation = self.generation;
        self.fetch(
            cx,
            move |client| client.set_favorite(&item_id, favorite),
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(user_data) => {
                        if let Page::Detail(data) = &mut this.page {
                            data.item.user_data = user_data;
                        }
                    }
                    Err(err) => this.toast("Could not update favorite", format!("{err:#}"), cx),
                }
                cx.notify();
            },
        );
    }

    // ----- playback ---------------------------------------------------------

    /// Plays the next unwatched episode of a series, or the first episode
    /// when everything has been watched.
    pub fn play_series(&mut self, series: &Item, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.session.as_ref().map(|s| s.client.clone()) else {
            return;
        };
        let series_id = series.id.clone();
        let generation = self.generation;
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Item>> = cx
                .background_executor()
                .spawn(async move {
                    if let Some(next) = client.series_next_up(&series_id)? {
                        return Ok(Some(next));
                    }
                    let seasons = client.seasons(&series_id)?;
                    let Some(first) = seasons
                        .iter()
                        .find(|s| s.index_number != Some(0))
                        .or_else(|| seasons.first())
                    else {
                        return Ok(None);
                    };
                    let episodes = client.episodes(&series_id, &first.id)?;
                    Ok(episodes.into_iter().next())
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(Some(episode)) => {
                        let resume = episode.resume_secs() > 0;
                        this.play(&episode, resume, window, cx);
                    }
                    Ok(None) => this.toast("Nothing to play", "This series has no episodes.", cx),
                    Err(err) => this.toast("Could not start playback", format!("{err:#}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn play(&mut self, item: &Item, resume: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return };
        let start_secs = if resume { item.resume_secs() } else { 0 };
        self.player.play(PlayRequest {
            client: session.client.clone(),
            item_id: item.id.clone(),
            url: session.client.stream_url(&item.id),
            title: item.display_title(),
            start_secs,
        });
        self.player_status = self.player.status();
        self.player_open = true;
        window.focus(&self.player_focus, cx);
        self.show_controls();
        self.tracks_version = 0;
        self.rebuild_track_menus(cx);
        self.start_player_poll(cx);
        cx.notify();
    }

    /// Called on pointer activity over the video; controls hide again after a delay.
    pub fn show_controls(&mut self) {
        self.last_pointer_activity = std::time::Instant::now();
        self.controls_visible = true;
    }

    /// Controls stay while paused, buffering, scrubbing, or when a track menu is open.
    fn controls_pinned(&self, cx: &App) -> bool {
        self.player_status.paused
            || self.player_status.state != PlayState::Playing
            || self.scrubbing
            || self.audio_menu.read(cx).is_open()
            || self.subtitle_menu.read(cx).is_open()
    }

    pub fn close_player_view(&mut self, cx: &mut Context<Self>) {
        self.player_open = false;
        cx.notify();
    }

    pub fn open_player_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.player_status.state != PlayState::Idle {
            self.player_open = true;
            window.focus(&self.player_focus, cx);
            cx.notify();
        }
    }

    /// Swaps in the newest video frame and releases the previous GPU texture.
    fn sync_frame(&mut self, window: &mut Window) {
        let seq = self.player.frame_seq();
        if seq == self.frame_seq {
            return;
        }
        self.frame_seq = seq;
        let (_, frame) = self.player.frame();
        if let Some(old) = std::mem::replace(&mut self.current_frame, frame) {
            let _ = window.drop_image(old);
        }
    }

    /// Rebuilds the audio and subtitle menus from the current track list.
    pub fn rebuild_track_menus(&mut self, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let status = &self.player_status;
        let build = |kind: &str, none_label: &str, set: fn(&Player, Option<i64>)| {
            let mut items = Vec::new();
            let tracks: Vec<&crate::player::Track> =
                status.tracks.iter().filter(|t| t.kind == kind).collect();
            let any_selected = tracks.iter().any(|t| t.selected);
            if kind == "sub" {
                let handle = this.clone();
                items.push(
                    MenuItem::new(
                        gpui_kit::SharedString::from(format!("track.{kind}.none")),
                        none_label,
                    )
                    .radio(!any_selected)
                    .on_click(move |_, _, cx| {
                        handle.update(cx, |t, _| set(&t.player, None)).ok();
                    }),
                );
            }
            for track in tracks {
                let id = track.id;
                let handle = this.clone();
                items.push(
                    MenuItem::new(
                        gpui_kit::SharedString::from(format!("track.{kind}.{id}")),
                        track.label(),
                    )
                    .radio(track.selected)
                    .on_click(move |_, _, cx| {
                        handle.update(cx, |t, _| set(&t.player, Some(id))).ok();
                    }),
                );
            }
            if items.is_empty() {
                items.push(
                    MenuItem::new(
                        gpui_kit::SharedString::from(format!("track.{kind}.empty")),
                        "No tracks",
                    )
                    .disabled(true),
                );
            }
            items
        };
        let audio = build("audio", "", |p, id| p.set_audio(id));
        let subs = build("sub", "Off", |p, id| p.set_subtitle(id));
        self.audio_menu
            .update(cx, |menu, cx| menu.set_items(audio, cx));
        self.subtitle_menu
            .update(cx, |menu, cx| menu.set_items(subs, cx));
    }

    fn start_player_poll(&mut self, cx: &mut Context<Self>) {
        self.player_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let fast = this
                    .read_with(cx, |this, _| this.player_open)
                    .unwrap_or(false);
                cx.background_executor()
                    .timer(Duration::from_millis(if fast { 8 } else { 500 }))
                    .await;
                let keep_going = this
                    .update(cx, |this, cx| {
                        let status = this.player.status();
                        let ended = status.state == PlayState::Ended;
                        let frame_changed = this.player.frame_seq() != this.frame_seq;
                        let changed = frame_changed
                            || status.position != this.player_status.position
                            || status.paused != this.player_status.paused
                            || status.buffering != this.player_status.buffering
                            || status.state != this.player_status.state
                            || status.tracks_version != this.player_status.tracks_version;
                        this.player_status = status;
                        if this.player_status.tracks_version != this.tracks_version {
                            this.tracks_version = this.player_status.tracks_version;
                            this.rebuild_track_menus(cx);
                        }
                        if !this.scrubbing && this.player_status.duration > 0. {
                            let (pos, dur) = (
                                this.player_status.position as f32,
                                this.player_status.duration as f32,
                            );
                            this.seek_slider.update(cx, |s, cx| {
                                *s = SliderState::new()
                                    .min(0.)
                                    .max(dur)
                                    .step(1.)
                                    .default_value(pos);
                                cx.notify();
                            });
                        }
                        if ended {
                            this.player_open = false;
                            if let Some(err) = this.player_status.error.clone() {
                                this.toast("Playback stopped", err, cx);
                            }
                            this.player.acknowledge_end();
                            this.player_status = this.player.status();
                            this.load_page(cx);
                        }
                        let idle = this.last_pointer_activity.elapsed() > CONTROLS_HIDE_AFTER;
                        let should_show = !this.player_open || !idle || this.controls_pinned(cx);
                        let controls_changed = should_show != this.controls_visible;
                        this.controls_visible = should_show;
                        if changed || ended || controls_changed {
                            cx.notify();
                        }
                        !ended
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }
}

pub fn library_query(view: &Item, sort: &str, start: usize) -> ItemQuery {
    let include_types = match view.collection_type.as_deref() {
        Some("movies") => vec!["Movie".to_string()],
        Some("tvshows") => vec!["Series".to_string()],
        _ => Vec::new(),
    };
    let recursive = !include_types.is_empty();
    ItemQuery {
        parent_id: Some(view.id.clone()),
        include_types,
        recursive: Some(recursive),
        sort_by: Some(sort.to_string()),
        sort_order: Some(
            if sort == "SortName" {
                "Ascending"
            } else {
                "Descending"
            }
            .to_string(),
        ),
        limit: Some(PAGE_SIZE),
        start: Some(start),
        ..Default::default()
    }
}

impl Render for Jellyui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_frame(window);
        let theme = UiTheme::read(cx).clone();
        let content = match self.screen {
            Screen::Connect => self.render_connect(window, cx).into_any_element(),
            Screen::Main => self.render_shell(window, cx).into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .bg(theme.colors.background)
            .text_color(theme.colors.foreground)
            .font_family(theme.fonts.body.clone())
            .text_size(px(14.))
            .line_height(px(20.))
            .child(content)
            .child(self.toasts.clone())
    }
}
