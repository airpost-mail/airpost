// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Blocking work from a thread that may have a tokio runtime entered.
//!
//! Plan 4.1 has network threads run a current-thread runtime, and both reqwest's blocking
//! client and a nested `block_on` panic inside one. When a runtime is entered the work runs
//! on a short-lived thread instead; the caller's thread blocks either way, as it would have
//! in the C++.

/// Runs `f` here, or on a scoped thread when this thread is inside a tokio runtime.
pub(crate) fn outside_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    if tokio::runtime::Handle::try_current().is_err() {
        return f();
    }
    std::thread::scope(|s| match s.spawn(f).join() {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_block_on_does_not_panic() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let v = rt.block_on(async {
            outside_runtime(|| {
                let inner = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap();
                inner.block_on(async { 7 })
            })
        });
        assert_eq!(v, 7);
    }
}
