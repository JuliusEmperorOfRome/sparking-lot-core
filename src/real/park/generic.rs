use crate::real::loom::{Condvar, Mutex};
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
