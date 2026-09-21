// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Minimal blocking Jellyfin REST client. Call it from a background thread.

use std::{sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::{APP_NAME, APP_VERSION};

pub const TICKS_PER_SECOND: i64 = 10_000_000;

#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    pub base: Arc<str>,
    pub device_id: Arc<str>,
    pub token: Option<Arc<str>>,
    pub user_id: Option<Arc<str>>,
}

impl Client {
    pub fn new(base: &str, device_id: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            base: base.trim_end_matches('/').into(),
            device_id: device_id.into(),
            token: None,
            user_id: None,
        }
    }

    pub fn with_session(mut self, token: &str, user_id: &str) -> Self {
        self.token = Some(token.into());
        self.user_id = Some(user_id.into());
        self
    }

    fn user(&self) -> Result<&str> {
        self.user_id
            .as_deref()
            .ok_or_else(|| anyhow!("not signed in"))
    }

    fn auth_header(&self) -> String {
        let device = hostname();
        let mut value = format!(
            "MediaBrowser Client=\"{APP_NAME}\", Device=\"{device}\", DeviceId=\"{}\", Version=\"{APP_VERSION}\"",
            self.device_id
        );
        if let Some(token) = &self.token {
            value.push_str(&format!(", Token=\"{token}\""));
        }
        value
    }

    /// Header for mpv so it can fetch the stream with this session. mpv splits
    /// header lists on commas, so only the comma-free token form is used.
    pub fn mpv_auth_header(&self) -> String {
        match &self.token {
            Some(token) => format!("Authorization: MediaBrowser Token=\"{token}\""),
            None => String::new(),
        }
    }

    fn url(&self, path: &str, query: &[(&str, String)]) -> String {
        let mut url = format!("{}{}", self.base, path);
        let mut first = true;
        for (k, v) in query {
            if v.is_empty() {
                continue;
            }
            url.push(if first { '?' } else { '&' });
            first = false;
            url.push_str(k);
            url.push('=');
            url.push_str(&urlencode(v));
        }
        url
    }

    fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let url = self.url(path, query);
        let mut response = self
            .agent
            .get(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .call()
            .map_err(|e| map_err(e, path))?;
        response
            .body_mut()
            .read_json::<T>()
            .with_context(|| format!("decode {path}"))
    }

    fn post<B: Serialize>(&self, path: &str, body: &B) -> Result<ureq::Body> {
        let url = self.url(path, &[]);
        let response = self
            .agent
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .send_json(body)
            .map_err(|e| map_err(e, path))?;
        Ok(response.into_body())
    }

    /// Body-less request (POST/DELETE) that decodes a JSON response.
    fn send<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let url = self.url(path, query);
        let request = match method {
            "DELETE" => self.agent.delete(&url).force_send_body(),
            _ => self.agent.post(&url),
        };
        let mut response = request
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .send_empty()
            .map_err(|e| map_err(e, path))?;
        response
            .body_mut()
            .read_json::<T>()
            .with_context(|| format!("decode {path}"))
    }

    // ----- Discovery & auth -------------------------------------------------

    pub fn public_info(&self) -> Result<PublicSystemInfo> {
        self.get("/System/Info/Public", &[])
    }

    pub fn public_users(&self) -> Result<Vec<User>> {
        self.get("/Users/Public", &[])
    }

    pub fn authenticate(&self, username: &str, password: &str) -> Result<AuthResult> {
        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        struct Body<'a> {
            username: &'a str,
            pw: &'a str,
        }
        let mut body = self.post(
            "/Users/AuthenticateByName",
            &Body {
                username,
                pw: password,
            },
        )?;
        body.read_json().context("decode auth response")
    }

    // ----- Browsing ---------------------------------------------------------

    pub fn views(&self) -> Result<Vec<Item>> {
        let user = self.user()?.to_string();
        let result: ItemsResult = self.get("/UserViews", &[("userId", user)])?;
        Ok(result.items)
    }

    pub fn item(&self, id: &str) -> Result<Item> {
        let user = self.user()?.to_string();
        self.get(&format!("/Items/{id}"), &[("userId", user)])
    }

    pub fn items(&self, q: &ItemQuery) -> Result<ItemsResult> {
        let user = self.user()?.to_string();
        let mut params: Vec<(&str, String)> = vec![
            ("userId", user),
            ("parentId", q.parent_id.clone().unwrap_or_default()),
            ("includeItemTypes", q.include_types.join(",")),
            (
                "recursive",
                q.recursive.map(|b| b.to_string()).unwrap_or_default(),
            ),
            ("sortBy", q.sort_by.clone().unwrap_or_default()),
            ("sortOrder", q.sort_order.clone().unwrap_or_default()),
            ("searchTerm", q.search.clone().unwrap_or_default()),
            ("limit", q.limit.map(|n| n.to_string()).unwrap_or_default()),
            (
                "startIndex",
                q.start.map(|n| n.to_string()).unwrap_or_default(),
            ),
            ("fields", ITEM_FIELDS.to_string()),
            ("enableImageTypes", "Primary,Backdrop,Thumb".to_string()),
            ("imageTypeLimit", "1".to_string()),
        ];
        if let Some(filters) = &q.filters {
            params.push(("filters", filters.clone()));
        }
        self.get("/Items", &params)
    }

    pub fn resume(&self, limit: usize) -> Result<Vec<Item>> {
        let user = self.user()?.to_string();
        let result: ItemsResult = self.get(
            "/UserItems/Resume",
            &[
                ("userId", user),
                ("limit", limit.to_string()),
                ("fields", ITEM_FIELDS.to_string()),
                ("mediaTypes", "Video".to_string()),
                ("enableImageTypes", "Primary,Backdrop,Thumb".to_string()),
            ],
        )?;
        Ok(result.items)
    }

    pub fn next_up(&self, limit: usize) -> Result<Vec<Item>> {
        let user = self.user()?.to_string();
        let result: ItemsResult = self.get(
            "/Shows/NextUp",
            &[
                ("userId", user),
                ("limit", limit.to_string()),
                ("fields", ITEM_FIELDS.to_string()),
                ("enableImageTypes", "Primary,Backdrop,Thumb".to_string()),
            ],
        )?;
        Ok(result.items)
    }

    pub fn latest(&self, parent_id: &str, limit: usize) -> Result<Vec<Item>> {
        let user = self.user()?.to_string();
        self.get(
            "/Items/Latest",
            &[
                ("userId", user),
                ("parentId", parent_id.to_string()),
                ("limit", limit.to_string()),
                ("fields", ITEM_FIELDS.to_string()),
                ("enableImageTypes", "Primary,Backdrop,Thumb".to_string()),
            ],
        )
    }

    pub fn seasons(&self, series_id: &str) -> Result<Vec<Item>> {
        let user = self.user()?.to_string();
        let result: ItemsResult = self.get(
            &format!("/Shows/{series_id}/Seasons"),
            &[("userId", user), ("fields", ITEM_FIELDS.to_string())],
        )?;
        Ok(result.items)
    }

    pub fn episodes(&self, series_id: &str, season_id: &str) -> Result<Vec<Item>> {
        let user = self.user()?.to_string();
        let result: ItemsResult = self.get(
            &format!("/Shows/{series_id}/Episodes"),
            &[
                ("userId", user),
                ("seasonId", season_id.to_string()),
                ("fields", ITEM_FIELDS.to_string()),
            ],
        )?;
        Ok(result.items)
    }

    /// The next unwatched episode of a series, if any.
    pub fn series_next_up(&self, series_id: &str) -> Result<Option<Item>> {
        let user = self.user()?.to_string();
        let result: ItemsResult = self.get(
            "/Shows/NextUp",
            &[
                ("userId", user),
                ("seriesId", series_id.to_string()),
                ("limit", "1".to_string()),
                ("fields", ITEM_FIELDS.to_string()),
            ],
        )?;
        Ok(result.items.into_iter().next())
    }

    // ----- User data --------------------------------------------------------

    pub fn set_played(&self, item_id: &str, played: bool) -> Result<UserData> {
        let user = self.user()?.to_string();
        let method = if played { "POST" } else { "DELETE" };
        self.send(
            method,
            &format!("/UserPlayedItems/{item_id}"),
            &[("userId", user)],
        )
    }

    pub fn set_favorite(&self, item_id: &str, favorite: bool) -> Result<UserData> {
        let user = self.user()?.to_string();
        let method = if favorite { "POST" } else { "DELETE" };
        self.send(
            method,
            &format!("/UserFavoriteItems/{item_id}"),
            &[("userId", user)],
        )
    }

    // ----- Media URLs -------------------------------------------------------

    /// Image endpoints are anonymous on Jellyfin, so the URL can be handed to
    /// gpui's image loader directly.
    pub fn image_url(
        &self,
        item_id: &str,
        kind: &str,
        tag: Option<&str>,
        max_width: u32,
    ) -> String {
        let mut url = format!(
            "{}/Items/{item_id}/Images/{kind}?maxWidth={max_width}&quality=90",
            self.base
        );
        if let Some(tag) = tag {
            url.push_str("&tag=");
            url.push_str(tag);
        }
        url
    }

    pub fn stream_url(&self, item_id: &str) -> String {
        format!(
            "{}/Videos/{item_id}/stream?static=true&mediaSourceId={item_id}",
            self.base
        )
    }

    pub fn user_image_url(&self, user_id: &str, tag: &str) -> String {
        format!(
            "{}/Users/{user_id}/Images/Primary?maxWidth=96&tag={tag}",
            self.base
        )
    }

    // ----- Playback reporting ----------------------------------------------

    pub fn report_start(&self, p: &Progress) -> Result<()> {
        self.post("/Sessions/Playing", &p.body(true)).map(drop)
    }

    pub fn report_progress(&self, p: &Progress) -> Result<()> {
        self.post("/Sessions/Playing/Progress", &p.body(true))
            .map(drop)
    }

    pub fn report_stopped(&self, p: &Progress) -> Result<()> {
        self.post("/Sessions/Playing/Stopped", &p.body(false))
            .map(drop)
    }
}

const ITEM_FIELDS: &str =
    "Overview,PrimaryImageAspectRatio,ProductionYear,Genres,ParentId,DateCreated";

fn map_err(err: ureq::Error, path: &str) -> anyhow::Error {
    match err {
        ureq::Error::StatusCode(401) => anyhow!("unauthorized (401) at {path}"),
        ureq::Error::StatusCode(code) => anyhow!("HTTP {code} at {path}"),
        other => anyhow!("{other} ({path})"),
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Mac".to_string())
}

// ----- Types -----------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ItemQuery {
    pub parent_id: Option<String>,
    pub include_types: Vec<String>,
    pub recursive: Option<bool>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub search: Option<String>,
    pub filters: Option<String>,
    pub limit: Option<usize>,
    pub start: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicSystemInfo {
    pub id: String,
    pub server_name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct User {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub primary_image_tag: Option<String>,
    #[serde(default)]
    pub has_password: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthResult {
    pub user: User,
    pub access_token: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemsResult {
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub total_record_count: usize,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "Type", default)]
    pub kind: String,
    #[serde(default)]
    pub collection_type: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub production_year: Option<i32>,
    #[serde(default)]
    pub run_time_ticks: Option<i64>,
    #[serde(default)]
    pub community_rating: Option<f32>,
    #[serde(default)]
    pub official_rating: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub series_name: Option<String>,
    #[serde(default)]
    pub series_id: Option<String>,
    #[serde(default)]
    pub index_number: Option<i32>,
    #[serde(default)]
    pub parent_index_number: Option<i32>,
    #[serde(default)]
    pub image_tags: ImageTags,
    #[serde(default)]
    pub backdrop_image_tags: Vec<String>,
    #[serde(default)]
    pub parent_backdrop_item_id: Option<String>,
    #[serde(default)]
    pub parent_backdrop_image_tags: Vec<String>,
    #[serde(default)]
    pub series_primary_image_tag: Option<String>,
    #[serde(default)]
    pub user_data: UserData,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageTags {
    #[serde(default)]
    pub primary: Option<String>,
    #[serde(default)]
    pub thumb: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserData {
    #[serde(default)]
    pub playback_position_ticks: i64,
    #[serde(default)]
    pub played: bool,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub unplayed_item_count: Option<i32>,
}

impl Item {
    pub fn is_playable(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "Movie" | "Episode" | "Video" | "MusicVideo"
        )
    }

    pub fn is_series(&self) -> bool {
        self.kind == "Series"
    }

    pub fn runtime_secs(&self) -> Option<i64> {
        self.run_time_ticks.map(|t| t / TICKS_PER_SECOND)
    }

    pub fn resume_secs(&self) -> i64 {
        self.user_data.playback_position_ticks / TICKS_PER_SECOND
    }

    /// Progress in 0..=1 when the item has been partially watched.
    pub fn progress(&self) -> Option<f32> {
        let pos = self.user_data.playback_position_ticks;
        let total = self.run_time_ticks?;
        (pos > 0 && total > 0).then(|| (pos as f32 / total as f32).clamp(0., 1.))
    }

    /// Poster-style image: own primary, else the series poster for episodes.
    pub fn poster_url(&self, client: &Client, width: u32) -> Option<String> {
        if let Some(tag) = &self.image_tags.primary {
            return Some(client.image_url(&self.id, "Primary", Some(tag), width));
        }
        match (&self.series_id, &self.series_primary_image_tag) {
            (Some(series), Some(tag)) => {
                Some(client.image_url(series, "Primary", Some(tag), width))
            }
            _ => None,
        }
    }

    /// Wide image: episode still, own backdrop, or the parent's backdrop.
    pub fn wide_url(&self, client: &Client, width: u32) -> Option<String> {
        if self.kind == "Episode"
            && let Some(tag) = &self.image_tags.primary
        {
            return Some(client.image_url(&self.id, "Primary", Some(tag), width));
        }
        if let Some(tag) = &self.image_tags.thumb {
            return Some(client.image_url(&self.id, "Thumb", Some(tag), width));
        }
        if let Some(tag) = self.backdrop_image_tags.first() {
            return Some(client.image_url(&self.id, "Backdrop", Some(tag), width));
        }
        match (
            &self.parent_backdrop_item_id,
            self.parent_backdrop_image_tags.first(),
        ) {
            (Some(parent), Some(tag)) => {
                Some(client.image_url(parent, "Backdrop", Some(tag), width))
            }
            _ => None,
        }
    }

    pub fn backdrop_url(&self, client: &Client, width: u32) -> Option<String> {
        if let Some(tag) = self.backdrop_image_tags.first() {
            return Some(client.image_url(&self.id, "Backdrop", Some(tag), width));
        }
        match (
            &self.parent_backdrop_item_id,
            self.parent_backdrop_image_tags.first(),
        ) {
            (Some(parent), Some(tag)) => {
                Some(client.image_url(parent, "Backdrop", Some(tag), width))
            }
            _ => None,
        }
    }

    /// "S2:E5" style label for episodes.
    pub fn episode_code(&self) -> Option<String> {
        match (self.parent_index_number, self.index_number) {
            (Some(s), Some(e)) => Some(format!("S{s}:E{e}")),
            (None, Some(e)) => Some(format!("E{e}")),
            _ => None,
        }
    }

    /// Title used for the player window.
    pub fn display_title(&self) -> String {
        match (&self.series_name, self.episode_code()) {
            (Some(series), Some(code)) => format!("{series} · {code} · {}", self.name),
            (Some(series), None) => format!("{series} · {}", self.name),
            _ => match self.production_year {
                Some(year) if self.kind == "Movie" => format!("{} ({year})", self.name),
                _ => self.name.clone(),
            },
        }
    }
}

/// Playback state reported back to the server.
#[derive(Clone, Debug)]
pub struct Progress {
    pub item_id: String,
    pub play_session_id: String,
    pub position_ticks: i64,
    pub paused: bool,
}

impl Progress {
    fn body(&self, with_state: bool) -> serde_json::Value {
        let mut body = serde_json::json!({
            "ItemId": self.item_id,
            "MediaSourceId": self.item_id,
            "PlaySessionId": self.play_session_id,
            "PositionTicks": self.position_ticks,
            "PlayMethod": "DirectPlay",
            "CanSeek": true,
        });
        if with_state {
            body["IsPaused"] = serde_json::Value::Bool(self.paused);
        }
        body
    }
}

pub fn format_duration(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn format_runtime(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_urls_and_headers() {
        let client =
            Client::new("https://media.example.com/", "dev-1").with_session("tok", "user-1");
        assert_eq!(&*client.base, "https://media.example.com");
        assert_eq!(
            client.url(
                "/Items",
                &[
                    ("userId", "u 1".into()),
                    ("limit", String::new()),
                    ("sortBy", "SortName,Name".into())
                ]
            ),
            "https://media.example.com/Items?userId=u%201&sortBy=SortName,Name"
        );
        assert_eq!(
            client.mpv_auth_header(),
            "Authorization: MediaBrowser Token=\"tok\""
        );
        let full = client.auth_header();
        assert!(full.starts_with("MediaBrowser Client=\"Jellyui\""));
        assert!(full.contains("DeviceId=\"dev-1\"") && full.ends_with("Token=\"tok\""));
        assert_eq!(
            client.stream_url("abc"),
            "https://media.example.com/Videos/abc/stream?static=true&mediaSourceId=abc"
        );
    }

    #[test]
    fn item_helpers() {
        let json = r#"{"Id":"e1","Name":"Pilot","Type":"Episode","SeriesName":"Show","SeriesId":"s1",
            "IndexNumber":1,"ParentIndexNumber":2,"RunTimeTicks":36000000000,
            "UserData":{"PlaybackPositionTicks":18000000000,"Played":false},
            "ImageTags":{},"SeriesPrimaryImageTag":"pt"}"#;
        let item: Item = serde_json::from_str(json).unwrap();
        let client = Client::new("http://h", "d");
        assert_eq!(item.episode_code().as_deref(), Some("S2:E1"));
        assert_eq!(item.display_title(), "Show · S2:E1 · Pilot");
        assert_eq!(item.runtime_secs(), Some(3600));
        assert_eq!(item.resume_secs(), 1800);
        assert_eq!(item.progress(), Some(0.5));
        assert_eq!(
            item.poster_url(&client, 300).unwrap(),
            "http://h/Items/s1/Images/Primary?maxWidth=300&quality=90&tag=pt"
        );
        assert_eq!(format_duration(3725), "1:02:05");
        assert_eq!(format_runtime(5400), "1h 30m");
    }
}
