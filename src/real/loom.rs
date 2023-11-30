use cfg_if::cfg_if;

cfg_if! {

if #[cfg(loom)] {
    pub(crate) use loom::cell::Cell;
    pub(crate) use loom::sync::{Mutex, MutexGuard};

    cfg_if! {

        if #[cfg(feature = "thread-parker")] {
            pub(crate) use loom::thread;
            pub(crate) use loom::sync::atomic::{AtomicPtr, AtomicBool};
        }
        else { // default to the old impl
            pub(crate) use loom::sync::Condvar;
        }

    }
}
else {
    pub(crate) use std::cell::Cell;
    pub(crate) use std::sync::{Mutex, MutexGuard};

    cfg_if! {

        if #[cfg(feature = "thread-parker")] {
            pub(crate) use std::thread;
            pub(crate) use std::sync::atomic::{AtomicPtr, AtomicBool};
        }
        else { // default to the old impl
            pub(crate) use std::sync::Condvar;
        }

    }
}

}
