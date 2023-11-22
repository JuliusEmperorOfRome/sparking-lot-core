#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod loom;
mod park;
mod parking_lot;

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
/// - The memory pointed to by `addr` isn't writter to,
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
/// - The memory pointed to by `addr` isn't writter to,
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
/// - The memory pointed to by `addr` isn't writter to,
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
/// - The memory pointed to by `addr` isn't writter to,
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
