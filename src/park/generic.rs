use crate::loom::{Condvar, Mutex};
pub(crate) struct Parker {
    should_unpark: Mutex<bool>,
    condvar: Condvar,
}

impl Parker {
    #[cfg(not(loom))]
    pub(crate) const fn new() -> Self {
        Self {
            should_unpark: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }
    #[cfg(loom)]
    pub(crate) fn new() -> Self {
        Self {
            should_unpark: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    pub(crate) fn park(&self) {
        /* # Note
         *
         * The only points in `park` and `unpark` that may panic are
         * `Mutex::lock()`, `Condvar::wait()` and `Condvar::notify_one()`.
         * Furthermore, `Mutex::lock()` is never called reentrantly and
         * `Condvar::wait()` is only called with `self.should_unpark`.
         * This means that if any of them panicked, it was a system error.
         * Furthermore, `std::sync::{Condvar, Mutex}` currently only check
         * for system errors in debug.
         *
         * There is a problem though. If `park` panics in `parking_lot::park`
         * it's thread data has to be unlinked or the program has to be aborted,
         * since failing to do so is likely to result in using ThreadData after
         * the thread was destroyed.
         *
         * It could be replaced with a spinlock, which wouldn't panic, but it's
         * only acceptable if most platforms don't use it. And as such:
         *
         * TODO: use spinlock after implementing linux & windows `Parker`s
         */
        let mut should_unpark = self.should_unpark.lock().unwrap();
        loop {
            if *should_unpark {
                *should_unpark = false;
                return;
            }
            should_unpark = self.condvar.wait(should_unpark).unwrap();
        }
    }

    pub(crate) fn unpark(&self) {
        // See note in `park`
        let mut should_unpark = self.should_unpark.lock().unwrap();
        if !*should_unpark {
            *should_unpark = true;
            self.condvar.notify_one();
        }
    }
}

unsafe impl Send for Parker {}
unsafe impl Sync for Parker {}

#[cfg(all(test, loom))]
mod tests {
    use super::Parker;
    use crate::loom::Cell;
    use core::sync::atomic::Ordering::{Acquire, Release};
    use loom::thread;
    use std::sync::Arc;

    #[test]
    fn keeps_unpark() {
        loom::model(|| {
            let parker = Parker::new();
            parker.unpark();
            parker.park();
        });
    }

    #[test]
    fn synchronises_one() {
        loom::model(|| {
            loom::lazy_static! {
                static ref WROTE: Cell<bool> = Cell::new(false);
            }
            let parker = Arc::new(Parker::new());
            {
                let parker = parker.clone();
                thread::spawn(move || {
                    WROTE.set(true);
                    parker.unpark();
                });
            }
            parker.park();
            assert!(WROTE.get());
        });
    }

    #[test]
    fn synchronises_multiple_unparks() {
        loom::model(|| {
            loom::lazy_static! {
                static ref WROTE: Cell<bool> = Cell::new(false);
            }
            let parker = Arc::new(Parker::new());
            {
                let parker = parker.clone();
                thread::spawn(move || {
                    WROTE.set(true);
                    {
                        let parker = parker.clone();
                        thread::spawn(move || parker.unpark());
                    }
                    parker.unpark();
                });
            }
            parker.park();
            assert!(WROTE.get());
        });
    }

    #[test]
    fn synchronises_multiple_parkers() {
        loom::model(|| {
            loom::lazy_static! {
                static ref PARKER1: Parker = Parker::new();
                static ref PARKER2: Parker = Parker::new();
                static ref WROTE: Cell<bool> = Cell::new(false);
            }

            let h1 = thread::spawn(|| {
                PARKER1.park();
                assert_eq!(WROTE.get(), true);
            });

            let h2 = thread::spawn(|| {
                PARKER2.park();
                assert_eq!(WROTE.get(), true);
            });

            WROTE.set(true);
            PARKER1.unpark();
            PARKER2.unpark();
            h1.join().unwrap();
            h2.join().unwrap();
        });
    }

    #[test]
    fn lives_long() {
        loom::model(|| {
            use loom::sync::atomic::AtomicPtr;
            loom::lazy_static! {
                static ref PARKER: AtomicPtr<Parker> = AtomicPtr::new(core::ptr::null_mut());
            }
            let h = thread::spawn(|| {
                let parker = Parker::new();
                PARKER.store(&parker as *const _ as *mut _, Release);
                parker.park();
            });
            let mut parker = PARKER.load(Acquire);
            while parker.is_null() {
                thread::yield_now();
                parker = PARKER.load(Acquire);
            }
            unsafe { &*(parker as *const Parker) }.unpark();
            h.join().unwrap();
        });
    }
}
