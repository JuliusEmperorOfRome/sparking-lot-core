#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod loom;
mod park;
mod parking_lot;

/// Parks the current thread on `addr` until notified,
/// but only if `expected` returns true.
///
/// # Safety
///
/// While this function itself is safe, using addresses that you
/// don't own is highly discouraged. This is because if multiple
/// libraries/modules/anything park on the same address without
/// knowledge of each other, it will cause something that from
/// their perspective looks like spurious wake-ups, which `park`
/// guarantees not to happen (unlike [`std::thread::park`]).
///
/// # Notes
///
/// - The memory pointed to by `addr` isn't writter to,
/// it isn't read and no references to it are formed.
/// - This function ensures that if  another thread does
/// something that would cause `expected` to return false
/// and only then calls [`unpark_one`] or [`unpark_all`],
/// `park` will either be woken up or will not sleep.
///
///
/// # Example
///
/// ```rust,no_run
/// use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
/// static WAKE_UP: AtomicBool = AtomicBool::new(false);
///
/// fn wait_for_event() {
///     //SAFETY: remember not to park on WAKE_UP in unrelated functions.
///     unsafe {
///         sparking_lot_core::park(&WAKE_UP as *const _ as *const _, || {
///             WAKE_UP.load(Relaxed) == false
///         })
///     }
/// }
///
/// fn notify_event_happened() {
///     //If these lines are reordered park may miss this notification
///     WAKE_UP.store(true, Relaxed);
///     sparking_lot_core::unpark_one(&WAKE_UP as *const _ as *const _)
/// }
/// ```
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub unsafe fn park(addr: *const (), expected: impl FnOnce() -> bool) {
    parking_lot::park(addr, expected)
}

/// Wakes one thread [`parked`](park()) on `addr`.
///
/// Should be called after making the `expected` of
/// the corresponding [`park`](park()) return false.
///
/// If no thread is waiting on `addr`, no thread
/// is woken, but it still requires locking, so it's
/// not recommended to call it without reason.
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub fn unpark_one(addr: *const ()) {
    parking_lot::unpark_one(addr);
}

/// Wakes at most `count` threads [`parked`](park()) on `addr`.
/// Should be called after making the `expected` of
/// the corresponding [`parks`](park()) return false.
///
/// If no thread is waiting on `addr`, no thread
/// is woken, but it still requires locking, so it's
/// not recommended to call it without reason.
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub fn unpark_some(addr: *const (), count: usize) {
    parking_lot::unpark_some(addr, count);
}

/// Wakes all threads [`parked`](park()) on `addr`.
///
/// Should be called after making the `expected` of
/// the corresponding [`parks`](park()) return false.
///
/// If no thread is waiting on `addr`, no thread
/// is woken, but it still requires locking, so it's
/// not recommended to call it without reason.
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub fn unpark_all(addr: *const ()) {
    parking_lot::unpark_all(addr);
}
