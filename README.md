# [sparking-lot-core][me]

[`s(implified-)parking-lot-core`][me] is a simplified version of [`parking_lot_core`],
the backend of [`parking_lot`]. It doesn't include timeouts and park or unpark
tokens, and doesn't readjust based on thread count, so going above certain thread
counts (96 by default, 384 with the [`more-concurrency`](#features) feature), will
lead to worse than [`parking_lot_core`]. However, it has static memory usage and,
most importantly, [`sparking-lot-core`][me] has **[`loom 0.7`][`loom`]** support
with `--cfg loom` for concurrency testing.

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
random on most platforms. As such, when using [`loom`], there are things to keep in mind.
When `parking` on different addresses, there are two possible outcomes: they may map to
the same bucket, providing more synchronisation, or different ones. This additional
synchronisation shouldn't be relied on &mdash; the only way to guarantee the same bucket
when not running [`loom`] is to use the same address with `park`. To give users control
over this, when running [`loom`], there are 2 buckets: one for even addresses, one for odd
addresses. In loom tests you should at least include the case with different buckets, since
a shared bucket will provide more synchronisation and it shouldn't be really possible that 
looser synchronisation will exclude the states possible with stricter ones. One approach is
to use one base address, [`cast`][cast] to [`u8`][u8] and then
[`offset`][offset] by 1. For example, when implementing a SPSC channel, the sender
could park on *`<address of inner state>`* and the receiver on
<code style="white-space: nowrap;"><i>\<address of inner state></i>.[cast]::<[u8]>().[offset]`(1)`</code> to park on different
buckets. A nice property of this approach is that it also works in non-loom contexts where
normally you would park on two non-ZST members. The current integration of [`loom`] has some
big flaws:
- No more than 2 distinct addresses can be used if you want to properly test the case of
non-colliding buckets.
- Requires some extra work to use [`loom`].
- Dependents of dependents of [`sparking-lot-core`][me] can't really use loom tests, because
it can easily become impossible to test the case of non-colliding buckets.

However, changing this behsviour would be a breaking change, so it will stay this way for probably
a long time.

## Features

- `more-concurrency` - increases the number of buckets, which reduces contention, but requires
more memory. This flag is unlikely to produce meaningful results if thread count is below 100,
but it also isn't all that expensive &mdash; in the worst case it uses 24 extra KiB of RAM
(adds ~12 KiB for x86-64).

## License

This project is licensed under the [MIT LICENSE](https://github.com/JuliusEmperorOfRome/sparking-lot-core/blob/master/LICENSE)

[me]: https://crates.io/crates/sparking-lot-core
[`parking_lot_core`]: https://crates.io/crates/parking_lot_core
[`parking_lot`]: https://crates.io/crates/parking_lot
[`loom`]: https://crates.io/crates/loom/0.7.0
[`byte_offset`]: https://doc.rust-lang.org/stable/core/primitive.pointer.html#method.byte_offset
[u8]: https://doc.rust-lang.org/stable/core/primitive.u8.html
[cast]: https://doc.rust-lang.org/stable/core/primitive.pointer.html#method.cast
[offset]: https://doc.rust-lang.org/stable/core/primitive.pointer.html#method.offset