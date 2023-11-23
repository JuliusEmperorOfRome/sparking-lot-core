#[cfg(not(doc))]
#[cfg(not(all(loom, feature = "loom-test")))]
compile_error!("[internal error] `mod fake` must be used with loom + feature = loom-test");

pub(super) mod parking_lot {
    use core::ptr;
    use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};
    use loom::cell::Cell;
    use loom::sync::atomic::AtomicBool;
    use loom::sync::{Mutex, MutexGuard};
    use loom::thread::Thread;

    struct ThreadData {
        next: Cell<*const ThreadData>,
        addr: Cell<*const ()>,
        parker: Parker,
    }

    impl ThreadData {
        fn new() -> Self {
            Self {
                parker: Parker::new(),
                addr: Cell::new(ptr::null()),
                next: Cell::new(ptr::null()),
            }
        }
    }

    fn lock_bucket(addr: *const ()) -> MutexGuard<'static, Bucket> {
        const ADDRESS_LIMIT: usize = 64;
        use std::cell::Cell as StdCell;
        use std::sync::atomic::AtomicUsize as StdAtomUsize;
        struct Hashtable {
            buckets: [(StdCell<*const ()>, Mutex<Bucket>); ADDRESS_LIMIT],
            assigned_count: StdAtomUsize,
        }
        loom::lazy_static! {
            static ref HASHTABLE: Hashtable = Hashtable {
                assigned_count: StdAtomUsize::new(0),
                buckets: core::array::from_fn(|_| {
                    (
                        StdCell::new(std::ptr::null()),
                        Mutex::new(
                            Bucket {
                                first: Cell::new(std::ptr::null()),
                                last: Cell::new(std::ptr::null()),
                            }
                        ),
                    )
                })
            };
        }

        let len = HASHTABLE.assigned_count.load(Relaxed);
        for bucket in &HASHTABLE.buckets[0..len] {
            if bucket.0.get() == addr {
                return bucket.1.lock().unwrap();
            }
        }
        assert!(
            len < ADDRESS_LIMIT,
            "can't park on more than {ADDRESS_LIMIT} addresses when doing loom tests"
        );
        let entry = &HASHTABLE.buckets[len];
        entry.0.set(addr);
        HASHTABLE.assigned_count.store(len + 1, Relaxed);
        entry.1.lock().unwrap()
    }

    #[inline(always)]
    fn with_thread_data<R>(f: impl FnOnce(&ThreadData) -> R) -> R {
        loom::thread_local!(static THREAD_DATA: ThreadData = ThreadData::new());
        match THREAD_DATA.try_with(|x| x as *const _) {
            Ok(ptr) => unsafe { f(&*ptr) },
            Err(_) => {
                let td = ThreadData::new();
                f(&td)
            }
        }
    }

    pub(crate) fn park(addr: *const (), expected: impl FnOnce() -> bool) {
        with_thread_data(|thread_data| {
            let bucket = lock_bucket(addr);
            if !expected() {
                return;
            }

            thread_data.next.set(ptr::null());
            thread_data.addr.set(addr);

            if bucket.first.get().is_null() {
                bucket.first.set(thread_data);
            } else {
                unsafe {
                    assert!(!bucket.last.get().is_null());
                    &*bucket.last.get()
                }
                .next
                .set(thread_data);
            }
            bucket.last.set(thread_data);
            // not releasing `bucket` lock before parking would deadlock
            drop(bucket);

            thread_data.parker.park();
        });
    }

    pub(crate) fn unpark_one(addr: *const ()) {
        let bucket = lock_bucket(addr);
        let current = bucket.first.get();
        if !current.is_null() {
            /*SAFETY:
             * - sleeping threads can't destroy their ThreadData.
             * - the bucket is locked, so threads can't be unlinked by others.
             * - `current` isn't null
             */
            unsafe {
                // fix tail if needed, goes first to deduce `previous`
                if current == bucket.last.get() {
                    bucket.last.set(ptr::null());
                }
                // remove `current` from the list
                bucket.first.set((*current).next.get());
                // the thread to wake has been unlinked, release the lock
                drop(bucket);

                (*current).parker.unpark();
            }
        }
    }

    pub(crate) fn unpark_all(addr: *const ()) {
        let mut current = {
            let bucket = lock_bucket(addr);
            //This isn't needed, but it allows detecting errors
            bucket.last.set(std::ptr::null());

            bucket.first.replace(std::ptr::null())
        };
        /*SAFETY:
         * - sleeping threads can't destroy their ThreadData.
         * - this list was removed from bucket, so we own it.
         */
        unsafe {
            while !current.is_null() {
                let node = current;
                current = (*current).next.get();
                (*node).parker.unpark();
            }
        }
    }

    pub(crate) fn unpark_some(addr: *const (), mut count: usize) {
        let bucket = lock_bucket(addr);
        let first = bucket.first.get();
        let mut current = first;

        /*SAFETY:
         * - sleeping threads can't destroy their ThreadData.
         * - the bucket is locked, so threads can't be unlinked by others.
         * So, if `*const ThreadData` isn't null, then it's safe to dereference.
         */
        unsafe {
            loop {
                if current.is_null() {
                    bucket.first.set(std::ptr::null());
                    break;
                }

                count -= 1;
                if count == 0 {
                    bucket.first.set((*current).next.replace(std::ptr::null()));
                    break;
                }

                current = (*current).next.get();
            }
        }
        drop(bucket);

        current = first;
        /*SAFETY:
         * - sleeping threads can't destroy their ThreadData.
         * - this list was removed from bucket, so we own it.
         */
        unsafe {
            while !current.is_null() {
                let node = current;
                current = (*current).next.get();
                (*node).parker.unpark();
            }
        }
    }
    struct Bucket {
        first: Cell<*const ThreadData>,
        last: Cell<*const ThreadData>,
    }

    unsafe impl Send for Bucket {}
    struct Parker(AtomicBool, Thread);

    impl Parker {
        fn new() -> Self {
            Self(AtomicBool::new(false), loom::thread::current())
        }

        fn park(&self) {
            for _ in 0..4 {
                if self
                    .0
                    .compare_exchange(true, false, Acquire, Acquire)
                    .is_ok()
                {
                    return;
                }
                loom::thread::park();
            }
        }

        fn unpark(&self) {
            let thread = self.1.clone();
            self.0.store(true, Release);
            thread.unpark();
        }
    }

    #[cfg(test)]
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
}
