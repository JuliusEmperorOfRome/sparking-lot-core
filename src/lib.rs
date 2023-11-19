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
/// - The argument of `expected` is `addr`.
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
/// static wake_up: AtomicBool = AtomicBool::new(false);
///
/// fn wait_for_event() {
///     sparking_lot_core::park((&wake_up as *const _).cast(), |ptr| {
///         /* SAFETY:
///          * - `ptr` is the address of `wake_up`
///          *  - `park` doesn't write, read or form references using `ptr`
///          *  - this closure is invoked before `park` returns, so even
///          *  the locals of `wait_for_event` are alive.
///          */
///         let wake_up = unsafe {&*(ptr as *const AtomicBool)};
///         wake_up.load(Relaxed) == false
///     })
/// }
///
/// fn notify_event_happened() {
///     wake_up.store(1, Relaxed);
///     sparking_lot_core::unpark_one((&wake_up as *const _).cast())
/// }
/// ```
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub unsafe fn park(addr: *const (), expected: impl FnOnce(*const ()) -> bool) {
    parking_lot::park(addr, expected)
}

/// Wakes one thread [`parked`](park()) on `addr`.
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
    use crate::loom::AtomicUsize;
    use loom::thread;
    use std::sync::{atomic::Ordering::Relaxed, Arc};

    #[test]
    fn unpark_one() {
        loom::model(|| {
            let arc = Arc::new(AtomicUsize::new(0));

            {
                let arc = arc.clone();
                thread::spawn(move || {
                    arc.store(1, Relaxed);
                    super::unpark_one(0 as *const ());
                });
            }
            super::park(0 as *const (), |_| arc.load(Relaxed) == 0);
            assert_eq!(arc.load(Relaxed), 1);
        });
    }

    #[test]
    fn unpark_all() {
        loom::model(|| {
            let arc = Arc::new(AtomicUsize::new(0));
            let h1 = {
                let arc = arc.clone();
                thread::spawn(move || {
                    super::park(0 as *const (), |_| arc.load(Relaxed) == 0);
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h2 = {
                let arc = arc.clone();
                thread::spawn(move || {
                    super::park(0 as *const (), |_| arc.load(Relaxed) == 0);
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
            {
                let arc1 = arc1.clone();
                thread::spawn(move || {
                    arc1.store(1, Relaxed);
                    super::unpark_one(0 as *const ());
                });
            }
            {
                let arc2 = arc2.clone();
                thread::spawn(move || {
                    arc2.store(1, Relaxed);
                    super::unpark_one(2 as *const ());
                });
            }
            super::park(0 as *const (), |_| arc1.load(Relaxed) == 0);
            assert_eq!(arc1.load(Relaxed), 1);
            super::park(2 as *const (), |_| arc2.load(Relaxed) == 0);
            assert_eq!(arc2.load(Relaxed), 1);
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
                    super::park(0 as *const (), |_| arc.load(Relaxed) == 0);
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h2 = {
                let arc = arc1.clone();
                thread::spawn(move || {
                    super::park(0 as *const (), |_| arc.load(Relaxed) == 0);
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h3 = {
                let arc = arc2.clone();
                thread::spawn(move || {
                    super::park(2 as *const (), |_| arc.load(Relaxed) == 0);
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
                    super::park(0 as *const (), |_| arc.load(Relaxed) == 0);
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h2 = {
                let arc = arc1.clone();
                thread::spawn(move || {
                    super::park(0 as *const (), |_| arc.load(Relaxed) == 0);
                    assert_eq!(arc.load(Relaxed), 1);
                })
            };
            let h3 = {
                let arc = arc2.clone();
                thread::spawn(move || {
                    super::park(2 as *const (), |_| arc.load(Relaxed) == 0);
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
