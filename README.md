# [sparking-lot-core][me]

[`s(implified-)parking-lot-core`][me] is a simplified version of [`parking_lot_core`],
the backend of [`parking_lot`]. It doesn't include timeouts and park or unpark
tokens, and doesn't readjust based on thread count, so going above certain thread
counts (96 by default, 384 with the `more-concurrency` feature), will
lead to worse scaling than [`parking_lot_core`]. However, it has static memory usage
and, most importantly, [`sparking-lot-core`][me] has **[`loom 0.7`][`loom`]**
support with `--cfg loom` for concurrency testing.

## Usage

First, add this to your Cargo.toml:

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

[`loom`] is enabled with `--cfg loom`. When running loom tests, it's recommended to enable the `loom-test` feature, as the default test implementation is severely limited. The old behaviour
is described in the [docs](https://docs.rs/sparking-lot-core/0.1.3/sparking_lot_core/).

## License

This project is licensed under the [MIT LICENSE](https://github.com/JuliusEmperorOfRome/sparking-lot-core/blob/master/LICENSE)

[me]: https://crates.io/crates/sparking-lot-core
[`parking_lot_core`]: https://crates.io/crates/parking_lot_core
[`parking_lot`]: https://crates.io/crates/parking_lot
[`loom`]: https://crates.io/crates/loom/0.7.0