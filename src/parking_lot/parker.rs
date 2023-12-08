cfg_if::cfg_if! {
    if #[cfg(loom)] {
        use loom as stdlib;
    }
    else {
        use std as stdlib;
    }
}

use stdlib::cell::Cell;
use stdlib::sync::atomic::AtomicBool;
use stdlib::thread::{self, Thread};

use core::mem::ManuallyDrop;
use core::sync::atomic::Ordering::{Acquire, Release};

pub(super) struct Parker {
    signalled: AtomicBool,
    state: Cell<ThreadOrToken>,
}

impl Parker {
    pub(super) fn new() -> Self {
        Self {
            signalled: AtomicBool::new(false),
            state: Cell::new(ThreadOrToken {
                thread: ManuallyDrop::new(thread::current()),
            }),
        }
    }

    /// # Safety
    ///
    /// - can only be called once per
    #[inline]
    pub(super) unsafe fn park(&self) -> u32 {
        while !self.signalled.load(Acquire) {
            thread::park();
        }
        self.state.replace(ThreadOrToken { token: 0 }).token
    }

    #[inline]
    pub(super) unsafe fn unpark(this: *const Self, token: u32) {
        let t =
            ManuallyDrop::into_inner((*this).state.replace(ThreadOrToken { token: token }).thread);
        (*this).signalled.store(true, Release);
        /* NOTE
         * DO NOT USE `*this` AFTER THIS STORE
         *
         * Atomic* are UnsafeCell'ed so we're safe from
         * https://github.com/rust-lang/rust/issues/55005
         */
        t.unpark();
    }
}

union ThreadOrToken {
    thread: ManuallyDrop<Thread>,
    token: u32,
}

#[cfg(all(test, loom))]
mod tests {
    use super::Parker;
    use loom::cell::Cell;
    use std::sync::Arc;

    #[test]
    fn sync() {
        loom::model(|| {
            let shared = Arc::new((Parker::new(), Cell::new(false)));
            let t = {
                let shared = shared.clone();
                loom::thread::spawn(move || {
                    shared.1.set(true);
                    unsafe { Parker::unpark(&shared.0, 42) };
                })
            };

            assert_eq!(unsafe { shared.0.park() }, 42);
            assert!(shared.1.get());

            t.join().unwrap();
        });
    }
}
