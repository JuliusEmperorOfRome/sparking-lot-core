cfg_if::cfg_if! {
    if #[cfg(loom)] {
        use loom as stdlib;
    }
    else {
        use std as stdlib;
    }
}

use stdlib::cell::Cell;
use stdlib::sync::{Mutex, MutexGuard};

use core::ptr::{self, addr_of, NonNull};

mod parker;
use parker::Parker;

pub(crate) fn park(addr: usize, expected: impl FnOnce() -> bool) -> Option<u32> {
    let bucket = lock_bucket(addr);
    if !expected() {
        return None;
    }

    let thread_data = &ThreadData {
        next: Cell::new(ptr::null()),
        addr: addr,
        parker: Parker::new(),
    };

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

    //SAFETY: `park` only called on this thread.
    Some(unsafe { thread_data.parker.park() })
}

pub(crate) fn unpark_one(addr: usize, token: u32) {
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
            if (*current).addr == addr {
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
                Parker::unpark(addr_of!((*current).parker), token);
                return;
            }
            previous = current;
            current = next;
        }
    }
}

pub(crate) fn unpark_all(addr: usize, token: u32) {
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
            if (*current).addr == addr {
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
            Parker::unpark(addr_of!((*current).parker), token);

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

pub(crate) fn unpark_some(addr: usize, mut count: usize, token: u32) {
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
            if (*current).addr == addr {
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
            Parker::unpark(addr_of!((*current).parker), token);

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

struct Bucket {
    first: Cell<*const ThreadData>,
    last: Cell<*const ThreadData>,
}

unsafe impl Send for Bucket {}

#[repr(C)]
struct ThreadData {
    next: Cell<*const ThreadData>,
    addr: usize,
    parker: Parker,
}

fn lock_bucket(addr: usize) -> MutexGuard<'static, Bucket> {
    use cfg_if::cfg_if;
    cfg_if! {

    if #[cfg(not(loom))] {
        cfg_if! {

        if #[cfg(not(feature = "more-concurrency"))] {
            // parking-lot uses a max load factor of 3,
            // so 32(1 << 5) buckets is enough for 96 threads. In
            // the case that more threads use sparking-lot,
            // it will perform worse than parking-lot and
            // that's acceptable
            const BUCKET_BITS: usize = 5;
        }
        else {
            // In this case, performs better until 384 threads instead
            const BUCKET_BITS: usize = 7;
        }

        }
    }
    else {
        cfg_if! {

        if #[cfg(feature = "legacy-loom")] {
            const BUCKET_BITS: usize = 1;
        }
        else {
            // allows up to 32 different addresses
            const BUCKET_BITS: usize = 5;
        }

        }
    }

    }

    const BUCKET_COUNT: usize = 1 << BUCKET_BITS;

    struct Hashtable {
        buckets: [Slot; BUCKET_COUNT],
    }

    impl Hashtable {
        #[cfg(not(loom))]
        const fn new() -> Self {
            const INIT: Slot = Slot(Mutex::new(Bucket {
                first: Cell::new(ptr::null()),
                last: Cell::new(ptr::null()),
            }));
            Self {
                buckets: [INIT; BUCKET_COUNT],
            }
        }

        #[cfg(loom)]
        fn new() -> Self {
            Self {
                buckets: core::array::from_fn(|_| {
                    Slot(Mutex::new(Bucket {
                        first: Cell::new(ptr::null()),
                        last: Cell::new(ptr::null()),
                    }))
                }),
            }
        }

        fn lock_bucket(&self, addr: usize) -> MutexGuard<'_, Bucket> {
            //SAFETY: `hash` returns values in [0, BUCKET_COUNT)
            unsafe {
                debug_assert!(Self::hash(addr) < BUCKET_COUNT);
                self.buckets.get_unchecked(Self::hash(addr))
            }
            .0
            .lock()
            .expect("`expexcted` paniced in a previous `park` call")
        }

        // Legacy loom tests use an odd and even address buckets
        #[cfg(all(loom, feature = "legacy-loom"))]
        fn hash(n: usize) -> usize {
            n & (BUCKET_COUNT - 1)
        }

        #[cfg(all(loom, not(feature = "legacy-loom")))]
        fn hash(n: usize) -> usize {
            use std::cell::RefCell;
            struct AddrMap {
                addrs: [usize; BUCKET_COUNT],
                count: usize,
            }

            impl AddrMap {
                fn new() -> Self {
                    Self {
                        addrs: [0; BUCKET_COUNT],
                        count: 0,
                    }
                }

                fn to_index(&mut self, addr: usize) -> usize {
                    let end = self.count;
                    for (i, a) in self.addrs[0..end].iter().enumerate() {
                        if *a == addr {
                            return i;
                        }
                    }
                    assert_ne!(
                        end, BUCKET_COUNT,
                        "[sparking-lot-core] can't use more than {BUCKET_COUNT} different addresses in loom tests"
                    );
                    let last = end;
                    self.count += 1;

                    self.addrs[last] = addr;
                    return last;
                }
            }

            loom::lazy_static!(static ref MAP: RefCell<AddrMap> = RefCell::new(AddrMap::new()););
            MAP.borrow_mut().to_index(n)
        }

        #[cfg(not(loom))]
        fn hash(n: usize) -> usize {
            #[cfg(target_pointer_width = "64")]
            return n.wrapping_mul(0x9E3779B97F4A7C15) >> (64 - BUCKET_BITS);
            #[cfg(target_pointer_width = "32")]
            return n.wrapping_mul(0x9E3779B9) >> (32 - BUCKET_BITS);
            #[cfg(not(any(target_pointer_width = "64", target_pointer_width = "32")))]
            (0..BUCKET_BITS).fold(0, |h, i| h | (n >> i) & (1 << i))
        }
    }
    #[cfg(not(loom))]
    static TABLE: Hashtable = Hashtable::new();
    #[cfg(loom)]
    loom::lazy_static!(static ref TABLE: Hashtable = Hashtable::new(););
    return TABLE.lock_bucket(addr as usize);

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
    struct Slot(Mutex<Bucket>);
}
