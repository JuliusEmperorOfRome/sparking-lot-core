# sparking-lot-core

`s(implified-)parking-lot-core` is a simplified version of [`parking_lot_core`],
the backend of [`parking_lot`]. It doesn't include timeouts and park or unpark
tokens, and doesn't readjust based on thread count, so going above certain thread
counts (96 by default, 384 with the `more-concurrency` feature) it scales worse
than [`parking_lot_core`]. However, it has static memory usage and, most importantly, `sparking-lot-core` has **[`loom 0.7`]** support with `--cfg loom` for concurrency
testing.

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
    static WAKE_UP: AtomicBool = AtomicBool::new(false);
    let t = thread::spawn(|| {
        unsafe{
            park(&WAKE_UP as *const _ as *const _, || {
                !WAKE_UP.load(Relaxed)
            });
        };
    });
    /* Since Relaxed stores/loads are used, this wouldn't guarantee
     * the ordering of loads/stores to other variables.
     * 
     * But this is guaranteed to exit.
     */
    WAKE_UP.store(true, Relaxed);
    unpark_one(&WAKE_UP as *const _ as *const _);
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

## Features

- `more-concurrency` - increases the number of buckets, which reduces contention, but requires
more memory. This flag is unlikely to produce meaningful results if thread count is below 100,
but it also isn't all that expensive &mdash; in the worst case it uses 24 extra KiB of RAM
(adds ~12 KiB for x86-64).

[`parking_lot_core`]: https://crates.io/crates/parking_lot_core
[`parking_lot`]: https://crates.io/crates/parking_lot
[`loom 0.7`]: https://crates.io/crates/loom/0.7.0
[`loom`]: https://crates.io/crates/loom/0.7.0