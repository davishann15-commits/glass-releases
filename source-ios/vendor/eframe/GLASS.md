# eframe 0.36.2, patched for Glass

This is eframe 0.36.2 from crates.io (MIT OR Apache-2.0, © the egui authors,
https://github.com/emilk/egui) with one change to how its OpenGL windows
present frames on Wayland. `Cargo.toml` in the app uses it via
`[patch.crates-io]`; everything else is unchanged.

Why: eframe asks the GL driver to wait for the display before each frame
(swap interval 1). On Wayland the driver waits for the compositor's frame
callback, and a window on a workspace you aren't looking at gets none: as soon
as Glass drew while hidden (a song change, the progress bar), it blocked the
main thread until the window was shown again. Nothing else ran meanwhile,
including the reply to the compositor's "are you still there" ping, so
Hyprland reported Glass as not responding every time you switched away from it
for a few seconds. Seen with NVIDIA's EGL (libnvidia-egl-wayland2), where the
wait has no timeout.

winit already knows how to wait properly: when an app calls
`Window::pre_present_notify` before presenting, winit asks for a frame
callback and holds the next `RedrawRequested` until it arrives, without
blocking its event loop. eframe doesn't call it.

Changes (marked `Glass` in the source):

- `src/native/glow_integration.rs` — `pre_present_notify` before every
  `swap_buffers` (the main window and immediate viewports such as the mini
  player), and on Wayland the swap interval is 0, so the driver never waits;
  winit's frame callbacks pace drawing at the display's rate instead, and a
  hidden window doesn't draw. X11, macOS and Windows keep the swap interval
  eframe chose.
