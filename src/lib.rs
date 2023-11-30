#![deny(missing_docs)]
//! This library provides a low-level API for parking
//! on addresses.
//!
//! # The parking lot
//!
//! To keep synchronisation primitives small, most of the parking/unparking
//! can be off-loaded to the parking lot. This allows writing locks that may
//! even use a single bit. The idea comes from Webkit [`WTF::ParkingLot`],
//! which in turn was inspired by Linux [`futexes`]. The API provided by this
//! crate is significantly simpler &mdash; no park/unpark tokens or timeouts
//! are provided and it also doesn't readjust based on thread count, which
//! means with large enough thread counts the contention may be worse than
//! when using other crates.
//!
//! The parking lot provides two operations:
//!
//! - **Parking** &mdash; pausing a thread and enqueing it in a queue keyed
//! by an address. This can be done with [`park`].
//! - **Unparking** &mdash; unpausing a thread that was queued on an address.
//! This can be done with [`unpark_one`], [`unpark_some`] and [`unpark_all`].
//!
//! For more information read the function docs.
//!
//! # [`loom`]
//! This crate has [`loom 0.7`][`loom`] integrated, which can be enabled with
//! `--cfg loom` and optionally the [`loom-test`](#features) feature. Using the
//! feature is recommended, but if it's not present, legacy [`loom`] testing will
//! be enabled.
//!
//! > ## Legacy [`loom`]
//! >
//! > [`loom`] requires consistency in it's executions, but program addresses are
//! > intentionally random on most platforms. As such, when using legacy [`loom`],
//! > there are things to keep in mind. When [`parking`](park) on different addresses, there
//! > are two possible outcomes: they may map to the same bucket, which may provide extra
//! > synchronisation, or different ones, which doesn't. This additional synchronisation
//! > shouldn't be relied on &mdash; the only way to guarantee the same bucket when not
//! > running [`loom`] is to use the same address with [`park`]. To give users control over
//! > this, when running legacy [`loom`], there are 2 buckets: one for even addresses, one
//! > for odd addresses. In loom tests you should at least include the case with different
//! > buckets, since a shared bucket will provide more synchronisation and it shouldn't really
//! > be possible that looser synchronisation will exclude the states possible with stricter
//! > ones. One approach is to use a base address, and a second parking address can be made
//! > with a [`cast`][cast] to [`u8`][u8] and then [`offsetting`][offset] by 1. For example,
//! > when implementing a SPSC channel, the sender could park on *`<address of inner state>`*
//! > and the receiver on <code style="white-space: nowrap;">
//! > <i>\<address of inner state></i>.[cast]::<[u8]>().[offset]`(1)`
//! > </code> to park on different buckets. A nice property of this approach is that it also
//! > works in non-loom contexts where normally you would park on two non-ZST members.
//! >
//! > ### Limitations
//! >
//! > The legacy [`loom`] integration technique has some major drawbacks:
//! >
//! > - No more than 2 distinct addresses can be used if you want to properly test the case of
//! > non-colliding buckets.
//! > - Requires some extra work to use [`loom`].
//! > - Dependents of dependents of [`sparking-lot-core`](crate) can't really use loom tests, because
//! > it can easily become impossible to test the case of non-colliding buckets.
//!
//! # Features
//!
//! - `more-concurrency` - increases the number of buckets, which reduces contention,
//! but requires more memory. This flag is unlikely to produce meaningful results if
//! thread count is below 100, but it also isn't all that expensive &mdash; in the
//! worst case it uses 24 extra KiB of RAM (adds ~12 KiB for x86-64).
//! - `loom-test` - enables better [`loom`] tests. Has no effect without `--cfg loom`.
//! - `thread-parker` - changes the parking implementation from a [`std::sync::Mutex`]
//! to a [`std::thread::park`] based one. It may or may not perform better.
//!
//! [`WTF::ParkingLot`]: https://webkit.org/blog/6161/locking-in-webkit/
//! [`futexes`]: http://man7.org/linux/man-pages/man2/futex.2.html
//! [`loom`]: https://crates.io/crates/loom/0.7.0
//! [`byte_offset`]: https://doc.rust-lang.org/stable/core/primitive.pointer.html#method.byte_offset
//! [u8]: https://doc.rust-lang.org/stable/core/primitive.u8.html
//! [cast]: https://doc.rust-lang.org/stable/core/primitive.pointer.html#method.cast
//! [offset]: https://doc.rust-lang.org/stable/core/primitive.pointer.html#method.offset

#[cfg(not(all(loom, feature = "loom-test")))]
mod real;
#[cfg(not(all(loom, feature = "loom-test")))]
use real::parking_lot;

#[cfg(all(loom, feature = "loom-test"))]
mod fake;
#[cfg(all(loom, feature = "loom-test"))]
use fake::parking_lot;

/// Parks the current thread on `addr` until notified,
/// but only if `expected` returns true.
///
/// There are no spurious wake-ups (unlike [`std::thread::park`]).
///
/// # Safety
/// - `expected` can't call any functions from this [`crate`],
/// as this may cause deadlocks or panics.
/// - Using addresses that you don't own is highly discouraged.
/// This is because if multiple libraries/modules/anything [`park`]
/// on the same address without knowledge of each other, it
/// will cause something that from their perspective looks like
/// spurious wake-ups, which is likely to break code, since [`park`]
/// guarantees that no spurious wake-ups will happen.
///
/// # Notes
///
/// - The memory pointed to by `addr` isn't written to,
/// it isn't read and no references to it are formed.
/// - `expected` is called under a lock, which could block
/// other [`park`], [`unpark_one`], [`unpark_some`] or
/// [`unpark_all`] calls (even with different `addr`). As such,
/// `expected` should return quickly.
/// - This function ensures that if another thread does
/// something that would cause `expected` to return false
/// and only then calls [`unpark_one`], [`unpark_some`] or
/// [`unpark_all`], [`park`] will either be woken
/// up or will not sleep.
///
/// [`park`]: crate::park()
///
/// # Example
///
/// ```rust,no_run
/// use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
/// static WAKE_UP: AtomicBool = AtomicBool::new(false);
///
/// fn wait_for_event() {
///     //SAFETY: remember not to park on WAKE_UP in unrelated functions.
///     unsafe {
///         sparking_lot_core::park(&WAKE_UP as *const _ as *const _, || {
///             WAKE_UP.load(Relaxed) == false
///         })
///     }
/// }
///
/// fn notify_event_happened() {
///     //If these lines are reordered park may miss this notification
///     WAKE_UP.store(true, Relaxed);
///     sparking_lot_core::unpark_one(&WAKE_UP as *const _ as *const _)
/// }
/// ```
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub unsafe fn park(addr: *const (), expected: impl FnOnce() -> bool) {
    parking_lot::park(addr, expected)
}

/// Wakes one thread [`parked`](park()) on `addr`.
///
/// Should be called after making the `expected` of
/// the corresponding [`park`] return false.
///
/// # Notes
///
/// - The memory pointed to by `addr` isn't written to,
/// it isn't read and no references to it are formed.
/// - If no thread is waiting on `addr`, no thread is
/// woken, but it still requires locking, so it's not
/// recommended to call it without reason.
/// - This function ensures that if it is called after an
/// effect, that would cause the `expected` of a call to
/// [`park`] with the same `addr`, [`park`] will either
/// be woken, or it will not have gone to sleep and
/// will return.
///
/// [`park`]: crate::park()
///
/// # Example
///
/// ```
/// use core::sync::atomic::AtomicBool;
/// use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};
///
/// use sparking_lot_core::{park, unpark_one};
///
/// struct BadMutex(AtomicBool);
///
/// impl BadMutex {
///     fn new() -> Self {
///         Self(AtomicBool::new(false))
///     }
///
///     fn lock(&self) {
///         loop {
///             if self.0.compare_exchange(false, true, Acquire, Relaxed).is_ok() {
///                 return
///             }
///             /* SAFETY:
///              * - no calls to sparking_lot_core funtions in closure
///              * - owned address
///              */
///             unsafe {
///                 park(self as *const _ as *const _, || self.0.load(Acquire));
///             }
///         }
///     }
///
///     fn unlock(&self) {
///         self.0.store(false, Release);
///         unpark_one(self as *const _ as *const _);
///     }
/// }
///
/// ```
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub fn unpark_one(addr: *const ()) {
    parking_lot::unpark_one(addr);
}

/// Wakes at most `count` threads [`parked`](park()) on `addr`.
///
/// Should be called after making the `expected` of
/// the corresponding [`parks`](park()) return false.
///
/// # Notes
///
/// - The memory pointed to by `addr` isn't written to,
/// it isn't read and no references to it are formed.
/// - If no thread is waiting on `addr`, no thread is
/// woken, but it still requires locking, so it's not
/// recommended to call it without reason.
/// - This function ensures that if it is called after an
/// effect, that would cause the `expected` of a call to
/// [`park`] with the same `addr`, [`park`] will either
/// be woken, or it will not have gone to sleep and
/// will return.
///
/// [`park`]: crate::park()
///
/// # Example
///
/// ```
/// # struct TaskQueue;
/// # struct Task {};
/// # impl TaskQueue {
/// #     const fn new() -> Self { Self }
/// #     fn push_task(&self, _: Task) {}
/// #     fn pop_task(&self) -> Option<Task> { None }
/// # }
/// use sparking_lot_core::{park, unpark_some};
///
/// static tasks: TaskQueue = TaskQueue::new();
///
/// fn add_tasks<T: Iterator<Item = Task>>(new_tasks: T) {
///     let mut count = 0;
///     for t in new_tasks {
///         tasks.push_task(t);
///         count += 1;
///     }
///     unpark_some(&tasks as *const _ as *const _, count);
/// }
///
/// fn get_task() -> Task {
///     let mut task = None;
///     loop {
///         // some other unblocked thread might
///         // have taken our task, so we loop
///         task = tasks.pop_task();
///         if let Some(task) = task {
///             return task;
///         }
///         unsafe {
///             /* SAFETY:
///              * - no calls to sparking_lot_core funtions in closure
///              * - the task queue **has to be** be private
///              */
///             park(&tasks as *const _ as *const _, || {
///                 task = tasks.pop_task();
///                 task.is_none()
///             });
///         }
///     }
/// }
/// ```
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub fn unpark_some(addr: *const (), count: usize) {
    parking_lot::unpark_some(addr, count);
}

/// Wakes all threads [`parked`](park()) on `addr`.
///
/// Should be called after making the `expected` of
/// the corresponding [`parks`](park()) return false.
///
/// # Notes
///
/// - The memory pointed to by `addr` isn't written to,
/// it isn't read and no references to it are formed.
/// - If no thread is waiting on `addr`, no thread is
/// woken, but it still requires locking, so it's not
/// recommended to call it without reason.
/// - This function ensures that if it is called after an
/// effect, that would cause the `expected` of a call to
/// [`park`] with the same `addr`, [`park`] will either
/// be woken, or it will not have gone to sleep and
/// will return.
///
/// [`park`]: crate::park()
///
/// # Example
///
/// ```
/// use core::sync::atomic::AtomicUsize;
/// use core::sync::atomic::Ordering::{AcqRel, Acquire};
///
/// use sparking_lot_core::{park, unpark_all};
///
/// struct Latch(AtomicUsize);
///
/// impl Latch {
///     fn new(num_threads: usize) -> Self {
///         Self(AtomicUsize::new(num_threads))
///     }
///
///     fn wait(&self) {
///         if self.0.fetch_sub(1, AcqRel) == 1 {
///             unpark_all(self as *const _ as *const _);
///         }
///         else {
///             /* SAFETY:
///              * - no calls to sparking_lot_core funtions in closure
///              * - owned address
///              */
///             unsafe {
///                 park(self as *const _ as *const _, || self.0.load(Acquire) != 0);   
///             }
///         }
///     }
/// }
/// ```
#[cfg_attr(not(loom), inline(always))]
#[cfg_attr(loom, track_caller)]
pub fn unpark_all(addr: *const ()) {
    parking_lot::unpark_all(addr);
}
