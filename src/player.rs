// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Embedded playback with libmpv. One mpv core lives on a worker thread; video
//! is software-rendered into BGRA frames that the UI paints as images.
//! Progress is reported back to Jellyfin from the same thread.

use std::{
    ffi::{CString, c_void},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use gpui_kit::RenderImage;
use libmpv2::{
    Format, Mpv,
    events::{Event, PropertyData},
    mpv_end_file_reason,
};
use libmpv2_sys as sys;
use serde::Deserialize;

use crate::{
    jellyfin::{Client, ExternalSubtitle, Progress, Streams, TICKS_PER_SECOND},
    keep_awake::KeepAwake,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayState {
    #[default]
    Idle,
    /// A file has been requested and is being opened.
    Starting,
    Playing,
    /// Playback finished; the UI should refresh and then acknowledge back to Idle.
    Ended,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Track {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub forced: bool,
    #[serde(default)]
    pub external: bool,
    /// FFmpeg's stream index, which Jellyfin's stream indexes follow.
    #[serde(default)]
    pub ff_index: Option<i64>,
    #[serde(default)]
    pub external_filename: Option<String>,
    #[serde(skip)]
    pub jellyfin_index: Option<i64>,
}

impl Track {
    fn stream_index(&self) -> Option<i64> {
        if self.external {
            self.jellyfin_index
        } else {
            self.ff_index
        }
    }

    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(title) = &self.title {
            parts.push(title.clone());
        }
        if let Some(lang) = &self.lang {
            parts.push(lang.to_uppercase());
        }
        if let Some(codec) = &self.codec {
            parts.push(codec.to_uppercase());
        }
        if self.forced {
            parts.push("forced".into());
        }
        if parts.is_empty() {
            format!("Track {}", self.id)
        } else {
            parts.join(" · ")
        }
    }
}

#[derive(Clone, Debug)]
pub struct PlayerStatus {
    pub state: PlayState,
    pub title: String,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub buffering: bool,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub subtitle_delay: f64,
    pub subtitle_error: Option<String>,
    pub tracks: Vec<Track>,
    /// Bumped whenever `tracks` changes so menus can be rebuilt lazily.
    pub tracks_version: u64,
    pub error: Option<String>,
    /// The file played to its end, rather than being stopped or failing.
    pub finished: bool,
}

impl Default for PlayerStatus {
    fn default() -> Self {
        Self {
            state: PlayState::default(),
            title: String::new(),
            position: 0.,
            duration: 0.,
            paused: false,
            buffering: false,
            volume: 100.,
            muted: false,
            speed: 1.,
            subtitle_delay: 0.,
            subtitle_error: None,
            tracks: Vec::new(),
            tracks_version: 0,
            error: None,
            finished: false,
        }
    }
}

pub struct PlayRequest {
    pub client: Client,
    pub item_id: String,
    pub url: String,
    pub title: String,
    pub start_secs: i64,
}

enum Cmd {
    Load(PlayRequest),
    TogglePause,
    SeekRelative(f64),
    SeekAbsolute(f64),
    SetAudio(Option<i64>),
    SetSubtitle(Option<i64>),
    SetVolume(f64),
    ToggleMute,
    SetSpeed(f64),
    AdjustSubtitleDelay(f64),
    ResetSubtitleDelay,
    Stop,
}

struct Shared {
    status: Mutex<PlayerStatus>,
    frame: Mutex<Option<Arc<RenderImage>>>,
    frame_seq: AtomicU64,
    target_w: AtomicU32,
    target_h: AtomicU32,
}

/// Shared handle owned by the UI. Cheap to clone.
#[derive(Clone)]
pub struct Player {
    shared: Arc<Shared>,
    commands: Arc<Mutex<Option<mpsc::Sender<Cmd>>>>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            shared: Arc::new(Shared {
                status: Mutex::new(PlayerStatus::default()),
                frame: Mutex::new(None),
                frame_seq: AtomicU64::new(0),
                target_w: AtomicU32::new(1280),
                target_h: AtomicU32::new(720),
            }),
            commands: Arc::new(Mutex::new(None)),
        }
    }
}

impl Player {
    pub fn status(&self) -> PlayerStatus {
        self.shared.status.lock().unwrap().clone()
    }

    /// Latest rendered frame and its sequence number.
    pub fn frame(&self) -> (u64, Option<Arc<RenderImage>>) {
        (
            self.shared.frame_seq.load(Ordering::Acquire),
            self.shared.frame.lock().unwrap().clone(),
        )
    }

    pub fn frame_seq(&self) -> u64 {
        self.shared.frame_seq.load(Ordering::Acquire)
    }

    /// Tells the renderer how large the video area is, in device pixels.
    pub fn set_target_size(&self, width: u32, height: u32) {
        self.shared.target_w.store(width.max(16), Ordering::Relaxed);
        self.shared
            .target_h
            .store(height.max(16), Ordering::Relaxed);
    }

    pub fn acknowledge_end(&self) {
        let mut status = self.shared.status.lock().unwrap();
        if status.state == PlayState::Ended {
            *status = PlayerStatus {
                volume: status.volume,
                muted: status.muted,
                speed: status.speed,
                ..Default::default()
            };
        }
        *self.shared.frame.lock().unwrap() = None;
        self.shared.frame_seq.fetch_add(1, Ordering::Release);
    }

    pub fn toggle_pause(&self) {
        self.send(Cmd::TogglePause);
    }
    pub fn seek_relative(&self, secs: f64) {
        self.send(Cmd::SeekRelative(secs));
    }
    pub fn seek_absolute(&self, secs: f64) {
        self.send(Cmd::SeekAbsolute(secs));
    }
    pub fn set_audio(&self, id: Option<i64>) {
        self.send(Cmd::SetAudio(id));
    }
    pub fn set_subtitle(&self, id: Option<i64>) {
        self.send(Cmd::SetSubtitle(id));
    }
    pub fn set_volume(&self, volume: f64) {
        if volume.is_finite() {
            self.send(Cmd::SetVolume(volume.clamp(0., 100.)));
        }
    }
    pub fn toggle_mute(&self) {
        self.send(Cmd::ToggleMute);
    }
    pub fn set_speed(&self, speed: f64) {
        if speed.is_finite() {
            self.send(Cmd::SetSpeed(speed.clamp(0.5, 2.)));
        }
    }
    pub fn adjust_subtitle_delay(&self, seconds: f64) {
        if seconds.is_finite() {
            self.send(Cmd::AdjustSubtitleDelay(seconds));
        }
    }
    pub fn reset_subtitle_delay(&self) {
        self.send(Cmd::ResetSubtitleDelay);
    }
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }

    /// Starts playback, replacing any running item.
    pub fn play(&self, request: PlayRequest) {
        {
            let mut status = self.shared.status.lock().unwrap();
            *status = PlayerStatus {
                state: PlayState::Starting,
                title: request.title.clone(),
                position: request.start_secs as f64,
                volume: status.volume,
                muted: status.muted,
                speed: status.speed,
                ..Default::default()
            };
        }
        self.ensure_thread();
        self.send(Cmd::Load(request));
    }

    fn send(&self, cmd: Cmd) {
        if let Some(tx) = self.commands.lock().unwrap().as_ref() {
            let _ = tx.send(cmd);
        }
    }

    fn ensure_thread(&self) {
        let mut slot = self.commands.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        *slot = Some(tx);
        let shared = self.shared.clone();
        let commands = self.commands.clone();
        thread::Builder::new()
            .name("mpv".into())
            .spawn(move || {
                if let Err(err) = run(shared.clone(), rx) {
                    log::error!("mpv worker failed: {err:#}");
                    let mut status = shared.status.lock().unwrap();
                    status.error = Some(format!("{err:#}"));
                    status.state = PlayState::Ended;
                }
                *commands.lock().unwrap() = None;
            })
            .expect("spawn mpv thread");
    }
}

// ----- worker thread ---------------------------------------------------------

struct Session {
    client: Client,
    item_id: String,
    play_session_id: String,
    last_report: Instant,
    /// The tracks Jellyfin picked, applied once the file has loaded.
    defaults: Streams,
    external_subtitles: Vec<ExternalSubtitle>,
    /// Tracks in the last progress report, to report a new choice right away.
    reported: Streams,
}

impl Session {
    fn tracks(&self, mut tracks: Vec<Track>) -> Vec<Track> {
        tracks.retain(|track| track.kind != "video");
        for subtitle in &self.external_subtitles {
            if let Some(track) = tracks.iter_mut().find(|track| {
                track.external && track.external_filename.as_deref() == Some(&subtitle.url)
            }) {
                track.jellyfin_index = Some(subtitle.index);
                track.forced = subtitle.forced;
            } else {
                tracks.push(Track {
                    id: subtitle.track_id(),
                    kind: "sub".into(),
                    external: true,
                    title: subtitle.title.clone(),
                    lang: subtitle.language.clone(),
                    codec: subtitle.codec.clone(),
                    forced: subtitle.forced,
                    jellyfin_index: Some(subtitle.index),
                    ..Default::default()
                });
            }
        }
        tracks
    }
}

struct Renderer {
    ctx: *mut sys::mpv_render_context,
    buffer: Vec<u8>,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe { sys::mpv_render_context_free(self.ctx) };
    }
}

unsafe extern "C" fn on_render_update(ctx: *mut c_void) {
    let flag = unsafe { &*(ctx as *const AtomicBool) };
    flag.store(true, Ordering::Release);
}

fn run(shared: Arc<Shared>, rx: mpsc::Receiver<Cmd>) -> Result<()> {
    let mut mpv = Mpv::with_initializer(|init| {
        init.set_option("vo", "libmpv")?;
        init.set_option("hwdec", "auto-safe")?;
        init.set_option("keep-open", "no")?;
        init.set_option("idle", "yes")?;
        init.set_option("terminal", "no")?;
        // Only exists in Lua-enabled builds; the static release build has no ytdl hook.
        let _ = init.set_option("ytdl", "no");
        init.set_option("config", "no")?;
        init.set_option("sub-auto", "no")?;
        init.set_option("audio-display", "no")?;
        init.set_option(
            "user-agent",
            format!("{}/{}", crate::config::APP_NAME, crate::config::APP_VERSION),
        )?;
        Ok(())
    })
    .map_err(|e| anyhow!("could not create libmpv core: {e}"))?;

    // Wake the loop from mpv's threads on events and on new frames.
    let wake = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    {
        let wake = wake.clone();
        mpv.set_wakeup_callback(move || {
            *wake.0.lock().unwrap() = true;
            wake.1.notify_one();
        });
    }
    let needs_render = Box::leak(Box::new(AtomicBool::new(false)));

    let renderer = create_renderer(&mpv, needs_render)?;

    mpv.observe_property("time-pos", Format::Double, 1)?;
    mpv.observe_property("duration", Format::Double, 2)?;
    mpv.observe_property("pause", Format::Flag, 3)?;
    mpv.observe_property("track-list", Format::String, 4)?;
    mpv.observe_property("paused-for-cache", Format::Flag, 5)?;
    mpv.observe_property("volume", Format::Double, 6)?;
    mpv.observe_property("mute", Format::Flag, 7)?;
    mpv.observe_property("speed", Format::Double, 8)?;
    mpv.observe_property("sub-delay", Format::Double, 9)?;

    let mut renderer = renderer;
    let mut session: Option<Session> = None;
    // END_FILE events still due from files that a later load replaced.
    let mut replaced_ends = 0u32;
    let mut awake = KeepAwake::default();

    loop {
        // Events from mpv.
        while let Some(event) = mpv.wait_event(0.0) {
            match event {
                Ok(Event::EndFile(_)) if replaced_ends > 0 => replaced_ends -= 1,
                Ok(event) => {
                    if let (Event::FileLoaded, Some(active)) = (&event, &session) {
                        if let Some(subtitle) = active
                            .external_subtitles
                            .iter()
                            .find(|sub| Some(sub.index) == active.defaults.subtitle)
                            && !load_external_subtitle(&mpv, subtitle, &shared)
                        {
                            let _ = mpv.set_property("sid", "no");
                        }
                        select_tracks(&mpv, active);
                    }
                    if handle_event(&event, &shared, &mut session) {
                        finish(&shared, &mut session);
                    }
                }
                // The wrapper reports events that carry an mpv error (such as a
                // failed end-file) as `Err`; treat those as a failed load.
                Err(err) => {
                    log::warn!("mpv event error: {err}");
                    if session.is_some() {
                        let message = match err {
                            libmpv2::Error::Raw(code) => mpv_error_text(code),
                            other => other.to_string(),
                        };
                        shared.status.lock().unwrap().error =
                            Some(format!("Playback failed: {message}"));
                        finish(&shared, &mut session);
                    }
                }
            }
        }

        // Commands from the UI.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Cmd::Load(req) => {
                    // `loadfile replace` ends the old file. Report it at its own position;
                    // the shared status already belongs to the new request.
                    if let Some(old) = session.take() {
                        let position = mpv.get_property::<f64>("time-pos").unwrap_or(0.);
                        let paused = mpv.get_property::<bool>("pause").unwrap_or(false);
                        report_stopped(&old, position, paused);
                        replaced_ends += 1;
                    }
                    let header = req.client.mpv_auth_header();
                    mpv.set_property("http-header-fields", header)?;
                    mpv.set_property("force-media-title", req.title.clone())?;
                    // mpv keeps `pause` across files.
                    mpv.set_property("pause", false)?;
                    // Subtitle timing belongs to this video, unlike speed and volume.
                    mpv.set_property("sub-delay", 0.)?;
                    let start = if req.start_secs > 0 {
                        format!("start=+{}", req.start_secs)
                    } else {
                        String::new()
                    };
                    let mut args: Vec<&str> = vec![req.url.as_str(), "replace", "-1"];
                    if !start.is_empty() {
                        args.push(&start);
                    }
                    if let Err(err) = mpv.command("loadfile", &args) {
                        let mut status = shared.status.lock().unwrap();
                        status.error = Some(format!("loadfile failed: {err}"));
                        status.state = PlayState::Ended;
                        continue;
                    }
                    // Fetched while mpv opens the file.
                    let (defaults, external_subtitles) = req
                        .client
                        .playback_info(&req.item_id)
                        .inspect_err(|err| log::warn!("playback info lookup failed: {err:#}"))
                        .unwrap_or_default();
                    let progress = Progress {
                        item_id: req.item_id.clone(),
                        play_session_id: uuid::Uuid::new_v4().to_string(),
                        position_ticks: req.start_secs * TICKS_PER_SECOND,
                        paused: false,
                        streams: defaults,
                    };
                    if let Err(err) = req.client.report_start(&progress) {
                        log::warn!("report start failed: {err:#}");
                    }
                    session = Some(Session {
                        client: req.client,
                        item_id: req.item_id,
                        play_session_id: progress.play_session_id,
                        last_report: Instant::now(),
                        defaults,
                        external_subtitles,
                        reported: defaults,
                    });
                }
                Cmd::TogglePause => {
                    let _ = mpv.command("cycle", &["pause"]);
                }
                Cmd::SeekRelative(secs) => {
                    let _ = mpv.command("seek", &[&secs.to_string(), "relative"]);
                }
                Cmd::SeekAbsolute(secs) => {
                    let _ = mpv.command("seek", &[&secs.to_string(), "absolute"]);
                }
                Cmd::SetAudio(id) => {
                    let value = id.map(|i| i.to_string()).unwrap_or_else(|| "no".into());
                    let _ = mpv.set_property("aid", value);
                }
                Cmd::SetSubtitle(id) => {
                    shared.status.lock().unwrap().subtitle_error = None;
                    if let Some(subtitle) = session.as_ref().and_then(|active| {
                        active
                            .external_subtitles
                            .iter()
                            .find(|sub| Some(sub.track_id()) == id)
                    }) {
                        load_external_subtitle(&mpv, subtitle, &shared);
                    } else {
                        let value = id.map(|i| i.to_string()).unwrap_or_else(|| "no".into());
                        let _ = mpv.set_property("sid", value);
                    }
                }
                Cmd::SetVolume(volume) => {
                    let _ = mpv.set_property("volume", volume);
                    if volume > 0. {
                        let _ = mpv.set_property("mute", false);
                    }
                }
                Cmd::ToggleMute => {
                    let _ = mpv.command("cycle", &["mute"]);
                }
                Cmd::SetSpeed(speed) => {
                    let _ = mpv.set_property("speed", speed);
                }
                Cmd::AdjustSubtitleDelay(seconds) => {
                    let _ = mpv.command("add", &["sub-delay", &seconds.to_string()]);
                }
                Cmd::ResetSubtitleDelay => {
                    let _ = mpv.set_property("sub-delay", 0.);
                }
                Cmd::Stop => {
                    let _ = mpv.command("stop", &[]);
                }
            }
        }

        // Periodic progress reporting, and right away when the tracks change:
        // Jellyfin remembers the tracks from these reports.
        if let Some(active) = session.as_mut() {
            let status = shared.status.lock().unwrap();
            let mut streams = selected_streams(&status.tracks);
            if status.subtitle_error.is_some() && streams.subtitle == Some(-1) {
                // A failed download must not replace a remembered subtitle with Off.
                streams.subtitle = active.reported.subtitle;
            }
            let changed = !status.tracks.is_empty() && streams != active.reported;
            let due = active.last_report.elapsed() >= Duration::from_secs(10);
            if status.state == PlayState::Playing && (changed || due) {
                let mut progress = progress_of(active, status.position, status.paused);
                drop(status);
                progress.streams = streams;
                active.last_report = Instant::now();
                active.reported = streams;
                if let Err(err) = active.client.report_progress(&progress) {
                    log::warn!("report progress failed: {err:#}");
                }
            } else if due {
                active.last_report = Instant::now();
            }
        }

        {
            let status = shared.status.lock().unwrap();
            awake.set(session.is_some() && status.state == PlayState::Playing && !status.paused);
        }

        // Video frame.
        if needs_render.swap(false, Ordering::AcqRel) {
            let flags = unsafe { sys::mpv_render_context_update(renderer.ctx) };
            if flags & (sys::mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME as u64) != 0
                && let Err(err) = render_frame(&mpv, &mut renderer, &shared)
            {
                log::debug!("render skipped: {err}");
            }
        }

        // Sleep until something happens (bounded so timers keep running).
        let (lock, cvar) = &*wake;
        let mut flag = lock.lock().unwrap();
        if !*flag && !needs_render.load(Ordering::Acquire) {
            let (guard, _) = cvar.wait_timeout(flag, Duration::from_millis(4)).unwrap();
            flag = guard;
        }
        *flag = false;
    }
}

fn create_renderer(mpv: &Mpv, needs_render: &'static AtomicBool) -> Result<Renderer> {
    let api = CString::new("sw").unwrap();
    let mut params = [
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
            data: api.as_ptr() as *mut c_void,
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        },
    ];
    let mut ctx: *mut sys::mpv_render_context = ptr::null_mut();
    let code =
        unsafe { sys::mpv_render_context_create(&mut ctx, mpv.ctx.as_ptr(), params.as_mut_ptr()) };
    if code < 0 || ctx.is_null() {
        return Err(anyhow!("mpv_render_context_create failed ({code})"));
    }
    unsafe {
        sys::mpv_render_context_set_update_callback(
            ctx,
            Some(on_render_update),
            needs_render as *const AtomicBool as *mut c_void,
        );
    }
    Ok(Renderer {
        ctx,
        buffer: Vec::new(),
    })
}

/// Renders the current frame at the largest size that fits the UI's video area
/// without exceeding the source resolution, then publishes it as a gpui image.
fn render_frame(mpv: &Mpv, renderer: &mut Renderer, shared: &Shared) -> Result<()> {
    let src_w = mpv.get_property::<i64>("dwidth").unwrap_or(0).max(0) as u32;
    let src_h = mpv.get_property::<i64>("dheight").unwrap_or(0).max(0) as u32;
    if src_w == 0 || src_h == 0 {
        return Ok(());
    }
    let target_w = shared.target_w.load(Ordering::Relaxed).max(16);
    let target_h = shared.target_h.load(Ordering::Relaxed).max(16);
    let scale = (target_w as f64 / src_w as f64)
        .min(target_h as f64 / src_h as f64)
        .min(1.0);
    let w = (((src_w as f64 * scale) as u32).max(2) / 2) * 2;
    let h = (((src_h as f64 * scale) as u32).max(2) / 2) * 2;
    let stride = (w * 4) as usize;
    renderer.buffer.resize(stride * h as usize, 0);

    let mut size = [w as i32, h as i32];
    let format = CString::new("bgr0").unwrap();
    let mut stride_val = stride;
    let mut params = [
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_SIZE,
            data: size.as_mut_ptr() as *mut c_void,
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_FORMAT,
            data: format.as_ptr() as *mut c_void,
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_STRIDE,
            data: &mut stride_val as *mut usize as *mut c_void,
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_POINTER,
            data: renderer.buffer.as_mut_ptr() as *mut c_void,
        },
        sys::mpv_render_param {
            type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        },
    ];
    let code = unsafe { sys::mpv_render_context_render(renderer.ctx, params.as_mut_ptr()) };
    if code < 0 {
        return Err(anyhow!("mpv_render_context_render failed ({code})"));
    }
    // mpv leaves the padding byte undefined; gpui wants opaque BGRA.
    let mut pixels = renderer.buffer.clone();
    for px in pixels.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    let image = image::RgbaImage::from_raw(w, h, pixels).context("frame buffer size")?;
    let frame = image::Frame::new(image);
    let render_image = RenderImage::new(smallvec::SmallVec::from_vec(vec![frame]));
    *shared.frame.lock().unwrap() = Some(Arc::new(render_image));
    shared.frame_seq.fetch_add(1, Ordering::Release);
    Ok(())
}

/// Applies one mpv event to the shared status. Returns true when playback ended.
fn handle_event(event: &Event, shared: &Shared, session: &mut Option<Session>) -> bool {
    let mut status = shared.status.lock().unwrap();
    match event {
        Event::PropertyChange { name, change, .. } => {
            match (*name, change) {
                ("time-pos", PropertyData::Double(v)) => status.position = *v,
                ("duration", PropertyData::Double(v)) => status.duration = *v,
                ("pause", PropertyData::Flag(v)) => status.paused = *v,
                ("volume", PropertyData::Double(v)) => status.volume = *v,
                ("mute", PropertyData::Flag(v)) => status.muted = *v,
                ("speed", PropertyData::Double(v)) => status.speed = *v,
                ("sub-delay", PropertyData::Double(v)) => status.subtitle_delay = *v,
                ("paused-for-cache", PropertyData::Flag(v)) => status.buffering = *v,
                ("track-list", PropertyData::Str(json)) => {
                    let tracks: Vec<Track> = serde_json::from_str(json).unwrap_or_default();
                    let tracks = match session.as_ref() {
                        Some(active) => active.tracks(tracks),
                        None => tracks.into_iter().filter(|t| t.kind != "video").collect(),
                    };
                    if tracks != status.tracks {
                        status.tracks = tracks;
                        status.tracks_version += 1;
                    }
                }
                _ => {}
            }
            false
        }
        Event::FileLoaded | Event::PlaybackRestart => {
            if session.is_some() {
                status.state = PlayState::Playing;
            }
            false
        }
        Event::EndFile(reason) => {
            if *reason == mpv_end_file_reason::Error {
                status.error = Some("mpv could not play this stream".to_string());
            }
            status.finished = *reason == mpv_end_file_reason::Eof;
            // Only the active session's end is meaningful (stop+load emits one too).
            session.is_some()
        }
        Event::Shutdown => true,
        _ => false,
    }
}

fn mpv_error_text(code: i32) -> String {
    unsafe {
        let ptr = sys::mpv_error_string(code);
        if ptr.is_null() {
            format!("mpv error {code}")
        } else {
            std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

fn progress_of(session: &Session, position: f64, paused: bool) -> Progress {
    Progress {
        item_id: session.item_id.clone(),
        play_session_id: session.play_session_id.clone(),
        position_ticks: (position * TICKS_PER_SECOND as f64) as i64,
        paused,
        streams: Streams::default(),
    }
}

/// The selected tracks as Jellyfin streams.
fn selected_streams(tracks: &[Track]) -> Streams {
    let selected = |kind: &str| tracks.iter().find(|t| t.kind == kind && t.selected);
    Streams {
        audio: selected("audio").and_then(|t| t.stream_index()),
        subtitle: match selected("sub") {
            Some(track) => track.stream_index(),
            None => Some(-1),
        },
    }
}

/// mpv `aid` and `sid` values that select `streams`, for those this file has.
fn track_choice(tracks: &[Track], streams: Streams) -> (Option<String>, Option<String>) {
    let find = |kind: &str, index: i64| {
        tracks
            .iter()
            .find(|t| t.id > 0 && t.kind == kind && t.stream_index() == Some(index))
            .map(|t| t.id.to_string())
    };
    let audio = streams.audio.and_then(|i| find("audio", i));
    let subtitle = match streams.subtitle {
        Some(-1) => Some("no".into()),
        Some(i) => find("sub", i),
        None => None,
    };
    (audio, subtitle)
}

fn load_external_subtitle(mpv: &Mpv, subtitle: &ExternalSubtitle, shared: &Shared) -> bool {
    if let Err(err) = mpv.command(
        "sub-add",
        &[
            &subtitle.url,
            "cached",
            subtitle.title.as_deref().unwrap_or(""),
            subtitle.language.as_deref().unwrap_or(""),
        ],
    ) {
        shared.status.lock().unwrap().subtitle_error =
            Some(format!("Could not load subtitle: {err}"));
        return false;
    }
    true
}

fn select_tracks(mpv: &Mpv, session: &Session) {
    let tracks: Vec<Track> = mpv
        .get_property::<String>("track-list")
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let (audio, subtitle) = track_choice(&session.tracks(tracks), session.defaults);
    if let Some(id) = audio {
        let _ = mpv.set_property("aid", id);
    }
    if let Some(id) = subtitle {
        let _ = mpv.set_property("sid", id);
    }
}

fn report_stopped(session: &Session, position: f64, paused: bool) {
    if let Err(err) = session
        .client
        .report_stopped(&progress_of(session, position, paused))
    {
        log::warn!("report stopped failed: {err:#}");
    }
}

/// Reports the stop to Jellyfin and marks the session ended.
fn finish(shared: &Shared, session: &mut Option<Session>) {
    let Some(active) = session.take() else { return };
    let snapshot = shared.status.lock().unwrap().clone();
    report_stopped(&active, snapshot.position, snapshot.paused);
    shared.status.lock().unwrap().state = PlayState::Ended;
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
    };

    struct SubtitleServer {
        base: String,
        requests: Arc<Mutex<Vec<(String, bool, String)>>>,
        default_subtitle: Arc<AtomicU32>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }

    impl SubtitleServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let default_subtitle = Arc::new(AtomicU32::new(9));
            let default_index = default_subtitle.clone();
            let (done, records) = (stop.clone(), requests.clone());
            let worker = thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    let (mut socket, _) = match listener.accept() {
                        Ok(connection) => connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(error) => panic!("mock server accept: {error}"),
                    };
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut reader = BufReader::new(&mut socket);
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let path = line.split_whitespace().nth(1).unwrap().to_string();
                    let mut length = 0;
                    let mut authenticated = false;
                    loop {
                        line.clear();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        let header = line.to_ascii_lowercase();
                        if let Some(value) = header.strip_prefix("content-length:") {
                            length = value.trim().parse().unwrap();
                        }
                        if header.starts_with("authorization:")
                            && header.contains("token=\"token\"")
                        {
                            authenticated = true;
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    records.lock().unwrap().push((
                        path.clone(),
                        authenticated,
                        String::from_utf8(body).unwrap(),
                    ));
                    let (status, content_type, response) = if path
                        .starts_with("/Items/test/PlaybackInfo")
                    {
                        (
                            "200 OK",
                            "application/json",
                            r#"{"MediaSources":[{"Id":"test","DefaultAudioStreamIndex":1,"DefaultSubtitleStreamIndex":9,"MediaStreams":[
                            {"Index":9,"Type":"Subtitle","IsExternal":true,"IsTextSubtitleStream":true,"Codec":"subrip","DisplayTitle":"English","Language":"eng"},
                            {"Index":10,"Type":"Subtitle","IsExternal":true,"IsTextSubtitleStream":true,"Codec":"ass","DisplayTitle":"French","Language":"fra"},
                            {"Index":11,"Type":"Subtitle","IsExternal":true,"IsTextSubtitleStream":true,"Codec":"subrip","DisplayTitle":"Missing"},
                            {"Index":12,"Type":"Subtitle","IsExternal":true,"IsTextSubtitleStream":true,"Codec":"webvtt","DisplayTitle":"Spanish","Language":"spa"}
                        ]}]}"#,
                        )
                    } else if path.contains("/Subtitles/11/") {
                        ("404 Not Found", "text/plain", "missing")
                    } else if path.ends_with("Stream.ass") {
                        (
                            "200 OK",
                            "text/plain",
                            "[Script Info]\nScriptType: v4.00+\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:29.00,Default,,0,0,0,,French subtitle\n",
                        )
                    } else if path.ends_with("Stream.vtt") {
                        (
                            "200 OK",
                            "text/vtt",
                            "WEBVTT\n\n00:00:00.000 --> 00:00:29.000\nSpanish subtitle\n",
                        )
                    } else if path.contains("/Subtitles/") {
                        (
                            "200 OK",
                            "text/plain",
                            "1\n00:00:00,000 --> 00:00:29,000\nSubtitle test\n",
                        )
                    } else {
                        ("200 OK", "application/json", "{}")
                    };
                    let response = response.replace(
                        "\"DefaultSubtitleStreamIndex\":9",
                        &format!(
                            "\"DefaultSubtitleStreamIndex\":{}",
                            default_index.load(Ordering::Relaxed)
                        ),
                    );
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    );
                    // A failed subtitle load may close the socket before reading the body.
                    let _ = socket.write_all(response.as_bytes());
                }
            });
            Self {
                base,
                requests,
                default_subtitle,
                stop,
                worker: Some(worker),
            }
        }
    }

    impl Drop for SubtitleServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.worker.take().unwrap().join().unwrap();
        }
    }

    #[test]
    fn external_tracks_keep_jellyfin_indexes_and_do_not_duplicate_placeholders() {
        let subtitle = ExternalSubtitle {
            index: 9,
            url: "https://media.example.com/sub.srt".into(),
            title: Some("English".into()),
            language: Some("eng".into()),
            codec: Some("subrip".into()),
            forced: true,
        };
        let session = Session {
            client: Client::new("https://media.example.com", "test"),
            item_id: "test".into(),
            play_session_id: "test".into(),
            last_report: Instant::now(),
            defaults: Streams::default(),
            reported: Streams::default(),
            external_subtitles: vec![subtitle],
        };
        let unloaded = session.tracks(Vec::new());
        assert_eq!(unloaded[0].id, -10);
        assert_eq!(
            track_choice(
                &unloaded,
                Streams {
                    audio: None,
                    subtitle: Some(9)
                }
            )
            .1,
            None
        );
        let tracks: Vec<Track> = serde_json::from_str(r#"[
            {"id":1,"type":"sub","ff-index":0},
            {"id":2,"type":"sub","ff-index":0,"external":true,"external-filename":"https://media.example.com/sub.srt","selected":true}
        ]"#).unwrap();
        let tracks = session.tracks(tracks);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[1].stream_index(), Some(9));
        assert!(tracks[1].forced);
        assert_eq!(selected_streams(&tracks).subtitle, Some(9));
        assert_eq!(
            track_choice(
                &tracks,
                Streams {
                    audio: None,
                    subtitle: Some(9)
                }
            )
            .1
            .as_deref(),
            Some("2")
        );
        assert_eq!(
            track_choice(
                &tracks,
                Streams {
                    audio: None,
                    subtitle: Some(0)
                }
            )
            .1
            .as_deref(),
            Some("1")
        );
    }

    #[test]
    fn parses_track_list_json() {
        let json = r#"[{"id":1,"type":"video","selected":true},
            {"id":1,"type":"audio","lang":"eng","codec":"aac","selected":true,"default":true},
            {"id":2,"type":"sub","title":"English SDH","lang":"eng","codec":"subrip","selected":false}]"#;
        let tracks: Vec<Track> = serde_json::from_str(json).unwrap();
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[1].label(), "ENG · AAC");
        assert_eq!(tracks[2].label(), "English SDH · ENG · SUBRIP");
    }

    #[test]
    fn maps_tracks_to_jellyfin_streams() {
        let json = r#"[{"id":1,"type":"audio","ff-index":1,"selected":true},
            {"id":2,"type":"audio","ff-index":2},
            {"id":1,"type":"sub","ff-index":3}]"#;
        let mut tracks: Vec<Track> = serde_json::from_str(json).unwrap();
        let streams = |audio, subtitle| Streams { audio, subtitle };
        assert_eq!(selected_streams(&tracks), streams(Some(1), Some(-1)));
        tracks[2].selected = true;
        assert_eq!(selected_streams(&tracks), streams(Some(1), Some(3)));

        let choice = |s| track_choice(&tracks, s);
        assert_eq!(
            choice(streams(Some(2), Some(3))),
            (Some("2".into()), Some("1".into()))
        );
        assert_eq!(choice(streams(None, Some(-1))), (None, Some("no".into())));
        // A stream this file doesn't have, such as an external subtitle, keeps mpv's pick.
        assert_eq!(choice(streams(Some(7), Some(8))), (None, None));
    }

    /// Plays a synthetic clip through the embedded core, checks frames arrive,
    /// pauses, seeks, and stops. Skipped when libmpv or ffmpeg are unavailable.
    #[test]
    fn embedded_playback_roundtrip() {
        let media = std::env::temp_dir().join("jellyui-embedded-test.mp4");
        let embedded_subtitle = std::env::temp_dir().join("jellyui-embedded-test.srt");
        std::fs::write(
            &embedded_subtitle,
            "1\n00:00:00,000 --> 00:00:30,000\nEmbedded subtitle\n",
        )
        .unwrap();
        let encoded = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=30:size=320x240:rate=10",
            ])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=30"])
            .arg("-i")
            .arg(&embedded_subtitle)
            .args(["-c:s", "mov_text"])
            .args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&media)
            .status();
        if !encoded.map(|s| s.success()).unwrap_or(false) {
            eprintln!("ffmpeg not available; skipping");
            return;
        }
        let server = SubtitleServer::start();
        let client = Client::new(&server.base, "test-device").with_session("token", "user");
        let player = Player::default();
        player.set_target_size(640, 480);
        let request = |start_secs| PlayRequest {
            client: client.clone(),
            item_id: "test".into(),
            url: media.display().to_string(),
            title: "Embedded test".into(),
            start_secs,
        };
        player.play(request(0));
        let wait = |what: &str, mut ok: Box<dyn FnMut() -> bool>| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !ok() {
                let status = player.status();
                assert!(status.error.is_none(), "{what}: {status:?}");
                assert!(Instant::now() < deadline, "{what} timed out: {status:?}");
                thread::sleep(Duration::from_millis(50));
            }
        };
        let p = player.clone();
        wait(
            "playing",
            Box::new(move || p.status().state == PlayState::Playing),
        );
        let p = player.clone();
        wait("first frame", Box::new(move || p.frame().1.is_some()));
        let (_, frame) = player.frame();
        let frame = frame.unwrap();
        assert_eq!(frame.size(0).width.0, 320);
        assert_eq!(frame.size(0).height.0, 240);
        let p = player.clone();
        wait(
            "tracks",
            Box::new(move || p.status().tracks.iter().any(|t| t.kind == "audio")),
        );
        // Remembered tracks are matched by FFmpeg index; mpv must report it.
        assert!(
            player
                .status()
                .tracks
                .iter()
                .filter(|t| t.id > 0)
                .all(|t| t.ff_index.is_some())
        );
        let p = player.clone();
        wait(
            "remembered external subtitle",
            Box::new(move || selected_streams(&p.status().tracks).subtitle == Some(9)),
        );
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|(path, _, _)| !path.contains("/Subtitles/10/"))
        );
        player.set_subtitle(Some(-11));
        let p = player.clone();
        wait(
            "external subtitle selected",
            Box::new(move || selected_streams(&p.status().tracks).subtitle == Some(10)),
        );
        let records = server.requests.clone();
        wait(
            "external index reported",
            Box::new(move || {
                records.lock().unwrap().iter().any(|(path, _, body)| {
            path == "/Sessions/Playing/Progress" && serde_json::from_str::<serde_json::Value>(body).unwrap()["SubtitleStreamIndex"] == 10
        })
            }),
        );
        player.set_subtitle(Some(-11));
        let p = player.clone();
        wait(
            "cached external subtitle",
            Box::new(move || selected_streams(&p.status().tracks).subtitle == Some(10)),
        );
        player.set_subtitle(Some(-12));
        let p = player.clone();
        wait(
            "subtitle failure is nonfatal",
            Box::new(move || p.status().subtitle_error.is_some()),
        );
        assert_eq!(player.status().state, PlayState::Playing);
        assert_eq!(selected_streams(&player.status().tracks).subtitle, Some(10));
        assert_eq!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(path, _, _)| path.contains("/Subtitles/10/"))
                .count(),
            1
        );
        player.set_subtitle(Some(-13));
        let p = player.clone();
        wait(
            "WebVTT subtitle selected",
            Box::new(move || selected_streams(&p.status().tracks).subtitle == Some(12)),
        );
        assert!(player.status().subtitle_error.is_none());
        let embedded = player
            .status()
            .tracks
            .iter()
            .find(|t| t.kind == "sub" && !t.external)
            .unwrap()
            .id;
        player.set_subtitle(Some(embedded));
        let p = player.clone();
        wait(
            "embedded subtitle selected",
            Box::new(move || selected_streams(&p.status().tracks).subtitle == Some(2)),
        );
        player.set_subtitle(None);
        let p = player.clone();
        wait(
            "subtitles off",
            Box::new(move || selected_streams(&p.status().tracks).subtitle == Some(-1)),
        );
        player.set_volume(35.);
        let p = player.clone();
        wait(
            "volume",
            Box::new(move || (p.status().volume - 35.).abs() < 0.1),
        );
        player.toggle_mute();
        let p = player.clone();
        wait("muted", Box::new(move || p.status().muted));
        player.set_volume(140.);
        let p = player.clone();
        wait(
            "volume capped and unmuted",
            Box::new(move || {
                let s = p.status();
                (s.volume - 100.).abs() < 0.1 && !s.muted
            }),
        );
        player.set_volume(35.);
        let p = player.clone();
        wait(
            "volume restored",
            Box::new(move || (p.status().volume - 35.).abs() < 0.1),
        );
        player.set_speed(1.5);
        let p = player.clone();
        wait(
            "speed",
            Box::new(move || (p.status().speed - 1.5).abs() < 0.01),
        );
        player.adjust_subtitle_delay(0.5);
        player.adjust_subtitle_delay(0.5);
        let p = player.clone();
        wait(
            "subtitle delay",
            Box::new(move || (p.status().subtitle_delay - 1.).abs() < 0.01),
        );
        player.reset_subtitle_delay();
        let p = player.clone();
        wait(
            "subtitle delay reset",
            Box::new(move || p.status().subtitle_delay.abs() < 0.01),
        );
        player.adjust_subtitle_delay(-0.5);
        let p = player.clone();
        wait(
            "subtitles earlier",
            Box::new(move || (p.status().subtitle_delay + 0.5).abs() < 0.01),
        );
        // Replacing a playing file must not end the new session.
        player.play(request(2));
        let p = player.clone();
        wait(
            "replaced",
            Box::new(move || p.status().state == PlayState::Playing),
        );
        thread::sleep(Duration::from_millis(500));
        assert!((player.status().volume - 35.).abs() < 0.1);
        assert!((player.status().speed - 1.5).abs() < 0.01);
        assert!(player.status().subtitle_delay.abs() < 0.01);
        player.set_speed(1.);
        let p = player.clone();
        wait(
            "normal speed",
            Box::new(move || (p.status().speed - 1.).abs() < 0.01),
        );
        assert_eq!(player.status().state, PlayState::Playing);
        player.toggle_pause();
        let p = player.clone();
        wait("paused", Box::new(move || p.status().paused));
        player.seek_absolute(20.0);
        let p = player.clone();
        wait("seeked", Box::new(move || p.status().position >= 19.0));
        player.stop();
        let p = player.clone();
        wait(
            "ended",
            Box::new(move || p.status().state == PlayState::Ended),
        );
        assert!(!player.status().finished);
        player.acknowledge_end();
        assert_eq!(player.status().state, PlayState::Idle);

        server.default_subtitle.store(11, Ordering::Relaxed);
        player.play(request(28));
        let p = player.clone();
        wait(
            "missing remembered subtitle",
            Box::new(move || p.status().subtitle_error.is_some()),
        );
        player.set_audio(None);
        let records = server.requests.clone();
        wait(
            "failed default remains remembered",
            Box::new(move || {
                records.lock().unwrap().iter().any(|(path, _, body)| {
            path == "/Sessions/Playing/Progress" && serde_json::from_str::<serde_json::Value>(body).unwrap()["SubtitleStreamIndex"] == 11
        })
            }),
        );
        let p = player.clone();
        wait(
            "played to the end",
            Box::new(move || p.status().state == PlayState::Ended),
        );
        assert!(player.status().finished);
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|(_, authenticated, _)| *authenticated)
        );
    }
}
