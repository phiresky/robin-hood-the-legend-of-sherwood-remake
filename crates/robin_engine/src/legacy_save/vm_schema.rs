//! Schema checks shared by each independently staged VM heap adopter.

use super::payload_vm::{LegacyVmMemberKind, LegacyVmMemberSchema};
use crate::scb::{MemberVariable, TypeTag};

/// Compare the saved member with the initialized compiled schema. Callers add
/// their owner/stage context without duplicating the binary type mapping.
pub(super) fn check_member_schema(
    saved: &LegacyVmMemberSchema,
    runtime: &MemberVariable,
) -> Result<(), String> {
    let expected_kind = if runtime.ty.tag == TypeTag::NativeType {
        match runtime.ty.native_type_name.as_str() {
            "Actor" => LegacyVmMemberKind::ActorRef,
            "Scroll" => LegacyVmMemberKind::ScrollRef,
            "Location" => LegacyVmMemberKind::Location,
            other => {
                return Err(format!(
                    "initialized class uses unsupported native type {other:?}"
                ));
            }
        }
    } else {
        LegacyVmMemberKind::Raw32 {
            tag: runtime.ty.tag,
        }
    };
    if saved.name != runtime.name
        || i32::try_from(saved.address).ok() != Some(runtime.address)
        || saved.kind != expected_kind
    {
        return Err(format!(
            "saved ({:?}, {}, {:?}) != runtime ({:?}, {}, {:?})",
            saved.name, saved.address, saved.kind, runtime.name, runtime.address, expected_kind
        ));
    }
    Ok(())
}

/// Checked end of a four-byte VM member; error carries the attempted end so
/// existing owner-specific diagnostics retain their precise range evidence.
pub(super) fn member_end(address: usize, heap_len: usize) -> Result<usize, usize> {
    match address.checked_add(4) {
        Some(end) if end <= heap_len => Ok(end),
        end => Err(end.unwrap_or(usize::MAX)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_ranges_reject_truncation_and_arithmetic_overflow() {
        assert_eq!(member_end(0, 4), Ok(4));
        assert_eq!(member_end(1, 4), Err(5));
        assert_eq!(member_end(usize::MAX - 2, usize::MAX), Err(usize::MAX));
    }
}
