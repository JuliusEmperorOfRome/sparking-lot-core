pub(crate) trait ParkerT {
    const CHEAP_NEW: bool;
    /// # Safety
    ///
    /// - can only be called by one 'owner' thread
    unsafe fn park(&self);
    /// # Safety
    ///
    /// - must point to a living `Self`
    unsafe fn unpark(this: *const Self);
}

cfg_if::cfg_if! {

if #[cfg(feature = "thread-parker")] {
    mod std_thread;
    pub(crate) use std_thread::Parker;
}
else {// default to the old impl
    mod std_mutex;
    pub(crate) use std_mutex::Parker;
}

}

#[cfg(all(test, loom))]
mod tests {
    use super::{Parker, ParkerT};
    use core::sync::atomic::Ordering::{Acquire, Release};
    use loom::cell::Cell;
    use loom::thread;
    use std::ops::Deref;
    use std::sync::Arc;

    #[test]
    fn keeps_unpark() {
        loom::model(|| {
            let parker = Parker::new();
            unsafe { ParkerT::unpark(&parker) };
            unsafe { parker.park() };
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
                    unsafe { ParkerT::unpark(parker.deref()) };
                });
            }
            unsafe { parker.park() };
            assert!(WROTE.get());
        });
    }

    #[test]
    fn synchronises_multiple_parkers() {
        loom::model(|| {
            use core::sync::atomic::Ordering::Relaxed;
            use loom::sync::atomic::AtomicUsize;
            loom::lazy_static! {
                static ref PARKER1: Parker = Parker::new();
                static ref PARKER2: Parker = Parker::new();
                static ref WROTE: Cell<bool> = Cell::new(false);
                static ref INIT: AtomicUsize = AtomicUsize::new(0);
            }

            let h1 = thread::spawn(|| {
                let parker = &*PARKER1;
                INIT.fetch_add(1, Relaxed);
                unsafe { parker.park() };
                assert_eq!(WROTE.get(), true);
            });

            let h2 = thread::spawn(|| {
                let parker = &*PARKER2;
                INIT.fetch_add(1, Relaxed);
                unsafe { parker.park() };
                assert_eq!(WROTE.get(), true);
            });

            while INIT.load(Relaxed) != 2 {
                loom::thread::yield_now();
            }
            WROTE.set(true);
            unsafe { ParkerT::unpark(PARKER1.deref()) };
            unsafe { ParkerT::unpark(PARKER2.deref()) };
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
                unsafe { parker.park() };
            });
            let mut parker = PARKER.load(Acquire);
            while parker.is_null() {
                thread::yield_now();
                parker = PARKER.load(Acquire);
            }
            unsafe { ParkerT::unpark(parker) };
            h.join().unwrap();
        });
    }
}
