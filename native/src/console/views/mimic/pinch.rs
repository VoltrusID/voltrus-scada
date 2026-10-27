//! Trackpad pinch for the mimic. GPUI 0.2 does not deliver magnify
//! gestures, so on macOS an AppKit local event monitor reads them as they
//! pass (it hands every event on unchanged) and sends them down a channel
//! the page reads on the UI thread. Elsewhere there is no pinch; ⌘-scroll
//! and ⌘+/⌘− zoom everywhere. A candidate for GPUI itself.

use async_channel::Receiver;

/// A gesture, at a point of the window in points from its bottom-left
/// corner (AppKit's `locationInWindow`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Gesture {
    /// Pinch: zoom by `1 + amount`.
    Magnify { amount: f64, x: f64, y: f64 },
    /// Two-finger double tap.
    SmartMagnify { x: f64, y: f64 },
}

/// Start listening, once per process: the gestures from now on, or `None`
/// when already listening or the platform has no monitor.
pub(crate) fn listen() -> Option<Receiver<Gesture>> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LISTENING: AtomicBool = AtomicBool::new(false);
    if LISTENING.swap(true, Ordering::SeqCst) {
        return None;
    }
    let (sender, receiver) = async_channel::bounded::<Gesture>(64);
    #[cfg(target_os = "macos")]
    {
        mac::install(sender).then_some(receiver)
    }
    #[cfg(not(target_os = "macos"))]
    {
        drop((sender, receiver));
        None
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void};

    use async_channel::Sender;
    use block::ConcreteBlock;

    use super::Gesture;

    type Id = *mut c_void;
    type Sel = *mut c_void;

    /// `NSPoint`: two doubles, returned in registers on arm64 and x86-64.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NsPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    /// `NSEventTypeMagnify` and `NSEventTypeSmartMagnify`.
    const MAGNIFY: u64 = 30;
    const SMART_MAGNIFY: u64 = 32;

    type SendKind = unsafe extern "C" fn(Id, Sel) -> u64;
    type SendFloat = unsafe extern "C" fn(Id, Sel) -> f64;
    type SendPoint = unsafe extern "C" fn(Id, Sel) -> NsPoint;
    type SendMonitor = unsafe extern "C" fn(Id, Sel, u64, *const c_void) -> Id;

    /// `+[NSEvent addLocalMonitorForEventsMatchingMask:handler:]` for the
    /// magnify gestures; whether the monitor is in place.
    pub fn install(sender: Sender<Gesture>) -> bool {
        // SAFETY: `objc_msgSend` is called through the exact signature of
        // each message (an NSUInteger, a CGFloat, an NSPoint, an object);
        // the class and selectors are AppKit's own.
        let (
            class,
            kind_of,
            magnification,
            location,
            add,
            send_kind,
            send_float,
            send_point,
            send_monitor,
        ) = unsafe {
            (
                objc_getClass(c"NSEvent".as_ptr()),
                sel_registerName(c"type".as_ptr()),
                sel_registerName(c"magnification".as_ptr()),
                sel_registerName(c"locationInWindow".as_ptr()),
                sel_registerName(c"addLocalMonitorForEventsMatchingMask:handler:".as_ptr()),
                std::mem::transmute::<unsafe extern "C" fn(), SendKind>(objc_msgSend),
                std::mem::transmute::<unsafe extern "C" fn(), SendFloat>(objc_msgSend),
                std::mem::transmute::<unsafe extern "C" fn(), SendPoint>(objc_msgSend),
                std::mem::transmute::<unsafe extern "C" fn(), SendMonitor>(objc_msgSend),
            )
        };
        if class.is_null() {
            return false;
        }
        let handler = ConcreteBlock::new(move |event: Id| -> Id {
            if event.is_null() {
                return event;
            }
            // SAFETY: `event` is the NSEvent AppKit hands the monitor, of
            // one of the two types asked for; the messages match.
            let gesture = unsafe {
                let at = send_point(event, location);
                match send_kind(event, kind_of) {
                    MAGNIFY => Some(Gesture::Magnify {
                        amount: send_float(event, magnification),
                        x: at.x,
                        y: at.y,
                    }),
                    SMART_MAGNIFY => Some(Gesture::SmartMagnify { x: at.x, y: at.y }),
                    _ => None,
                }
            };
            if let Some(gesture) = gesture {
                let _ = sender.try_send(gesture);
            }
            event
        })
        .copy();
        // SAFETY: the handler block lives for the process (it is never
        // released), as the monitor does.
        let monitor = unsafe {
            send_monitor(
                class,
                add,
                (1 << MAGNIFY) | (1 << SMART_MAGNIFY),
                (&*handler as *const block::Block<(Id,), Id>).cast::<c_void>(),
            )
        };
        std::mem::forget(handler);
        !monitor.is_null()
    }
}
