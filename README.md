# sparking-lot-core

`s(implified-)parking-lot-core` is a simplified version of [`parking_lot_core`],
the backend of [`parking_lot`]. It doesn't include timeouts and park or unpark
tokens, and doesn't readjust based on thread count, so going above certain thread
counts (96 by default, 384 with the `more-concurrency` feature) it scales worse
than [`parking_lot_core`]. However, it has static memory usage and when platform
specific parkers are implemented will most likely be faster than [`parking_lot_core`].
Most importantly, `sparking-lot-core` has **[`loom 0.5`]** support with `--cfg loom`
for concurrency testing.

## Usage

First, add this to Cargo.toml:

```toml
[dependencies]
sparking-lot-core = "0.1"
```

Then use it:

```rust,no_run
use sparking_lot_core::{park, unpark_one};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread;

fn main() {
    static wake: AtomicBool = AtomicBool::new(false);
    let t = thread::spawn(|| {
        park(&wake as *const _ as *const _, |ptr| {
            /* park only uses `addr` like `addr as usize`.
             * It doesn't dereference, read or write to the
             * underlying memory. You can actually pass it
             * dangling pointers for all `park` cares. This
             * is also true for `unpark_one` and `unpark_all`.
             * 
             * This means that if you pass in *mut T you
             * can safely cast `ptr` back to *mut T, if
             * you pass in &T, you can safely dereference
             * `ptr` to &T and so on.
             */
            unsafe {&*ptr}.load(Relaxed) == false
            // This means if `wake` == false, park this thread.
        })
    });
    /* Since Relaxed stores/loads are used, this wouldn't guarantee
     * the ordering of loads/stores to not `wake`.
     * 
     * But this is guaranteed to exit.
     */
    wake.store(true, Relaxed);
    unpark_one(&wake as *const _ as *const _);
    t.join().unwrap();
}
```

## [`loom`]

[`loom`] requires consistency in it's executions, but program addresses are intentionally
random on most platforms. As such, when using [`loom`] you may want to pass [`usize`](https://doc.rust-lang.org/std/primitive.usize.html)
constants instead of addresses. `sparking-lot-core` has two types of parking: different
addresses may or may not map to the same bucket. When running [`loom`], there are 2 buckets:
one for even addresses, one for odd addresses. In loom tests you should at least include the
case with different buckets, since a shared bucket can introduce synchronisation that will
not be present when using different buckets (the only way to guarantee the same bucket when
not running loom is to use the same address with `park`);

[`parking_lot_core`]: https://crates.io/crates/parking_lot_core
[`parking_lot`]: https://crates.io/crates/parking_lot
[`loom 0.5`]: https://crates.io/crates/loom/0.5.6
[`loom`]: https://crates.io/crates/loom/0.5.6