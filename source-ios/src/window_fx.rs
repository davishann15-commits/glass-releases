//! Native window glass: transparency, background blur, the system's
//! reduce-motion and reduce-transparency settings, and (macOS) the window
//! reopening where it was closed.
//!
//! macOS: the window is transparent and WindowServer blurs whatever is behind
//! it at `theme.blur` points, through `CGSSetWindowBackgroundBlurRadius` — the
//! private but long-stable call Ghostty, kitty, Alacritty and WezTerm use for
//! the same setting. AppKit's public `NSVisualEffectView` only offers fixed
//! materials (no radius) and adds its own tint, so it's the fallback when the
//! call isn't available. The symbol is looked up at runtime, so Glass still
//! starts if a future macOS removes it.
//!
//! Linux: whether anything behind the window is blurred is up to the
//! compositor (KWin's blur effect, Hyprland or picom rules); an app can only
//! ask for transparency. Plain transparency over a busy desktop is hard to
//! read, so unless `GLASS_LINUX_BLUR=1` says the compositor blurs Glass, the
//! background opacity is raised to at least `LINUX_MIN_OPACITY`.

use crate::theme_config::Theme;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, Ordering};

/// Set by tests: no AppKit calls, no system settings, no media keys.
static HEADLESS: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub fn set_headless() {
    HEADLESS.store(true, Ordering::Relaxed);
}

pub fn headless() -> bool {
    HEADLESS.load(Ordering::Relaxed)
}

/// The least background opacity on Linux when nothing blurs behind the window.
const LINUX_MIN_OPACITY: f32 = 0.88;

/// The window's WindowServer number (macOS), once known.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
static WINDOW: AtomicI64 = AtomicI64::new(-1);
/// 0 unknown, 1 native radius blur, 2 vibrancy fallback, 3 transparency only.
static MODE: AtomicU8 = AtomicU8::new(0);

/// What the background effect actually is on this machine, for Settings.
pub fn blur_status() -> &'static str {
    match MODE.load(Ordering::Relaxed) {
        1 => "Native background blur (macOS WindowServer)",
        2 => "macOS vibrancy (fixed blur; the blur radius has no effect)",
        3 if cfg!(target_os = "linux") && linux_compositor_blur() => "Transparency; blur is left to your compositor",
        3 => "Transparency only (no blur available)",
        _ => "Not started",
    }
}

/// `GLASS_LINUX_BLUR=1`: the compositor blurs Glass's window (see the module docs).
fn linux_compositor_blur() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("GLASS_LINUX_BLUR").is_ok_and(|v| v == "1"));
    *ON
}

/// The background opacity to paint: the theme's, raised where no blur makes
/// raw transparency unreadable, and solid when the system asks for reduced
/// transparency.
pub fn effective_opacity(theme: &Theme) -> f32 {
    if system_reduce_transparency() {
        return 1.0;
    }
    if cfg!(target_os = "linux") && !linux_compositor_blur() {
        return theme.opacity.max(LINUX_MIN_OPACITY);
    }
    theme.opacity
}

/// Hooks up the window. Call once from eframe's creation callback.
pub fn install(cc: &eframe::CreationContext<'_>, theme: &Theme) {
    #[cfg(target_os = "macos")]
    mac::install(cc);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cc;
        MODE.store(3, Ordering::Relaxed);
    }
    apply(theme);
}

/// Applies the theme's blur. Cheap; call whenever the theme changes.
pub fn apply(theme: &Theme) {
    #[cfg(target_os = "macos")]
    mac::set_blur(theme.blur);
    #[cfg(not(target_os = "macos"))]
    let _ = theme;
}

/// The operating system's "reduce transparency" accessibility setting
/// (checked at most once a second).
pub fn system_reduce_transparency() -> bool {
    use std::sync::Mutex;
    if headless() {
        return false;
    }
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    if let Some((at, v)) = *c
        && at.elapsed() < Duration::from_secs(1)
    {
        return v;
    }
    #[cfg(target_os = "macos")]
    let v = mac::reduce_transparency();
    #[cfg(not(target_os = "macos"))]
    let v = false;
    *c = Some((Instant::now(), v));
    v
}

/// The operating system's "reduce motion" accessibility setting.
pub fn system_reduce_motion() -> bool {
    if headless() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        mac::reduce_motion()
    }
    #[cfg(target_os = "linux")]
    {
        // GNOME and most GTK desktops: animations off means reduce motion.
        std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "enable-animations"]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{MODE, WINDOW};
    use std::ffi::{CStr, c_void};
    use std::sync::atomic::Ordering;

    type MainConnection = unsafe extern "C" fn() -> i32;
    type SetBlur = unsafe extern "C" fn(i32, u32, i32) -> i32;

    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
    }
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

    fn symbol(name: &CStr) -> Option<*mut c_void> {
        // SAFETY: dlsym with RTLD_DEFAULT and a NUL-terminated name only looks the name up.
        let p = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
        (!p.is_null()).then_some(p)
    }

    fn blur_api() -> Option<(MainConnection, SetBlur)> {
        let conn = symbol(c"CGSMainConnectionID")?;
        let set = symbol(c"CGSSetWindowBackgroundBlurRadius")?;
        // SAFETY: both symbols have had these C signatures since macOS 10.x.
        Some(unsafe { (std::mem::transmute::<*mut c_void, MainConnection>(conn), std::mem::transmute::<*mut c_void, SetBlur>(set)) })
    }

    pub fn install(cc: &eframe::CreationContext<'_>) {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSView;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let Some(_mtm) = MainThreadMarker::new() else { return };
        let Ok(handle) = cc.window_handle() else { return };
        let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
        // SAFETY: winit's content view, alive for as long as the window.
        let content: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
        let Some(window) = content.window() else { return };
        WINDOW.store(window.windowNumber() as i64, Ordering::Relaxed);
        // Reopen where the window was last closed, at the same size. AppKit keeps the frame in
        // the user defaults and moves it back onto a screen if that screen is gone.
        if std::env::var_os("GLASS_WINDOW").is_none() && std::env::var_os("GLASS_DEMO").is_none() {
            let name = objc2_foundation::ns_string!("Glass");
            window.setFrameUsingName(name);
            window.setFrameAutosaveName(name);
        }
        if blur_api().is_some() {
            MODE.store(1, Ordering::Relaxed);
        } else {
            vibrancy(content);
            MODE.store(2, Ordering::Relaxed);
        }
    }

    /// The public fallback: a vibrancy view *below* winit's content view (a
    /// subview would cover everything egui draws).
    fn vibrancy(content: &objc2_app_kit::NSView) {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSAutoresizingMaskOptions, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode};
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(frame) = (unsafe { content.superview() }) else { return };
        let blur = NSVisualEffectView::initWithFrame(mtm.alloc(), frame.bounds());
        blur.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
        blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        blur.setState(NSVisualEffectState::Active);
        blur.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        frame.addSubview_positioned_relativeTo(&blur, NSWindowOrderingMode::Below, Some(content));
    }

    pub fn set_blur(radius: u32) {
        let wid = WINDOW.load(Ordering::Relaxed);
        if wid < 0 || MODE.load(Ordering::Relaxed) != 1 {
            return;
        }
        if let Some((conn, set)) = blur_api() {
            // SAFETY: a valid connection and this process's own window number.
            let err = unsafe { set(conn(), wid as u32, radius.min(100) as i32) };
            if err != 0 {
                log::warn!("background blur unavailable (CGError {err})");
            }
        }
    }

    pub fn reduce_motion() -> bool {
        objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }

    pub fn reduce_transparency() -> bool {
        objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
    }
}
