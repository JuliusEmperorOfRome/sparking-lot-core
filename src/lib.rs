#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod loom;
mod park;
mod parking_lot;

/// Parks the current thread on `addr` if `expected` is true.
///
/// `expected` is invoked with `addr` under a lock, ensuring that
/// if [`unpark_one`] or [`unpark_all`] is called on `addr` after
/// making `expected` return false, `park` will either have exited
/// or already made the thread wakeable, meaning unparks won't be lost.
/// As such code like this will not deadlock.
///
/// ```rust,no_run
/// use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
/// static wake_up: AtomicBool = AtomicBool::new(false);
///
/// fn wait_for_event() {
///     sparking_lot_core::park((&wake_up as *const _).cast(), |ptr| {
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
pub fn park(addr: *const (), expected: impl FnOnce(*const ()) -> bool) {
    parking_lot::park(addr, expected)
}

/// Wakes one thread [`parked`](park) on `addr`.
///
/// If no thread is waiting on `addr`, no thread
/// is woken, but it still requires locking, so it's
/// not recommended to call it without reason.
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub fn unpark_one(addr: *const ()) {
    parking_lot::unpark_one(addr);
}

/// Wakes all threads [`parked`](park) on `addr`.
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
