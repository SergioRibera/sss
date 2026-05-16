# sss_capture

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../../README.md)

> Cross-platform screen, monitor, window and region capture for
> [Super ScreenShot][repo]. **No third-party capture library is used** —
> every backend talks directly to the OS through the canonical low-level
> Rust bindings.

```rust
use sss_capture::{Capturer, Rect};

let cap = Capturer::new()?;                       // best backend, auto-picked
cap.capture_all()?.save("/tmp/desktop.png")?;     // full virtual desktop
cap.capture_at_cursor()?.save("/tmp/here.png")?;  // monitor under the cursor
cap.capture_region(Rect::from_xywh(0, 0, 1920, 1080))?
   .save("/tmp/region.png")?;
```

---

## Backends — what's under the hood

`sss_capture` is **not** a wrapper around `xcap`, `libwayshot`, `screenshots`
or `scap`. Every backend is implemented from scratch on top of the canonical
low-level bindings for its platform:

| Platform           | Protocols & APIs                                                            | Crate(s) used                                                  |
| ------------------ | --------------------------------------------------------------------------- | -------------------------------------------------------------- |
| Linux Wayland      | `wl_compositor`, `wl_shm`, `wl_output`, `zxdg_output_v1`, `zwlr_screencopy_v1`, `zwlr_foreign_toplevel_v1` / `ext_foreign_toplevel_list_v1` | `wayland-client`, `wayland-protocols(-wlr)`, `memmap2`, `rustix` |
| Linux Wayland fallback | `org.freedesktop.portal.Screenshot` (DBus)                              | `dbus`, `percent-encoding`, `image`                            |
| Linux X11          | XGetImage, RANDR 1.5 `GetMonitors` (with 1.2 `GetScreenResources` fallback), EWMH `_NET_CLIENT_LIST`, `QueryPointer` | `x11rb` (pure-Rust XCB, no `libxcb.so` runtime)              |
| Windows            | `EnumDisplayMonitors`, `GetMonitorInfoW`, `GetDpiForMonitor`, `BitBlt(SRCCOPY \| CAPTUREBLT)`, `GetDIBits`, `EnumWindows`, `GetCursorPos` | `windows` (Win32 metadata bindings)                            |
| macOS              | `CGGetActiveDisplayList`, `CGDisplayCreateImage`, `CGDisplayCreateImageForRect`, `CGWindowListCopyWindowInfo`, `CGWindowListCreateImage`, `NSEvent.mouseLocation` | `core-graphics`, `core-foundation`, `objc2`, `objc2-foundation`, `objc2-app-kit` |

These are *bindings*, not capture libraries — they expose the OS's own
protocol primitives. The capture logic (event-driven Wayland dispatch, SHM
pool plumbing, X11 pixel-format decoding, GDI BitBlt + `GetDIBits`, CoreGraphics
display walking) is written in this crate.

---

## What you get

- **Every capture mode on every OS.** Full desktop, single monitor, single
  window, arbitrary cross-monitor region, monitor-at-cursor,
  monitor-at-point. If a backend genuinely can't fulfil a request (e.g.
  pointer position on Wayland) you get a *typed* error — never a panic or a
  silent empty buffer.
- **Logical coordinate space everywhere.** Rotation, panel orientation, and
  HiDPI scaling are factored out by the crate. Pass pixels in, get pixels
  out, no off-by-one between OSes.
- **Auto backend selection with override.** On Linux the builder prefers
  native `wlr-screencopy` when the compositor advertises it, falls back to
  `xdg-desktop-portal` (GNOME, KDE), then to X11 via XWayland.
  `Capturer::builder().backend(BackendKind::X11).build()` forces a specific
  backend.
- **Strong typing.** Newtype `MonitorId` / `WindowId`, ordered enums for
  rotation and backend, `#[non_exhaustive]` `CaptureError` so callers can
  `match` exhaustively, dedicated variants for every failure mode
  (`MonitorNotFound`, `PointOutsideDesktop`, `CursorUnavailable`,
  `Cancelled`, `Timeout`, …).
- **No `unsafe` outside narrowly-scoped FFI.** The Win32 and macOS backends
  use `unsafe` only where the C ABI requires it; the Wayland and X11
  backends are entirely safe code.

---

## Type-system tour

```rust
pub struct Capturer { /* … */ }
pub struct CapturerBuilder { /* … */ }
pub struct CaptureOptions { pub show_cursor: bool, pub retry_on_failure: bool }

pub enum BackendKind {
    Auto,
    Wayland, WaylandPortal, X11,
    WindowsGdi, WindowsDxgi,
    MacOS,
}

pub struct Point { pub x: i32, pub y: i32 }
pub struct Size  { pub width: u32, pub height: u32 }
pub struct Rect  { pub origin: Point, pub size: Size }
pub type   Area  = Rect;     // alias, for scapture-style call sites

pub enum Rotation {
    Normal, Rotate90, Rotate180, Rotate270,
    Flipped, Flipped90, Flipped180, Flipped270,
}

pub struct MonitorId(/* opaque */);
pub struct WindowId (/* opaque */);
pub struct Monitor  { /* id, name, bounds, scale, rotation, refresh_rate, primary */ }
pub struct Window   { /* id, title, app_name, bounds, monitor, min/max/focus flags */ }

pub struct WindowSearch { id?, title_contains?, app_contains? }
// `From<u32>`, `From<&str>`, `From<String>`, `From<WindowId>`

pub struct Image { /* wraps image::RgbaImage */ }

#[non_exhaustive]
pub enum CaptureError {
    NoMonitors,
    NoWindows,
    MonitorNotFound(MonitorId),
    WindowNotFound(WindowId),
    PointOutsideDesktop { x: i32, y: i32 },
    RegionOutsideDesktop(Rect),
    EmptyRegion(Rect),
    CursorUnavailable(String),
    NoBackend(Vec<String>),
    Unsupported { backend: &'static str, detail: String },
    Timeout(Duration),
    Cancelled,
    Backend { backend: &'static str, detail: String },
    Io(std::io::Error),
    ImageConversion(String),
}
```

---

## Cookbook

### Enumerate monitors

```rust
for m in Capturer::new()?.monitors()? {
    println!("{m}");
}
```

### Capture the monitor under the cursor

```rust
let cap = Capturer::new()?;
cap.capture_at_cursor()?.save("here.png")?;
```

### Capture by point

```rust
use sss_capture::Point;
cap.capture_at(Point::new(2500, 600))?;
```

### Cross-monitor region

```rust
use sss_capture::Rect;
// Top-half of two 1920×1080 monitors side by side:
cap.capture_region(Rect::from_xywh(0, 0, 3840, 540))?;
```

### Capture a window by title

```rust
let win = cap.window_by_title("Firefox")?;
cap.capture_window(&win)?.save("firefox.png")?;
```

### Include the cursor

```rust
use sss_capture::CaptureOptions;
cap.capture_all_with(CaptureOptions::with_cursor())?;
```

### Force a backend

```rust
use sss_capture::{Capturer, BackendKind};
let cap = Capturer::builder()
    .backend(BackendKind::X11)            // fall back to XWayland on Wayland
    .show_cursor(true)
    .build()?;
```

### Interop with the `image` crate

```rust
use sss_capture::image::RgbaImage;

let rgba: RgbaImage = sss_capture::Capturer::new()?.capture_all()?.into_rgba();
```

---

## Examples

```bash
cargo run -p sss_capture --example list_monitors
cargo run -p sss_capture --example list_windows
cargo run -p sss_capture --example capture_all       -- /tmp/desktop.png
cargo run -p sss_capture --example capture_primary   -- /tmp/primary.png
cargo run -p sss_capture --example capture_at_cursor -- /tmp/cursor.png
cargo run -p sss_capture --example capture_region    -- 0,0 1920x1080 /tmp/region.png
cargo run -p sss_capture --example capture_window    -- Firefox /tmp/window.png
cargo run -p sss_capture --example select_backend    -- wayland /tmp/wl.png
```

---

## Caveats (real ones, not hidden footguns)

- **Cursor position on Wayland.** The protocol intentionally hides the
  pointer location from apps. We return `CaptureError::CursorUnavailable`.
  Callers who track the pointer through their own surface can use
  [`Capturer::capture_at`] to supply a `Point` directly.
- **Window enumeration on pure Wayland.** Available through
  `zwlr_foreign_toplevel_v1` (wlroots) or `ext_foreign_toplevel_list_v1`
  (modern compositors). Neither is required by the core spec — when both
  are missing `windows()` returns an empty list. The portal backend has no
  window enumeration at all.
- **Window capture on Wayland.** `wlr-screencopy` doesn't have a
  per-window capture call. Use the portal backend on GNOME/KDE, or X11 via
  XWayland.
- **DXGI Desktop Duplication on Windows.** Currently mapped to the same
  implementation as GDI (`BackendKind::WindowsDxgi` resolves to the GDI
  path). The DXGI route would require a `Direct3D11` device per session and
  a continuous frame loop; the GDI path is simpler, capture-on-demand, and
  works in RDP / Citrix sessions where DXGI does not.

---

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option, matching the rest of
the workspace.

[repo]: https://github.com/SergioRibera/sss
