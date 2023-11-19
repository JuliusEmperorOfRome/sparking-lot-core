macro_rules! spread_attr {
    (
        #[$cfg:meta]
        $($i:item)*
    ) => {
        $(
            #[$cfg]
            $i
        )*
    };
}

spread_attr! {
#[cfg(not(loom))]

pub(crate) use core::sync::atomic::AtomicUsize;
pub(crate) use std::sync::{Condvar, Mutex, MutexGuard};
pub(crate) use core::cell::Cell;
}

spread_attr! {
#[cfg(loom)]

pub(crate) use loom::sync::atomic::AtomicUsize;
pub(crate) use loom::sync::{Condvar, Mutex, MutexGuard};
pub(crate) use loom::cell::Cell;
}
