//! debug stub — Amiga `kprintf` no-op.
//!
//! Some legacy builds exposed an Amiga OS-style debug hook whose only
//! implemented function was a variadic `kprintf` that did nothing. We
//! preserve the function as a no-op.

/// No-op debug print, matching the original Amiga `kprintf` stub.
pub fn kprintf(_fmt: &str) {
    // intentionally empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kprintf_does_not_panic() {
        kprintf("hello %d");
    }
}
