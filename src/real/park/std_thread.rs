use core::marker::PhantomPinned;
use core::pin::Pin;
use core::ptr::{self, addr_of};
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use std::ptr::NonNull;

use crate::real::loom::thread::{self, Thread};
use crate::real::loom::Cell;
use crate::real::loom::{AtomicBool, AtomicPtr};

use super::ParkerT;

pub struct Parker(AtomicPtr<ParkEvent>);

impl ParkerT for Parker {
    const CHEAP_NEW: bool = true;

    unsafe fn park(&self) {
        /* If `self.0` == notified, then `unpark` was called and we need to
         * synchronise with it with `Acquire` on success.
         */
        if self
            .0
            .compare_exchange(Self::notified().as_ptr(), ptr::null_mut(), Acquire, Relaxed)
            .is_err()
        {
            ParkEvent::with(|event| {
                let old = self.0.swap(event.get_ref() as *const _ as *mut _, AcqRel);
                if old != Self::notified().as_ptr() {
                    #[cfg(loom)]
                    assert_eq!(old, ptr::null_mut());
                    #[cfg(not(loom))]
                    debug_assert_eq!(old, ptr::null_mut());
                    // The event is now registered for unparking
                    event.wait();
                }
                self.0.store(ptr::null_mut(), Release);
            });
        }
    }

    unsafe fn unpark(this: *const Self) {
        if let Some(event) = NonNull::new((*this).0.swap(Self::notified().as_ptr(), AcqRel)) {
            #[cfg(not(loom))]
            debug_assert_ne!(event, Self::notified());
            #[cfg(loom)]
            assert_ne!(event, Self::notified());

            ParkEvent::signal(event.as_ptr());
        }
    }
}

impl Parker {
    fn notified() -> NonNull<ParkEvent> {
        static NOTIFIED: u8 = 0;
        let ret = NonNull::from(&NOTIFIED).cast();
        ret
    }

    #[cfg(not(loom))]
    pub(crate) const fn new() -> Self {
        Self(AtomicPtr::new(ptr::null_mut()))
    }

    #[cfg(loom)]
    pub(crate) fn new() -> Self {
        Self(AtomicPtr::new(ptr::null_mut()))
    }
}

struct ParkEvent {
    thread: Cell<Option<Thread>>,
    signaled: AtomicBool,
    _pin: PhantomPinned,
}

impl ParkEvent {
    #[cold]
    #[inline(never)]
    fn with<R>(f: impl FnOnce(Pin<&Self>) -> R) -> R {
        let event = core::pin::pin!(Self {
            thread: Cell::new(Some(thread::current())),
            signaled: AtomicBool::new(false),
            _pin: PhantomPinned,
        });
        f(event.as_ref())
    }

    fn wait(self: Pin<&Self>) {
        while !self.signaled.load(Acquire) {
            thread::park();
        }
    }

    /// # Safety
    ///
    /// - `this` must be alive when called.
    /// - `this` must be pinned
    #[cold]
    #[inline(never)]
    unsafe fn signal(this: *const Self) {
        let thread = {
            let maybe_thread = (*this).thread.take();
            #[cfg(not(loom))]
            debug_assert!(maybe_thread.is_some());
            #[cfg(loom)]
            assert!(maybe_thread.is_some());
            unsafe {
                // threads don't park without setting the `thread` field to some
                maybe_thread.unwrap_unchecked()
            }
        };
        let signal_flag = addr_of!((*this).signaled);

        // FIXME (maybe): This is a case of https://github.com/rust-lang/rust/issues/55005.
        (*signal_flag).store(true, Release);

        thread.unpark();
    }
}
