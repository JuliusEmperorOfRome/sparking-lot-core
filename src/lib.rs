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

#[cfg(all(loom, test))]
mod tests {
    use loom::sync::atomic::AtomicUsize;
    use loom::thread;
    use std::sync::{atomic::Ordering::Relaxed, Arc};

    #[test]
    fn unpark_one() {
        loom::model(|| {
            let arc = Arc::new(AtomicUsize::new(0));

            let h = {
                let arc = arc.clone();
                thread::spawn(move || {
                    arc.store(1, Relaxed);
                    super::unpark_one(0 as *const ());
                })
            };
            unsafe { super::park(0 as *const (), || arc.load(Relaxed) == 0) };
            assert_eq!(arc.load(Relaxed), 1);
            h.join().unwrap();
        });
    }

    #[test]
    fn unpark_all() {
        loom::model(|| {
            let arc = Arc::new(AtomicUsize::new(0));
            let h1 = {
                let arc = arc.clone();
                thread::spawn(move || {
                    unsafe { super::park(0 as *const (), || arc.load(Relaxed) == 0) };
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h2 = {
                let arc = arc.clone();
                thread::spawn(move || {
                    unsafe { super::park(0 as *const (), || arc.load(Relaxed) == 0) };
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            arc.store(1, Relaxed);
            super::unpark_all(0 as *const ());
            h1.join().unwrap();
            h2.join().unwrap();
        });
    }

    #[test]
    fn unpark_one_bucket_collision() {
        loom::model(|| {
            let arc1 = Arc::new(AtomicUsize::new(0));
            let arc2 = Arc::new(AtomicUsize::new(0));
            let h1 = {
                let arc1 = arc1.clone();
                thread::spawn(move || {
                    arc1.store(1, Relaxed);
                    super::unpark_one(0 as *const ());
                })
            };
            let h2 = {
                let arc2 = arc2.clone();
                thread::spawn(move || {
                    arc2.store(1, Relaxed);
                    super::unpark_one(2 as *const ());
                })
            };
            unsafe { super::park(0 as *const (), || arc1.load(Relaxed) == 0) };
            assert_eq!(arc1.load(Relaxed), 1);
            unsafe { super::park(2 as *const (), || arc2.load(Relaxed) == 0) };
            assert_eq!(arc2.load(Relaxed), 1);
            h1.join().unwrap();
            h2.join().unwrap();
        });
    }

    #[test]
    #[ignore] //takes ~15 min
    fn unpark_all_bucket_collision_var1() {
        loom::model(|| {
            let arc1 = Arc::new(AtomicUsize::new(0));
            let arc2 = Arc::new(AtomicUsize::new(0));
            let h1 = {
                let arc = arc1.clone();
                thread::spawn(move || {
                    unsafe { super::park(0 as *const (), || arc.load(Relaxed) == 0) };
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h2 = {
                let arc = arc1.clone();
                thread::spawn(move || {
                    unsafe { super::park(0 as *const (), || arc.load(Relaxed) == 0) };
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h3 = {
                let arc = arc2.clone();
                thread::spawn(move || {
                    unsafe { super::park(2 as *const (), || arc.load(Relaxed) == 0) };
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };

            arc1.store(1, Relaxed);
            super::unpark_all(0 as *const ());
            h1.join().unwrap();
            h2.join().unwrap();

            arc2.store(1, Relaxed);
            super::unpark_all(2 as *const ());
            h3.join().unwrap();
        });
    }

    #[test]
    #[ignore] //takes ~30 min
    fn unpark_all_bucket_collision_var2() {
        loom::model(|| {
            let arc1 = Arc::new(AtomicUsize::new(0));
            let arc2 = Arc::new(AtomicUsize::new(0));
            let h1 = {
                let arc = arc1.clone();
                thread::spawn(move || {
                    unsafe { super::park(0 as *const (), || arc.load(Relaxed) == 0) };
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h2 = {
                let arc = arc1.clone();
                thread::spawn(move || {
                    unsafe { super::park(0 as *const (), || arc.load(Relaxed) == 0) };
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h3 = {
                let arc = arc2.clone();
                thread::spawn(move || {
                    unsafe { super::park(2 as *const (), || arc.load(Relaxed) == 0) };
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };

            arc2.store(1, Relaxed);
            super::unpark_all(2 as *const ());
            h3.join().unwrap();

            arc1.store(1, Relaxed);
            super::unpark_all(0 as *const ());
            h1.join().unwrap();
            h2.join().unwrap();
        });
    }
}
