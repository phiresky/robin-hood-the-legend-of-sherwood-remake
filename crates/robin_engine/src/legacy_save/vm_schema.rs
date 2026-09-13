//! Schema checks shared by each independently staged VM heap adopter.

use super::adopt::LegacyEntityFixups;
use super::adopt_common::{AdoptErrorKind, AdoptSite, LegacyAdoptError};
use super::payload_base::LegacyElementRef;
use super::payload_vm::{LegacyVmMemberKind, LegacyVmMemberSchema};
use crate::element::{Entity, EntityId};
use crate::engine::EngineInner;
use crate::natives::ScriptHandleCodec;
use crate::scb::{MemberVariable, TypeTag};

/// Largest index representable in a script handle's 28-bit payload.
pub(super) const HANDLE_INDEX_MAX: usize = 0x0fff_ffff;

/// Failure of [`resolve_entity_handle`]; callers wrap it with their
/// owner/stage context.
pub(super) enum EntityHandleError {
    Reference(LegacyAdoptError),
    /// The reference resolved to a missing entity or one failing the
    /// member's class predicate.
    WrongEntity(EntityId),
    IndexOverflow(usize),
}

impl EntityHandleError {
    /// Attach the VM owner and member name; reference failures already carry
    /// their own complete message.
    pub(super) fn at(
        self,
        site: &AdoptSite,
        member: &str,
        expected: &'static str,
    ) -> LegacyAdoptError {
        match self {
            Self::Reference(error) => error,
            Self::WrongEntity(entity_id) => site.field_error(
                member.to_owned(),
                AdoptErrorKind::WrongEntityKind {
                    entity_id,
                    expected,
                },
            ),
            Self::IndexOverflow(index) => site.field_error(
                member.to_owned(),
                AdoptErrorKind::VmHandleOverflow { index },
            ),
        }
    }
}

/// Check a saved Location's sector and layer against the initialized
/// topology before it is allocated into the shared VM arena.
pub(super) fn check_location_topology(
    site: &AdoptSite,
    member: &str,
    sector: Option<u16>,
    sector_count: usize,
    layer: u16,
    layer_count: usize,
) -> Result<(), LegacyAdoptError> {
    if let Some(sector) = sector
        && usize::from(sector) >= sector_count
    {
        return Err(site.out_of_range(
            member.to_owned(),
            "sector",
            usize::from(sector),
            sector_count,
        ));
    }
    if usize::from(layer) >= layer_count {
        return Err(site.out_of_range(member.to_owned(), "layer", usize::from(layer), layer_count));
    }
    Ok(())
}

/// Resolve a saved `Actor`/`Scroll` VM member to its script handle bits:
/// creation-order lookup, then class predicate, then handle-range check.
/// A null reference encodes as 0.
pub(super) fn resolve_entity_handle(
    engine: &EngineInner,
    entities: &LegacyEntityFixups,
    reference: LegacyElementRef,
    predicate: impl FnOnce(&Entity) -> bool,
) -> Result<u32, EntityHandleError> {
    let Some(entity_id) = entities
        .resolve_element(reference)
        .map_err(EntityHandleError::Reference)?
    else {
        return Ok(0);
    };
    if !engine.world.entities.get(entity_id).is_some_and(predicate) {
        return Err(EntityHandleError::WrongEntity(entity_id));
    }
    let index = entity_id.index() as usize;
    if index > HANDLE_INDEX_MAX {
        return Err(EntityHandleError::IndexOverflow(index));
    }
    Ok(ScriptHandleCodec::actor_handle(entity_id) as u32)
}

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
