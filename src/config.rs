// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Persistent app configuration: servers, per-server profiles, active selection.
//!
//! Stored as JSON under the OS config directory with owner-only permissions,
//! because it holds Jellyfin access tokens.

use std::{fs, path::PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "Jellyui";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Stable per-installation id sent to Jellyfin as `DeviceId`.
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub servers: Vec<Server>,
    /// Server id and user id of the profile to open on launch.
    #[serde(default)]
    pub active: Option<(String, String)>,
    #[serde(default)]
    pub dark: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Server {
    /// Jellyfin's own server id from `/System/Info/Public`.
    pub id: String,
    pub name: String,
    /// Base URL without trailing slash, e.g. `https://media.example.com`.
    pub url: String,
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub user_id: String,
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub image_tag: Option<String>,
    #[serde(default)]
    pub autoplay_next: bool,
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("jellyui")
            .join("config.json")
    }

    pub fn load() -> Self {
        let mut config = fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Config>(&bytes).ok())
            .unwrap_or_default();
        if config.device_id.is_empty() {
            config.device_id = uuid::Uuid::new_v4().to_string();
        }
        config
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn server(&self, id: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    pub fn server_mut(&mut self, id: &str) -> Option<&mut Server> {
        self.servers.iter_mut().find(|s| s.id == id)
    }

    /// Inserts or replaces a server by id, keeping existing profiles.
    pub fn upsert_server(&mut self, server: Server) {
        match self.server_mut(&server.id) {
            Some(existing) => {
                existing.name = server.name;
                existing.url = server.url;
            }
            None => self.servers.push(server),
        }
    }

    pub fn upsert_profile(&mut self, server_id: &str, profile: Profile) {
        if let Some(server) = self.server_mut(server_id) {
            server.profiles.retain(|p| p.user_id != profile.user_id);
            server.profiles.push(profile);
        }
    }

    pub fn remove_profile(&mut self, server_id: &str, user_id: &str) {
        if let Some(server) = self.server_mut(server_id) {
            server.profiles.retain(|p| p.user_id != user_id);
        }
        if self
            .active
            .as_ref()
            .is_some_and(|(s, u)| s == server_id && u == user_id)
        {
            self.active = None;
        }
    }

    pub fn remove_server(&mut self, server_id: &str) {
        self.servers.retain(|s| s.id != server_id);
        if self.active.as_ref().is_some_and(|(s, _)| s == server_id) {
            self.active = None;
        }
    }
}

/// Normalizes user input like `media.example.com:8096/` into a base URL.
pub fn normalize_url(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_urls() {
        assert_eq!(
            normalize_url(" media.example.com:8096/ "),
            "http://media.example.com:8096"
        );
        assert_eq!(
            normalize_url("https://x.example.com/"),
            "https://x.example.com"
        );
    }

    #[test]
    fn profiles_round_trip() {
        let mut config = Config::default();
        config.upsert_server(Server {
            id: "s".into(),
            name: "S".into(),
            url: "http://s".into(),
            profiles: vec![],
        });
        config.upsert_profile(
            "s",
            Profile {
                user_id: "u".into(),
                name: "U".into(),
                token: "t".into(),
                image_tag: None,
                autoplay_next: false,
            },
        );
        config.upsert_profile(
            "s",
            Profile {
                user_id: "u".into(),
                name: "U2".into(),
                token: "t2".into(),
                image_tag: None,
                autoplay_next: false,
            },
        );
        assert_eq!(config.server("s").unwrap().profiles.len(), 1);
        assert_eq!(config.server("s").unwrap().profiles[0].name, "U2");
        config.active = Some(("s".into(), "u".into()));
        config.remove_profile("s", "u");
        assert!(config.active.is_none());
        config.upsert_server(Server {
            id: "s".into(),
            name: "Renamed".into(),
            url: "http://s2".into(),
            profiles: vec![],
        });
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.server("s").unwrap().name, "Renamed");
    }
    #[test]
    fn autoplay_is_opt_in_and_saved_per_profile() {
        let old = r#"{"user_id":"u","name":"User","token":"test-token"}"#;
        let mut profile: Profile = serde_json::from_str(old).unwrap();
        assert!(!profile.autoplay_next);
        profile.autoplay_next = true;
        let restored: Profile =
            serde_json::from_str(&serde_json::to_string(&profile).unwrap()).unwrap();
        assert!(restored.autoplay_next);
        let other: Profile = serde_json::from_str(old).unwrap();
        assert!(!other.autoplay_next);
    }
}
