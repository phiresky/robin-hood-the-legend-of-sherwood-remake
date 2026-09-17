//! Opt-in admission budget for derived sprite documents. No wire-format changes.
use crate::coder::Result;

#[cfg(feature = "std")]
std::thread_local! {
    static REMAINING: core::cell::Cell<Option<usize>> = const { core::cell::Cell::new(None) };
}

/// Charge before allocating either decoder scratch or decoded collection storage.
/// Twice the requested storage plus allocation overhead conservatively covers
/// geometric growth. Charges are cumulative, even after a scratch buffer dies.
#[inline]
pub(crate) fn charge(count: usize, element_bytes: usize) -> Result<()> {
    #[cfg(feature = "std")]
    return REMAINING.with(|remaining| {
        let Some(budget) = remaining.get() else {
            return Ok(());
        };
        let bytes = count
            .checked_mul(element_bytes.max(1))
            .and_then(|bytes| bytes.checked_mul(2))
            .and_then(|bytes| bytes.checked_add(64));
        let next = bytes
            .and_then(|bytes| budget.checked_sub(bytes))
            .ok_or_else(|| crate::error::error("decode allocation budget exceeded"))?;
        remaining.set(Some(next));
        Ok(())
    });
    #[cfg(not(feature = "std"))]
    {
        let _ = (count, element_bytes);
        Ok(())
    }
}

/// Decode with a cumulative storage budget for the built-in derive decoders.
/// Checks run during population, before scratch expansion and before collection
/// construction. This is a conservative storage/work bound, not a process RSS
/// limit. Custom `Decode` implementations must charge their own allocations;
/// the Serde adapter and `Buffer::decode` are not entry points for this API.
/// Existing `decode` calls outside this scope retain their previous behavior.
#[cfg(feature = "std")]
pub fn decode_with_limit<'a, T: crate::Decode<'a>>(bytes: &'a [u8], limit: usize) -> Result<T> {
    // Thread-local runtime guard; never part of the serialized document.
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REMAINING.with(|remaining| remaining.set(self.0));
        }
    }
    REMAINING.with(|remaining| {
        // Reject nesting rather than allowing a second call to reset the budget.
        if remaining.get().is_some() {
            return crate::error::err("nested bounded decode");
        }
        let _restore = Restore(remaining.replace(Some(limit)));
        charge(1, core::mem::size_of::<T>())?;
        crate::decode(bytes)
    })
}
