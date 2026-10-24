//! Handing freed memory back to the system once the caches are trimmed (every
//! 30 seconds, see `App::tick`).
//!
//! macOS: mimalloc keeps the pages a thread allocated until that thread
//! allocates again or collects, even after other threads free everything in
//! them. The runtime's two workers parse the big pages (Liked Songs is ~15 MB
//! of JSON) and then sit idle, so what the trim let go stayed theirs. Loading
//! 1,692 liked songs and trimming them left the process 7.9 MB larger than
//! before; 1.8 MB once the workers collect. Each worker collects the first
//! time it goes idle after a trim.
//!
//! Linux: glibc keeps freed memory in its own lists; `malloc_trim` returns it.

use std::sync::atomic::{AtomicU32, Ordering};

/// Bumped after every trim; a worker that hasn't collected since does.
static TRIMS: AtomicU32 = AtomicU32::new(0);

/// Called on the runtime once the caches have been trimmed.
pub fn trimmed() {
    TRIMS.fetch_add(1, Ordering::Relaxed);
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    tokio::task::spawn_blocking(|| {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> std::ffi::c_int;
        }
        // SAFETY: glibc's; it only returns free memory to the system.
        unsafe { malloc_trim(0) };
    });
}

/// The runtime's `on_thread_park`: a worker going idle after a trim collects.
pub fn on_idle() {
    #[cfg(target_os = "macos")]
    {
        thread_local! {
            static SEEN: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        }
        let latest = TRIMS.load(Ordering::Relaxed);
        if SEEN.replace(latest) != latest {
            unsafe extern "C" {
                fn mi_collect(force: bool);
            }
            // SAFETY: mimalloc's (the global allocator on macOS); it frees only
            // this thread's pages that hold nothing.
            unsafe { mi_collect(true) };
        }
    }
}
