#![cfg(loom)]

use loom::sync::atomic::AtomicUsize;
use loom::thread;

use core::sync::atomic::{AtomicUsize as StdAtomUsize, Ordering::Relaxed};
use std::sync::Arc;

use sparking_lot_core as slc;
use sparking_lot_core::DEFAULT_TOKEN;

mod basic {
    use super::*;

    #[test]
    fn unpark_one() {
        loom::model(|| {
            let arc = Arc::new(AtomicUsize::new(0));

            let h = {
                let arc = arc.clone();
                thread::spawn(move || {
                    arc.store(1, Relaxed);
                    slc::unpark_one(0, DEFAULT_TOKEN);
                })
            };
            unsafe { slc::park(0, || arc.load(Relaxed) == 0) };
            h.join().unwrap();
        });
    }

    #[test]
    fn unpark_some() {
        loom::model(|| {
            let arc = Arc::new(AtomicUsize::new(0));

            let create_waiter = {
                || {
                    let arc = arc.clone();
                    thread::spawn(move || unsafe { slc::park(0, || arc.load(Relaxed) == 0) })
                }
            };

            let h1 = create_waiter();
            let h2 = create_waiter();

            arc.store(1, Relaxed);
            slc::unpark_some(0, 2, DEFAULT_TOKEN);

            h1.join().unwrap();
            h2.join().unwrap();
        });
    }

    #[test]
    fn unpark_all() {
        loom::model(|| {
            let arc = Arc::new(AtomicUsize::new(0));

            let create_waiter = {
                || {
                    let arc = arc.clone();
                    thread::spawn(move || unsafe { slc::park(0, || arc.load(Relaxed) == 0) })
                }
            };

            let h1 = create_waiter();
            let h2 = create_waiter();

            arc.store(1, Relaxed);
            slc::unpark_all(0, DEFAULT_TOKEN);

            h1.join().unwrap();
            h2.join().unwrap();
        });
    }
}

fn spawn_waiter(addr: usize, arc: Arc<AtomicUsize>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        unsafe { slc::park(addr, || arc.load(Relaxed) == 0) };
    })
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
                slc::unpark_one(0, DEFAULT_TOKEN);
            })
        };
        let h2 = {
            let arc2 = arc2.clone();
            thread::spawn(move || {
                arc2.store(1, Relaxed);
                slc::unpark_one(2, DEFAULT_TOKEN);
            })
        };
        unsafe { slc::park(0, || arc1.load(Relaxed) == 0) };
        h1.join().unwrap();
        unsafe { slc::park(2, || arc2.load(Relaxed) == 0) };
        h2.join().unwrap();
    });
}

#[test]
fn unpark_some_walks_bucket() {
    loom::model(|| {
        let arc1 = Arc::new(AtomicUsize::new(0));
        let arc2 = Arc::new(AtomicUsize::new(0));

        let h1 = spawn_waiter(0, arc1.clone());
        let h2 = spawn_waiter(2, arc2.clone());

        arc1.store(1, Relaxed);
        slc::unpark_some(0, 1, DEFAULT_TOKEN);
        h1.join().unwrap();

        arc2.store(1, Relaxed);
        slc::unpark_some(2, 1, DEFAULT_TOKEN);
        h2.join().unwrap();
    })
}

#[test]
fn unpark_some_bucket_collision_lite() {
    loom::model(|| {
        let arc1 = Arc::new(AtomicUsize::new(0));
        let arc2 = Arc::new(AtomicUsize::new(0));

        let h1 = spawn_waiter(0, arc1.clone());
        let h2 = spawn_waiter(2, arc2.clone());

        arc1.store(1, Relaxed);
        slc::unpark_some(0, 2, DEFAULT_TOKEN);
        h1.join().unwrap();

        arc2.store(1, Relaxed);
        slc::unpark_some(2, 2, DEFAULT_TOKEN);
        h2.join().unwrap();
    });
}

#[test]
fn unpark_some_is_bounded_lite() {
    loom::model(|| {
        struct State {
            park_token: AtomicUsize,
            first_park_index: StdAtomUsize, // See note in thread::spawn closure
        }
        let arc = Arc::new(State {
            park_token: AtomicUsize::new(!0),
            first_park_index: StdAtomUsize::new(!0),
        });

        let mut ts: [_; 2] = std::array::from_fn(|i| {
            let arc = arc.clone();
            Some(thread::spawn(move || unsafe {
                slc::park(0, || {
                    /* This atomic isn't loom, but because it's
                     * at the beginning of the thread, loom also
                     * tests the case where this isn't set by
                     * starting this thread late. And since it's
                     * not used to synchornise access to data, the
                     * causality checks by loom aren't needed either.
                     *
                     * This is done because it speeds the test up by
                     * **a lot**.
                     */
                    let _ = arc
                        .first_park_index
                        .compare_exchange(!0, i, Relaxed, Relaxed);
                    arc.park_token.load(Relaxed) == !0
                });
            }))
        });
        arc.park_token.store(0, Relaxed);
        slc::unpark_some(0, 1, DEFAULT_TOKEN);

        match arc.first_park_index.load(Relaxed) {
            x if x == !0 => {}
            i => {
                ts[i].take().map(|t| t.join().unwrap());
            }
        }

        slc::unpark_one(0, DEFAULT_TOKEN);

        for mut t in ts {
            t.take().map(|t| t.join().unwrap());
        }
    });
}

#[test]
fn unpark_all_bucket_collision_lite() {
    loom::model(|| {
        let arc1 = Arc::new(AtomicUsize::new(0));
        let arc2 = Arc::new(AtomicUsize::new(0));

        let h1 = spawn_waiter(0, arc1.clone());
        let h2 = spawn_waiter(2, arc2.clone());

        arc1.store(1, Relaxed);
        slc::unpark_all(0, DEFAULT_TOKEN);
        h1.join().unwrap();

        arc2.store(1, Relaxed);
        slc::unpark_all(2, DEFAULT_TOKEN);
        h2.join().unwrap();
    });
}

/// From loom's perspective, this is magic - threads can communicate
/// about parking permissions without loom ever seeing traffic between
/// threads. This significantly increases loom speeds, but when used
/// incorrectly loom may miss bugs.
struct MagicParkToken(StdAtomUsize);

impl MagicParkToken {
    #[inline(always)]
    const fn new() -> Self {
        Self(StdAtomUsize::new(0))
    }

    #[inline(always)]
    fn reset(&self) {
        self.0.store(0, Relaxed);
    }

    #[inline(always)]
    fn stop_parks(&self) {
        self.0.store(1, Relaxed);
    }

    /// # Safety
    ///
    /// This function must be called only
    /// in contexts where loom can pause it.
    #[inline(always)]
    unsafe fn can_park(&self) -> bool {
        self.0.load(Relaxed) == 0
    }

    /// # Safety
    ///
    /// Make sure loom can test the cases of:
    /// - waiter parks
    /// - waiter unparks
    ///
    /// Since this pattern is used extensively, here is a proof for its
    /// safety:
    ///
    /// ```
    /// let token = MagicParkToken::new();
    /// let h = token.spawn_waiter(<addr>);
    /// token.stop_parks();
    /// unpark_one(<addr>); // or other unpark variant
    /// ```
    ///
    /// The reason this works is because loom can either continue on the main
    /// thread or move to the new one.
    ///
    /// - In the case it chooses to continue on the main thread, it will be guaranteed
    /// to not let the new thread park, testing no parking.
    /// - In the case that execution is moved to the new thread means it gets to a loom
    /// mutex, where it can choose:
    /// 1. move to the main thread, not parking once again.
    /// 2. continue on the new thread, guaranteeing it will park.
    ///
    /// Additionally, when it doesn't park, loom doesn't record any synchronisation, which
    /// adds more cases it considers errors, but no new legal executions are made. When it
    /// does park, the synchronisation is caused by real code, not `MagicParkToken`, so it's
    /// valid to assume it there.
    unsafe fn spawn_waiter(&'static self, addr: usize) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            unsafe { slc::park(addr, || self.can_park()) };
        })
    }
}

// These tests make optimisations that are very brittle,
// but are needed because otherwise these tests are very
// slow (as of in they can take upwards of 6 hours).

#[test]
fn unpark_some_is_bounded_full() {
    loom::model(|| {
        struct State {
            park_token: StdAtomUsize,       // see `MagicUnparkToken`
            first_park_index: StdAtomUsize, // See note in thread::spawn closure
            second_park_index: StdAtomUsize,
        }
        let arc = Arc::new(State {
            park_token: StdAtomUsize::new(!0),
            first_park_index: StdAtomUsize::new(!0),
            second_park_index: StdAtomUsize::new(!0),
        });

        let mut ts: [_; 3] = std::array::from_fn(|i| {
            let arc = arc.clone();
            Some(thread::spawn(move || unsafe {
                slc::park(0, || {
                    /* These atomics aren't loom, but because it's
                     * at the beginning of the thread, loom also
                     * tests the case where this isn't done by
                     * starting this thread late. And since it's
                     * not used to synchornise access to data, the
                     * causality checks by loom aren't needed either.
                     *
                     * This is done because it speeds the test up by
                     * **a lot**.
                     */
                    if arc
                        .first_park_index
                        .compare_exchange(!0, i, Relaxed, Relaxed)
                        .is_err()
                    {
                        let _ = arc
                            .second_park_index
                            .compare_exchange(!0, i, Relaxed, Relaxed);
                    }
                    arc.park_token.load(Relaxed) == !0
                });
            }))
        });
        arc.park_token.store(0, Relaxed);
        slc::unpark_some(0, 2, DEFAULT_TOKEN);

        match arc.first_park_index.load(Relaxed) {
            x if x == !0 => {}
            i => {
                ts[i].take().map(|t| t.join().unwrap()).unwrap();
                match arc.second_park_index.load(Relaxed) {
                    x if x == !0 => {}
                    i => ts[i].take().map(|t| t.join().unwrap()).unwrap(),
                }
            }
        }

        slc::unpark_one(0, DEFAULT_TOKEN);

        for mut t in ts {
            t.take().map(|t| t.join().unwrap());
        }
    });
}

#[test]
fn unpark_all_bucket_collision_var1() {
    static TOKEN1: MagicParkToken = MagicParkToken::new();
    static TOKEN2: MagicParkToken = MagicParkToken::new();
    loom::model(|| {
        TOKEN1.reset();
        TOKEN2.reset();
        /* SAFETY:
         * - see note on `MagicParkToken::spawn_waiter`
         * - loom can interleave any of these threads to
         * cause every possible state.
         */
        let (h1, h2, h3) = unsafe {
            (
                TOKEN1.spawn_waiter(0),
                TOKEN1.spawn_waiter(0),
                TOKEN2.spawn_waiter(2),
            )
        };

        TOKEN1.stop_parks();
        slc::unpark_all(0, DEFAULT_TOKEN);
        h1.join().unwrap();
        h2.join().unwrap();

        TOKEN2.stop_parks();
        slc::unpark_all(2, DEFAULT_TOKEN);
        h3.join().unwrap();
    });
}

#[test]
fn unpark_all_bucket_collision_var2() {
    static TOKEN1: MagicParkToken = MagicParkToken::new();
    static TOKEN2: MagicParkToken = MagicParkToken::new();
    loom::model(|| {
        TOKEN1.reset();
        TOKEN2.reset();

        /* SAFETY:
         * - see note on `MagicParkToken::spawn_waiter`
         * - loom can interleave any of these threads to
         * cause every possible state.
         */
        let (h1, h2, h3) = unsafe {
            (
                TOKEN1.spawn_waiter(0),
                TOKEN1.spawn_waiter(0),
                TOKEN2.spawn_waiter(2),
            )
        };

        TOKEN2.stop_parks();
        slc::unpark_all(2, DEFAULT_TOKEN);
        h3.join().unwrap();

        TOKEN1.stop_parks();
        slc::unpark_all(0, DEFAULT_TOKEN);
        h1.join().unwrap();
        h2.join().unwrap();
    });
}

// Should be the same as `unpark_all_bucket_collision_var1`,
// but with the first `unpark_all(...)` replaced by
// `unpark_some(..., 4)` and the second by `unpark_some(..., 2)`
#[test]
fn unpark_some_bucket_collision_var1() {
    static TOKEN1: MagicParkToken = MagicParkToken::new();
    static TOKEN2: MagicParkToken = MagicParkToken::new();
    loom::model(|| {
        TOKEN1.reset();
        TOKEN2.reset();

        /* SAFETY:
         * - see note on `MagicParkToken::spawn_waiter`
         * - loom can interleave any of these threads to
         * cause every possible state.
         */
        let (h1, h2, h3) = unsafe {
            (
                TOKEN1.spawn_waiter(0),
                TOKEN1.spawn_waiter(0),
                TOKEN2.spawn_waiter(2),
            )
        };

        TOKEN1.stop_parks();
        slc::unpark_some(0, 4, DEFAULT_TOKEN);
        h1.join().unwrap();
        h2.join().unwrap();

        TOKEN2.stop_parks();
        slc::unpark_some(2, 2, DEFAULT_TOKEN);
        h3.join().unwrap();
    });
}

// Should be the same as `unpark_all_bucket_collision_var1`,
// but with the first `unpark_all(...)` replaced by
// `unpark_some(..., 4)` and the second by `unpark_some(..., 3)`
#[test]
fn unpark_some_bucket_collision_var2() {
    static TOKEN1: MagicParkToken = MagicParkToken::new();
    static TOKEN2: MagicParkToken = MagicParkToken::new();
    loom::model(|| {
        TOKEN1.reset();
        TOKEN2.reset();

        /* SAFETY:
         * - see note on `MagicParkToken::spawn_waiter`
         * - loom can interleave any of these threads to
         * cause every possible state.
         */
        let (h1, h2, h3) = unsafe {
            (
                TOKEN1.spawn_waiter(0),
                TOKEN1.spawn_waiter(0),
                TOKEN2.spawn_waiter(2),
            )
        };

        TOKEN2.stop_parks();
        slc::unpark_some(2, 4, DEFAULT_TOKEN);
        h3.join().unwrap();

        TOKEN1.stop_parks();
        slc::unpark_some(0, 3, DEFAULT_TOKEN);
        h1.join().unwrap();
        h2.join().unwrap();
    });
}
