use crate::real::loom::{Cell, Mutex, MutexGuard};
use crate::real::park::{Parker, ParkerT};
use core::ptr::{self, addr_of, NonNull};

#[cfg(all(not(loom), not(feature = "more-concurrency")))]
// parking-lot uses a max load factor of 3,
// so 32(1 << 5) buckets is enough for 96 threads. In
// the case that more threads use sparking-lot,
// it will perform worse than parking-lot and
// that's acceptable
const BUCKET_BITS: usize = 5;
#[cfg(all(not(loom), feature = "more-concurrency"))]
// In this case, performs better until 384 threads instead
const BUCKET_BITS: usize = 7;
#[cfg(loom)]
// Reduce load for loom
const BUCKET_BITS: usize = 1;

const BUCKET_COUNT: usize = 1 << BUCKET_BITS;

/* # Note
 *
 * repr(C) is required in `unpark_all` and this is
 * also the most compact way to store these members
 * without some odd fusing, which shouldn't really
 * be possible anyways. Also, it just so happens that
 * `next` is accessed the most, `addr` is second,
 * and `parker` is relatively cold, so this layout
 * is good anyways.
 */
#[repr(C)]
struct ThreadData {
    next: Cell<*const ThreadData>,
    addr: Cell<*const ()>,
    parker: Parker,
}

impl ThreadData {
    #[cfg(not(loom))]
    const fn new() -> Self {
        Self {
            parker: Parker::new(),
            addr: Cell::new(ptr::null()),
            next: Cell::new(ptr::null()),
        }
    }

    #[cfg(loom)]
    fn new() -> Self {
        Self {
            parker: Parker::new(),
            addr: Cell::new(ptr::null()),
            next: Cell::new(ptr::null()),
        }
    }
}

fn lock_bucket(addr: *const ()) -> MutexGuard<'static, Bucket> {
    struct Hashtable {
        buckets: [Mutex<Bucket>; BUCKET_COUNT],
    }

    impl Hashtable {
        #[cfg(not(loom))]
        const fn new() -> Self {
            const INIT: Mutex<Bucket> = Mutex::new(Bucket {
                first: Cell::new(ptr::null()),
                last: Cell::new(ptr::null()),
            });

            Self {
                buckets: [INIT; BUCKET_COUNT],
            }
        }

        #[cfg(loom)]
        fn new() -> Self {
            Self {
                buckets: core::array::from_fn(|_| {
                    Mutex::new(Bucket {
                        first: Cell::new(ptr::null()),
                        last: Cell::new(ptr::null()),
                    })
                }),
            }
        }

        #[inline]
        fn lock_bucket(&self, addr: *const ()) -> MutexGuard<'_, Bucket> {
            let idx = Self::hash(addr as usize);
            //SAFETY: guaranteed by the hash function
            unsafe {
                #[cfg(not(loom))]
                debug_assert!(idx < BUCKET_COUNT);
                #[cfg(loom)]
                assert!(idx < BUCKET_COUNT);
                self.buckets.get_unchecked(idx)
            }
            .lock()
            .unwrap()
        }

        /* loom tests with checkpoints, can't rely on
         * addresses, and this allows users to write
         * `n as *const()` to select buckets, but still
         * kind of works with addresses with disabled
         * loom checkpoints.
         */
        #[cfg(loom)]
        fn hash(n: usize) -> usize {
            n & (BUCKET_COUNT - 1)
        }

        #[cfg(not(loom))]
        fn hash(n: usize) -> usize {
            #[cfg(target_pointer_width = "64")]
            return n.wrapping_mul(0x9E3779B97F4A7C15) >> (64 - BUCKET_BITS);
            #[cfg(target_pointer_width = "32")]
            return n.wrapping_mul(0x9E3779B9) >> (32 - BUCKET_BITS);
            #[cfg(not(any(target_pointer_width = "64", target_pointer_width = "32")))]
            {
                // With random addresses has slightly
                // better bucket coverage than the
                // hashes above, with close-by ones
                // it's a lot worse.
                let mut h = 0;
                for i in 0..BUCKET_BITS {
                    h |= (n >> i) & (1 << i);
                }
                h
            }
        }
    }
    #[cfg(not(loom))]
    static HASHTABLE: Hashtable = Hashtable::new();
    #[cfg(loom)]
    loom::lazy_static! {
        static ref HASHTABLE: Hashtable = Hashtable::new();
    }
    HASHTABLE.lock_bucket(addr)
}

#[inline(always)]
fn with_thread_data<R>(f: impl FnOnce(&ThreadData) -> R) -> R {
    if !Parker::CHEAP_NEW {
        #[cfg(not(loom))]
        thread_local!(static THREAD_DATA: ThreadData = const {ThreadData::new()});
        #[cfg(loom)]
        loom::thread_local!(static THREAD_DATA: ThreadData = ThreadData::new());
        match THREAD_DATA.try_with(|x| x as *const _) {
            Ok(ptr) => unsafe { f(&*ptr) },
            Err(_) => {
                let td = ThreadData::new();
                f(&td)
            }
        }
    } else {
        let td = ThreadData::new();
        f(&td)
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
            //SAFETY: last isn't null if head isn't null
            unsafe {
                #[cfg(not(loom))]
                debug_assert!(!bucket.last.get().is_null());
                #[cfg(loom)]
                assert!(!bucket.last.get().is_null());
                &*bucket.last.get()
            }
            .next
            .set(thread_data);
        }
        bucket.last.set(thread_data);
        // not releasing `bucket` lock before parking would deadlock
        drop(bucket);

        // TODO: remove after implementing `Parker`s which guarantee no panics.
        let on_panic = {
            use core::mem::MaybeUninit;

            struct OnDrop<F: FnOnce()>(MaybeUninit<F>);
            impl<F: FnOnce()> Drop for OnDrop<F> {
                fn drop(&mut self) {
                    // Always initialised
                    unsafe { self.0.assume_init_read()() };
                }
            }
            OnDrop(MaybeUninit::new(|| {
                release(addr, thread_data);
                // Slight modification of `unpark_one`
                #[cold]
                fn release(addr: *const (), thread_data: &ThreadData) {
                    let bucket = lock_bucket(addr);
                    let mut current = bucket.first.get();
                    let mut previous = ptr::null();
                    /*SAFETY:
                     * - sleeping threads can't destroy their ThreadData.
                     * - the bucket is locked, so threads can't be unlinked by others.
                     * So, if `*const ThreadData` isn't null, then it's safe to dereference.
                     */
                    unsafe {
                        while !current.is_null() {
                            let next = (*current).next.get();
                            if ptr::eq(current, thread_data) {
                                // fix tail if needed, goes first to deduce `previous`
                                if current == bucket.last.get() {
                                    bucket.last.set(previous);
                                }
                                // remove `current` from the list
                                if previous.is_null() {
                                    bucket.first.set(next);
                                } else {
                                    (*previous).next.set(next);
                                }

                                return;
                            }
                            previous = current;
                            current = next;
                        }
                    }
                }
            }))
        };

        //SAFETY: `park` only called on this thread.
        unsafe {
            thread_data.parker.park();
        }

        //disengage panic guard
        core::mem::forget(on_panic);
    });
}

pub(crate) fn unpark_one(addr: *const ()) {
    let bucket = lock_bucket(addr);
    let mut current = bucket.first.get();
    let mut previous = ptr::null();
    /*SAFETY:
     * - sleeping threads can't destroy their ThreadData.
     * - the bucket is locked, so threads can't be unlinked by others.
     * So, if `*const ThreadData` isn't null, then it's safe to dereference.
     */
    unsafe {
        while !current.is_null() {
            let next = (*current).next.get();
            if (*current).addr.get() == addr {
                // fix tail if needed, goes first to deduce `previous`
                if current == bucket.last.get() {
                    bucket.last.set(previous);
                }
                // remove `current` from the list
                if previous.is_null() {
                    bucket.first.set(next);
                } else {
                    (*previous).next.set(next);
                }
                // the thread to wake has been unlinked, release the lock
                drop(bucket);

                // since ThreadData lives until the thread is
                // woken and threads sleep before `unpark` is
                // called, `parker` is alive.
                ParkerT::unpark(addr_of!((*current).parker));
                return;
            }
            previous = current;
            current = next;
        }
    }
}

pub(crate) fn unpark_all(addr: *const ()) {
    let bucket = lock_bucket(addr);
    let mut current = bucket.first.get();
    let mut previous = ptr::null();

    let unpark_list = Cell::new(ptr::null::<ThreadData>());
    let mut unpark_list_tail = NonNull::from(&unpark_list);

    /*SAFETY:
     * - sleeping threads can't destroy their ThreadData.
     * - the bucket is locked, so threads can't be unlinked by others.
     * So, if `*const ThreadData` isn't null, then it's safe to dereference.
     */
    unsafe {
        while !current.is_null() {
            let next = (*current).next.get();
            if (*current).addr.get() == addr {
                // fix tail if needed, goes first to deduce `previous`
                if current == bucket.last.get() {
                    bucket.last.set(previous);
                }
                // remove `current` from the list
                if previous.is_null() {
                    bucket.first.set(next);
                } else {
                    (*previous).next.set(next);
                }

                unpark_list_tail.as_ref().set(current);
                unpark_list_tail = NonNull::from(&(*current).next);
            } else {
                previous = current;
            }
            current = next;
        }
    }
    drop(bucket);

    let mut current = unpark_list.get();
    if current.is_null() {
        return;
    }
    loop {
        /*SAFETY:
         * - sleeping threads can't destroy their ThreadData until woken.
         * - this thread is the only awake thread with access to them.
         */
        unsafe {
            let next = (*current).next.get();
            // since ThreadData lives until the thread is
            // woken and threads sleep before `unpark` is
            // called, `parker` is alive.
            ParkerT::unpark(addr_of!((*current).parker));

            // `ThreadData` is repr(C) and `next` is the first element, so
            // (`current` as *const Cell<_>) gives the address of `current->next`.
            if ptr::eq(current as *const Cell<_>, unpark_list_tail.as_ptr()) {
                break;
            }
            // now *current may be destroyed, but it's no longer accessed.
            current = next;
        };
    }
}

pub(crate) fn unpark_some(addr: *const (), mut count: usize) {
    let bucket = lock_bucket(addr);
    let mut current = bucket.first.get();
    let mut previous = ptr::null();

    let unpark_list = Cell::new(ptr::null::<ThreadData>());
    let mut unpark_list_tail = NonNull::from(&unpark_list);

    /*SAFETY:
     * - sleeping threads can't destroy their ThreadData.
     * - the bucket is locked, so threads can't be unlinked by others.
     * So, if `*const ThreadData` isn't null, then it's safe to dereference.
     */
    unsafe {
        while !current.is_null() {
            let next = (*current).next.get();
            if (*current).addr.get() == addr {
                // fix tail if needed, goes first to deduce `previous`
                if current == bucket.last.get() {
                    bucket.last.set(previous);
                }
                // remove `current` from the list
                if previous.is_null() {
                    bucket.first.set(next);
                } else {
                    (*previous).next.set(next);
                }

                unpark_list_tail.as_ref().set(current);
                unpark_list_tail = NonNull::from(&(*current).next);

                count -= 1;
                if count == 0 {
                    break;
                }
            } else {
                previous = current;
            }
            current = next;
        }
    }
    drop(bucket);

    let mut current = unpark_list.get();
    if current.is_null() {
        return;
    }
    loop {
        /*SAFETY:
         * - sleeping threads can't destroy their ThreadData until woken.
         * - this thread is the only awake thread with access to them.
         */
        unsafe {
            let next = (*current).next.get();
            // since ThreadData lives until the thread is
            // woken and threads sleep before `unpark` is
            // called, `parker` is alive.
            ParkerT::unpark(addr_of!((*current).parker));

            // `ThreadData` is repr(C) and `next` is the first element, so
            // (`current` as *const Cell<_>) gives the address of `current->next`.
            if ptr::eq(current as *const Cell<_>, unpark_list_tail.as_ptr()) {
                break;
            }
            // now *current may be destroyed, but it's no longer accessed.
            current = next;
        };
    }
}

// Alignment values taken from crossbeam(https://crates.io/crates/crossbeam/0.8.2)

// Starting from Intel's Sandy Bridge, spatial prefetcher is now pulling pairs of 64-byte cache
// lines at a time, so we have to align to 128 bytes rather than 64.
//
// Sources:
// - https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-optimization-manual.pdf
// - https://github.com/facebook/folly/blob/1b5288e6eea6df074758f877c849b6e73bbb9fbb/folly/lang/Align.h#L107
//
// ARM's big.LITTLE architecture has asymmetric cores and "big" cores have 128-byte cache line size.
//
// Sources:
// - https://www.mono-project.com/news/2016/09/12/arm64-icache/
//
// powerpc64 has 128-byte cache line size.
//
// Sources:
// - https://github.com/golang/go/blob/3dd58676054223962cd915bb0934d1f9f489d4d2/src/internal/cpu/cpu_ppc64x.go#L9
// - https://github.com/torvalds/linux/blob/3516bd729358a2a9b090c1905bd2a3fa926e24c6/arch/powerpc/include/asm/cache.h#L26
#[cfg_attr(
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64",
    ),
    repr(align(128))
)]
// arm, mips, mips64, sparc, and hexagon have 32-byte cache line size.
//
// Sources:
// - https://github.com/golang/go/blob/3dd58676054223962cd915bb0934d1f9f489d4d2/src/internal/cpu/cpu_arm.go#L7
// - https://github.com/golang/go/blob/3dd58676054223962cd915bb0934d1f9f489d4d2/src/internal/cpu/cpu_mips.go#L7
// - https://github.com/golang/go/blob/3dd58676054223962cd915bb0934d1f9f489d4d2/src/internal/cpu/cpu_mipsle.go#L7
// - https://github.com/golang/go/blob/3dd58676054223962cd915bb0934d1f9f489d4d2/src/internal/cpu/cpu_mips64x.go#L9
// - https://github.com/torvalds/linux/blob/3516bd729358a2a9b090c1905bd2a3fa926e24c6/arch/sparc/include/asm/cache.h#L17
// - https://github.com/torvalds/linux/blob/3516bd729358a2a9b090c1905bd2a3fa926e24c6/arch/hexagon/include/asm/cache.h#L12
#[cfg_attr(
    any(
        target_arch = "arm",
        target_arch = "mips",
        target_arch = "mips32r6",
        target_arch = "mips64",
        target_arch = "mips64r6",
        target_arch = "sparc",
        target_arch = "hexagon",
    ),
    repr(align(32))
)]
// m68k has 16-byte cache line size.
//
// Sources:
// - https://github.com/torvalds/linux/blob/3516bd729358a2a9b090c1905bd2a3fa926e24c6/arch/m68k/include/asm/cache.h#L9
#[cfg_attr(target_arch = "m68k", repr(align(16)))]
// s390x has 256-byte cache line size.
//
// Sources:
// - https://github.com/golang/go/blob/3dd58676054223962cd915bb0934d1f9f489d4d2/src/internal/cpu/cpu_s390x.go#L7
// - https://github.com/torvalds/linux/blob/3516bd729358a2a9b090c1905bd2a3fa926e24c6/arch/s390/include/asm/cache.h#L13
#[cfg_attr(target_arch = "s390x", repr(align(256)))]
// x86, wasm, riscv, and sparc64 have 64-byte cache line size.
//
// Sources:
// - https://github.com/golang/go/blob/dda2991c2ea0c5914714469c4defc2562a907230/src/internal/cpu/cpu_x86.go#L9
// - https://github.com/golang/go/blob/3dd58676054223962cd915bb0934d1f9f489d4d2/src/internal/cpu/cpu_wasm.go#L7
// - https://github.com/torvalds/linux/blob/3516bd729358a2a9b090c1905bd2a3fa926e24c6/arch/riscv/include/asm/cache.h#L10
// - https://github.com/torvalds/linux/blob/3516bd729358a2a9b090c1905bd2a3fa926e24c6/arch/sparc/include/asm/cache.h#L19
//
// All others are assumed to have 64-byte cache line size.
#[cfg_attr(
    not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64",
        target_arch = "arm",
        target_arch = "mips",
        target_arch = "mips32r6",
        target_arch = "mips64",
        target_arch = "mips64r6",
        target_arch = "sparc",
        target_arch = "hexagon",
        target_arch = "m68k",
        target_arch = "s390x",
    )),
    repr(align(64))
)]
struct Bucket {
    first: Cell<*const ThreadData>,
    last: Cell<*const ThreadData>,
}

unsafe impl Send for Bucket {}
