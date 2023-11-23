// at the moment, the only impl
pub(crate) mod generic;
pub(crate) use generic::Parker;

#[cfg(all(test, loom))]
mod tests {
    use super::Parker;
    use core::sync::atomic::Ordering::{Acquire, Release};
    use loom::cell::Cell;
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
                parker.park();
                assert_eq!(WROTE.get(), true);
            });

            let h2 = thread::spawn(|| {
                let parker = &*PARKER2;
                INIT.fetch_add(1, Relaxed);
                parker.park();
                assert_eq!(WROTE.get(), true);
            });

            while INIT.load(Relaxed) != 2 {
                loom::thread::yield_now();
            }
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
