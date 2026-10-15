//! The few things that differ by OS (plan section 11). Nothing outside this module uses
//! Unix-only APIs.

/// `test-segfault`: crash the way a real segfault would, for the client's crash handling.
pub fn raise_segfault() -> ! {
    #[cfg(unix)]
    // SAFETY: raise only delivers a signal to this process; SIGSEGV's default action
    // terminates it.
    unsafe {
        libc::raise(libc::SIGSEGV);
    }
    std::process::abort()
}

/// Exits without running destructors or flushing deltas, as `std::exit(141)` did.
pub fn exit_now(code: i32) -> ! {
    ms_core::log::flush();
    std::process::exit(code)
}
