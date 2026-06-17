//! Native Wayland clipboard via `zwlr_data_control_manager_v1`.
//!
//! This sidesteps `arboard`'s fork-and-stay-alive strategy. Instead we:
//!
//!   1. Connect to the compositor, bind `zwlr_data_control_manager_v1`.
//!   2. Create a data source that offers the image as `image/png` (+ a few
//!      common alternatives so paste-targets can negotiate).
//!   3. Set it as the regular `clipboard` selection.
//!   4. Spin the wayland event loop. The compositor / clipboard manager
//!      requests `send` (we write the PNG bytes to the fd), and as soon
//!      as the manager publishes its own selection ours is `cancelled`,
//!      which lets us return to the caller — no daemonised zombie.
//!
//! When no manager is around, the first paste also fires `cancelled`
//! (some apps grab and re-set the clipboard), and worst case we hit a
//! short timeout and return anyway. The caller is expected to fall back
//! to `arboard` if this returns `ClipboardUnavailable`.

use std::fs::File;
use std::io::Write;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

/// How long we'll keep the wayland event loop alive after setting the
/// selection. Long enough for any clipboard manager to read + take over
/// (DankMaterialShell, cliphist, clipman all do this within ~50ms),
/// short enough that we never leave the user hanging if no manager
/// responds (without this the overlay's exit path blocks here on join).
const HANDOFF_TIMEOUT: Duration = Duration::from_millis(800);

/// Result of [`copy_png`].
#[derive(Debug, thiserror::Error)]
pub enum WlClipboardError {
    /// `$WAYLAND_DISPLAY` not set or the socket isn't reachable.
    #[error("not on a Wayland session")]
    NotOnWayland,
    /// The compositor doesn't advertise `zwlr_data_control_manager_v1`. The
    /// caller should fall back to a different clipboard mechanism (typically
    /// arboard).
    #[error("compositor lacks zwlr_data_control_manager_v1")]
    ClipboardUnavailable,
    /// Catch-all for protocol / I/O errors.
    #[error("wayland clipboard: {0}")]
    Other(String),
}

/// Publish `png_bytes` as the clipboard's `image/png` selection.
pub fn copy_png(png_bytes: Vec<u8>) -> Result<(), WlClipboardError> {
    copy_bytes(png_bytes, &["image/png", "image/x-png", "image/jpeg"])
}

/// Publish `text` as the clipboard's text selection.
///
/// Offers the full UTF-8 MIME set so GTK / Qt / xdotool / vim-paste all
/// negotiate without falling back to STRING (latin-1).
pub fn copy_text(text: String) -> Result<(), WlClipboardError> {
    copy_bytes(
        text.into_bytes(),
        &[
            "text/plain;charset=utf-8",
            "text/plain;charset=UTF-8",
            "UTF8_STRING",
            "text/plain",
            "STRING",
            "TEXT",
        ],
    )
}

/// Publish `bytes` under the given MIME-type set and wait for receiver(s).
///
/// Returns once we've served at least one `send` and either received a
/// `cancelled` event (a clipboard manager took over — we can exit safely),
/// or [`HANDOFF_TIMEOUT`] elapsed.
fn copy_bytes(bytes: Vec<u8>, mime_types: &[&str]) -> Result<(), WlClipboardError> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(WlClipboardError::NotOnWayland);
    }

    let conn = Connection::connect_to_env()
        .map_err(|e| WlClipboardError::Other(format!("connect: {e}")))?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)
        .map_err(|e| WlClipboardError::Other(format!("registry: {e}")))?;
    let qh = queue.handle();

    let manager: ZwlrDataControlManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .map_err(|_| WlClipboardError::ClipboardUnavailable)?;
    let seat: WlSeat = globals
        .bind(&qh, 1..=8, ())
        .map_err(|_| WlClipboardError::Other("no wl_seat".into()))?;

    let bytes = Arc::new(bytes);
    let source = manager.create_data_source(&qh, bytes.clone());
    for mime in mime_types {
        source.offer((*mime).to_string());
    }
    let device = manager.get_data_device(&seat, &qh, ());
    device.set_selection(Some(&source));

    let mut state = State {
        served: 0,
        cancelled: false,
    };
    let deadline = Instant::now() + HANDOFF_TIMEOUT;
    if let Err(e) = dispatch_until(&conn, &mut queue, &mut state, deadline) {
        tracing::warn!(error = %e, "wayland clipboard dispatch failed");
    }

    tracing::info!(
        served = state.served,
        cancelled = state.cancelled,
        elapsed_ms = ?(HANDOFF_TIMEOUT.saturating_sub(deadline.saturating_duration_since(Instant::now()))).as_millis(),
        "wayland clipboard handoff complete",
    );

    // Politely tear down. If we were cancelled the source is already
    // invalid; destroying is still safe.
    source.destroy();
    device.destroy();
    drop(queue);
    drop(conn);
    Ok(())
}

// -----------------------------------------------------------------------
// State + dispatch
// -----------------------------------------------------------------------

struct State {
    served: u32,
    cancelled: bool,
}

fn dispatch_until(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    deadline: Instant,
) -> Result<(), WlClipboardError> {
    use rustix::event::{poll, PollFd, PollFlags};

    loop {
        let _ = queue.flush();
        // Drain anything already queued so we honour cancelled / send
        // events that may have arrived during the previous round.
        queue
            .dispatch_pending(state)
            .map_err(|e| WlClipboardError::Other(format!("dispatch_pending: {e}")))?;

        // Have we received at least one send AND been cancelled? Then a
        // clipboard manager has taken over and there's nothing left to
        // serve — we can safely return.
        if state.cancelled && state.served > 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Timeout. Either nothing read our selection (no manager + no
            // paste), or the manager hasn't sent cancelled yet. Return
            // anyway — staying around longer would only make the user wait.
            return Ok(());
        }

        let guard = match conn.prepare_read() {
            Some(g) => g,
            None => {
                queue
                    .dispatch_pending(state)
                    .map_err(|e| WlClipboardError::Other(format!("dispatch_pending: {e}")))?;
                continue;
            }
        };
        let fd = guard.connection_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        match poll(&mut fds, None) {
            Ok(0) => continue,
            Ok(_) => {
                guard
                    .read()
                    .map_err(|e| WlClipboardError::Other(format!("read events: {e}")))?;
            }
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(WlClipboardError::Other(format!("poll: {e}"))),
        }
    }
}

// -----------------------------------------------------------------------
// Dispatch impls
// -----------------------------------------------------------------------

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlSourceV1, Arc<Vec<u8>>> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        bytes: &Arc<Vec<u8>>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type: _, fd } => {
                // The compositor / a paste-target requested our data. Write
                // the PNG bytes into the fd and close it.
                serve(fd, bytes);
                state.served = state.served.saturating_add(1);
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                // The compositor invalidated our source — typically because
                // a clipboard manager (or anyone else) published a new
                // selection. We can exit now.
                state.cancelled = true;
            }
            _ => {}
        }
    }
}

fn serve(fd: OwnedFd, bytes: &[u8]) {
    // SAFETY: `fd` is freshly owned, we take exclusive ownership and let
    // `File`'s `Drop` close it after writing.
    let mut file = unsafe { File::from_raw_fd(std::os::fd::IntoRawFd::into_raw_fd(fd)) };
    if let Err(e) = file.write_all(bytes) {
        tracing::warn!(error = %e, "wayland clipboard: serving Send failed");
    }
    let _ = file.flush();
}

// The data-control device emits events about other clients' selections that
// we don't care about (we're only ever writing). Tracking offers would
// require allocations and a `Dispatch<ZwlrDataControlOfferV1, _>` impl; the
// `data_offer` event is the only one that *requires* us to provide such an
// impl, so we wire a noop for it.
impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        _event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(State, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE =>
            (ZwlrDataControlOfferV1, ())
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlOfferV1,
        _: <ZwlrDataControlOfferV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore ZwlrDataControlManagerV1);
