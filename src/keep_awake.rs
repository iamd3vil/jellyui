// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keeps the screen from blanking/locking while video plays. With `vo=libmpv`
//! mpv owns no window, so its own `stop-screensaver` does nothing.

#[derive(Default)]
pub struct KeepAwake {
    wanted: bool,
    /// Dropping it releases the inhibit.
    held: Option<platform::Held>,
}

impl KeepAwake {
    pub fn set(&mut self, on: bool) {
        // Act on transitions only, so a missing service isn't retried every tick.
        if on == self.wanted {
            return;
        }
        self.wanted = on;
        self.held = None;
        if on {
            match platform::acquire() {
                Ok(held) => self.held = Some(held),
                Err(err) => log::warn!("could not inhibit screensaver: {err:#}"),
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use zbus::blocking::Connection;

    const DEST: &str = "org.freedesktop.ScreenSaver";
    const PATH: &str = "/org/freedesktop/ScreenSaver";

    /// The inhibit lasts as long as this connection, so it is kept around.
    pub struct Held(Connection, u32);

    pub fn acquire() -> anyhow::Result<Held> {
        let conn = Connection::session()?;
        let reply = conn.call_method(
            Some(DEST),
            PATH,
            Some(DEST),
            "Inhibit",
            &(crate::config::APP_NAME, "Playing video"),
        )?;
        let cookie: u32 = reply.body().deserialize()?;
        Ok(Held(conn, cookie))
    }

    impl Drop for Held {
        fn drop(&mut self) {
            let _ = self
                .0
                .call_method(Some(DEST), PATH, Some(DEST), "UnInhibit", &self.1);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_void};

    use anyhow::anyhow;

    type CFStringRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    const UTF8: u32 = 0x0800_0100;
    const LEVEL_ON: u32 = 255;

    pub struct Held(u32);

    pub fn acquire() -> anyhow::Result<Held> {
        unsafe {
            let kind = CFStringCreateWithCString(
                std::ptr::null(),
                c"PreventUserIdleDisplaySleep".as_ptr(),
                UTF8,
            );
            let name = CFStringCreateWithCString(std::ptr::null(), c"Playing video".as_ptr(), UTF8);
            let mut id = 0;
            let code = IOPMAssertionCreateWithName(kind, LEVEL_ON, name, &mut id);
            CFRelease(kind);
            CFRelease(name);
            if code != 0 {
                return Err(anyhow!("IOPMAssertionCreateWithName failed ({code:#x})"));
            }
            Ok(Held(id))
        }
    }

    impl Drop for Held {
        fn drop(&mut self) {
            unsafe { IOPMAssertionRelease(self.0) };
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    pub struct Held;

    pub fn acquire() -> anyhow::Result<Held> {
        Err(anyhow::anyhow!("not supported on this platform"))
    }
}
