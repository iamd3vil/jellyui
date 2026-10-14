// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
mod app;
mod config;
mod images;
mod jellyfin;
mod keep_awake;
mod player;
#[allow(dead_code, unused_imports)]
mod ui;

/// The installed gpuicn component tests expect `crate::init`.
#[cfg(test)]
pub use ui::theme::init;
mod views;

use std::borrow::Cow;

use gpui_icons::LucideAssetSource;
use gpui_kit::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, point, px, size,
};

use crate::{app::Jellyui, config::Config, ui::theme::UiTheme};

fn main() {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("jellyui=info"));

    gpui_kit::platform::application()
        .with_assets(LucideAssetSource)
        .run(|cx: &mut App| {
            ui::theme::init(cx);
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Regular.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Medium.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Regular.ttf")),
                ])
                .expect("load bundled Geist fonts");

            let config = Config::load();
            UiTheme::set(cx, app::theme(config.dark.unwrap_or(true)));

            let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Jellyui".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(14.), px(14.))),
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.), px(600.))),
                    window_background: WindowBackgroundAppearance::Opaque,
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("Jellyui");
                    cx.new(|cx| Jellyui::new(config, window, cx))
                },
            )
            .expect("open application window");
            cx.activate(true);
        });
}
