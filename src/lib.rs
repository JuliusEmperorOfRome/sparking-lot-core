#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod loom;
mod park;
mod parking_lot;

/// Parks the current thread on `addr` until notified,
/// but only if `expected` returns true.
///
/// The argument of `expected` is `addr`. `expected` stays
/// within the scope of `park`.
/// The memory pointed to by `addr` isn't writter to,
/// it isn't read and no references to it are formed.
///
/// # Notes
///
/// This function ensures that if one thread is parking and
/// another thread disables `expected` and only after that
/// calls [`unpark_one`] or [`unpark_all`] at least once,
/// `park` **will** exit.
///
/// It is highly discouraged to use addresses that you don't own.
/// If two libraries/modules/anything else park on the same address
/// without knowledge of each other, it will cause something that
/// from their perspective looks like spurious wake-ups, even though
/// spurious wake-ups don't actually happen with this `park`
/// (unlike [`std::thread::park`]).
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
pub fn park(addr: *const (), expected: impl FnOnce(*const ()) -> bool) {
    parking_lot::park(addr, expected)
}

/// Wakes one thread [`parked`](park()) on `addr`.
///
/// If no thread is waiting on `addr`, no thread
/// is woken, but it still requires locking, so it's
/// not recommended to call it without reason.
///
/// For many threads [`unpark_all`] should be prefered.
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
///
/// For one thread [`unpark_one`] should be prefered.
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
