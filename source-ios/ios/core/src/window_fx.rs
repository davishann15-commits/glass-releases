//! What the shared modules ask of the desktop's `window_fx` (window effects),
//! answered for iOS: there is no window to style, and accessibility settings
//! come from UIKit through the app (`GlassCore::set_accessibility`).

use std::sync::atomic::{AtomicBool, Ordering};

static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

/// Never headless on iOS: the app's own views are always there. (The desktop
/// uses this to skip device polling and network side effects in UI tests.)
pub fn headless() -> bool {
    false
}

/// UIAccessibility's Reduce Motion, as the app last reported it.
pub fn system_reduce_motion() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed)
}

pub fn set_system_reduce_motion(on: bool) {
    REDUCE_MOTION.store(on, Ordering::Relaxed);
}
