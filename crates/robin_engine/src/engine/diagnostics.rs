//! Process-local, observational diagnostic selection. Never stored in an engine,
//! save, replay, or simulation hash. Environment is sampled once on first use.
//! Set gates before process startup; changing them during a session is unsupported.

use crate::element::EntityId;
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, sync::OnceLock};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct ExactOwnerFrame {
    pub frame: u32,
    pub creation_order: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct PathOwnerFilter {
    pub frame: Option<u32>,
    pub creation_order: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum OwnerKind {
    Pc,
    Soldier,
    Civilian,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct OwnerFilter {
    kind: OwnerKind,
    index: u32,
}

impl OwnerFilter {
    fn matches(self, owner: EntityId) -> bool {
        self.index == owner.index()
            && matches!(
                (self.kind, owner),
                (OwnerKind::Pc, EntityId::Pc(_))
                    | (OwnerKind::Soldier, EntityId::Soldier(_))
                    | (OwnerKind::Civilian, EntityId::Civilian(_))
            )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct OwnerFrames {
    owner: OwnerFilter,
    from: u32,
    until: u32,
}

impl OwnerFrames {
    fn matches(self, frame: u32, owner: EntityId) -> bool {
        (self.from..=self.until).contains(&frame) && self.owner.matches(owner)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct DiagnosticConfig {
    drop_boundary: Option<OwnerFrames>,
    goal_owner: Option<OwnerFrames>,
    pub motion_latch: Option<ExactOwnerFrame>,
    pub attentive_owner: Option<ExactOwnerFrame>,
    pub path_owner: Option<PathOwnerFilter>,
    pub path_barrier: bool,
    pub post_seek_handoff: bool,
}

impl DiagnosticConfig {
    pub fn drop_boundary_matches(&self, frame: u32, owner: EntityId) -> bool {
        self.drop_boundary
            .is_some_and(|filter| filter.matches(frame, owner))
    }

    pub fn goal_owner_matches(&self, frame: u32, owner: EntityId) -> bool {
        self.goal_owner
            .is_some_and(|filter| filter.matches(frame, owner))
    }

    /// Pure lookup injection keeps tests independent of global environment and
    /// makes malformed supplied values distinguishable from absent filters.
    fn parse(get: impl Fn(&str) -> Option<OsString>) -> Result<Self, String> {
        let text = |name: &str| -> Result<Option<String>, String> {
            get(name)
                .map(|value| {
                    value
                        .into_string()
                        .map_err(|_| format!("{name} must be Unicode"))
                })
                .transpose()
        };
        let number = |name: &str| -> Result<Option<u32>, String> {
            text(name)?
                .map(|value| {
                    value
                        .parse()
                        .map_err(|error| format!("invalid {name}={value:?}: {error}"))
                })
                .transpose()
        };
        let required_number = |name: &str| -> Result<u32, String> {
            number(name)?
                .ok_or_else(|| format!("{name} is required when its diagnostic gate is enabled"))
        };
        let owner = |name: &str, pc_only: bool| -> Result<OwnerFilter, String> {
            let value = text(name)?
                .ok_or_else(|| format!("{name} is required when its diagnostic gate is enabled"))?;
            let (kind, index) = value
                .split_once(':')
                .ok_or_else(|| format!("{name} must look like pc|soldier|civilian:INDEX"))?;
            let kind = match kind {
                "pc" => OwnerKind::Pc,
                "soldier" if !pc_only => OwnerKind::Soldier,
                "civilian" if !pc_only => OwnerKind::Civilian,
                _ => return Err(format!("{name} has unsupported owner kind {kind:?}")),
            };
            let index = index
                .parse()
                .map_err(|error| format!("invalid {name}={value:?}: {error}"))?;
            Ok(OwnerFilter { kind, index })
        };
        let exact =
            |gate: &str, frame: &str, creation: &str| -> Result<Option<ExactOwnerFrame>, String> {
                if get(gate).is_none() {
                    return Ok(None);
                }
                Ok(Some(ExactOwnerFrame {
                    frame: required_number(frame)?,
                    creation_order: required_number(creation)?,
                }))
            };

        let drop_boundary = if get("PARITY_DEBUG_DROP_BOUNDARY").is_some() {
            let owner = owner("PARITY_DEBUG_DROP_OWNER", true)?;
            // Exact frame takes precedence, including over malformed range values.
            let (from, until) = if let Some(frame) = number("PARITY_DEBUG_DROP_FRAME")? {
                (frame, frame)
            } else {
                let from = required_number("PARITY_DEBUG_DROP_FROM")?;
                let until = number("PARITY_DEBUG_DROP_UNTIL")?.unwrap_or(from);
                if from > until {
                    return Err(
                        "PARITY_DEBUG_DROP_FROM must not exceed PARITY_DEBUG_DROP_UNTIL".into(),
                    );
                }
                (from, until)
            };
            Some(OwnerFrames { owner, from, until })
        } else {
            None
        };
        let goal_owner = if get("PARITY_DEBUG_GOAL_OWNER_HANDOFF").is_some() {
            let frame = required_number("PARITY_DEBUG_GOAL_OWNER_FRAME")?;
            Some(OwnerFrames {
                owner: owner("PARITY_DEBUG_GOAL_OWNER", false)?,
                from: frame,
                until: frame,
            })
        } else {
            None
        };
        let path_owner = if get("PARITY_DEBUG_PATH_OWNER_LIFECYCLE").is_some() {
            Some(PathOwnerFilter {
                frame: number("PARITY_DEBUG_PATH_OWNER_FRAME")?,
                creation_order: number("PARITY_DEBUG_PATH_OWNER_CREATION_ORDER")?,
            })
        } else {
            None
        };
        Ok(Self {
            drop_boundary,
            goal_owner,
            path_owner,
            motion_latch: exact(
                "PARITY_DEBUG_MOTION_LATCH",
                "PARITY_DEBUG_MOTION_LATCH_FRAME",
                "PARITY_DEBUG_MOTION_LATCH_CREATION_ORDER",
            )?,
            attentive_owner: exact(
                "PARITY_DEBUG_ATTENTIVE_OWNER_HANDOFF",
                "PARITY_DEBUG_ATTENTIVE_OWNER_FRAME",
                "PARITY_DEBUG_ATTENTIVE_OWNER_CREATION_ORDER",
            )?,
            path_barrier: get("PARITY_DEBUG_PATH_BARRIER").is_some(),
            post_seek_handoff: get("PARITY_DEBUG_POST_SEEK_HANDOFF").is_some(),
        })
    }
}

pub(super) fn config() -> &'static DiagnosticConfig {
    static CONFIG: OnceLock<DiagnosticConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        DiagnosticConfig::parse(|name| std::env::var_os(name))
            .unwrap_or_else(|error| panic!("invalid parity diagnostic configuration: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[(&str, &str)]) -> Result<DiagnosticConfig, String> {
        DiagnosticConfig::parse(|name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(*value))
        })
    }

    #[test]
    fn disabled_gates_ignore_unused_invalid_filters() {
        let config = parse(&[
            ("PARITY_DEBUG_DROP_OWNER", "invalid"),
            ("PARITY_DEBUG_MOTION_LATCH_FRAME", "invalid"),
        ])
        .unwrap();
        assert!(!config.drop_boundary_matches(0, EntityId::Pc(crate::entity_id::PcId(0))));
        assert!(config.motion_latch.is_none());
        assert!(config.path_owner.is_none());
    }

    #[test]
    fn presence_enables_even_empty_or_zero_gate_values() {
        let config = parse(&[
            ("PARITY_DEBUG_PATH_BARRIER", ""),
            ("PARITY_DEBUG_POST_SEEK_HANDOFF", "0"),
        ])
        .unwrap();
        assert!(config.path_barrier && config.post_seek_handoff);
    }

    #[test]
    fn drop_range_is_inclusive_and_pc_only() {
        let config = parse(&[
            ("PARITY_DEBUG_DROP_BOUNDARY", "1"),
            ("PARITY_DEBUG_DROP_OWNER", "pc:2"),
            ("PARITY_DEBUG_DROP_FROM", "10"),
            ("PARITY_DEBUG_DROP_UNTIL", "12"),
        ])
        .unwrap();
        for frame in 9..=13 {
            assert_eq!(
                config.drop_boundary_matches(frame, EntityId::Pc(crate::entity_id::PcId(2))),
                (10..=12).contains(&frame)
            );
        }
        assert!(
            !config.drop_boundary_matches(10, EntityId::Soldier(crate::entity_id::SoldierId(2)))
        );
        assert!(!config.drop_boundary_matches(10, EntityId::Pc(crate::entity_id::PcId(3))));
    }

    #[test]
    fn exact_drop_frame_overrides_range() {
        let config = parse(&[
            ("PARITY_DEBUG_DROP_BOUNDARY", "1"),
            ("PARITY_DEBUG_DROP_OWNER", "pc:2"),
            ("PARITY_DEBUG_DROP_FRAME", "10"),
            ("PARITY_DEBUG_DROP_FROM", "invalid"),
        ])
        .unwrap();
        assert!(config.drop_boundary_matches(10, EntityId::Pc(crate::entity_id::PcId(2))));
        assert!(!config.drop_boundary_matches(11, EntityId::Pc(crate::entity_id::PcId(2))));
    }

    #[test]
    fn malformed_enabled_filters_are_errors() {
        for (name, value) in [
            ("PARITY_DEBUG_DROP_OWNER", "soldier:1"),
            ("PARITY_DEBUG_DROP_OWNER", "pc:no"),
            ("PARITY_DEBUG_DROP_FRAME", "-1"),
            ("PARITY_DEBUG_DROP_FRAME", "4294967296"),
        ] {
            let mut values = vec![
                ("PARITY_DEBUG_DROP_BOUNDARY", "1"),
                ("PARITY_DEBUG_DROP_OWNER", "pc:1"),
                ("PARITY_DEBUG_DROP_FRAME", "1"),
            ];
            values.iter_mut().find(|(key, _)| *key == name).unwrap().1 = value;
            assert!(parse(&values).unwrap_err().contains(name));
        }
        assert!(parse(&[("PARITY_DEBUG_MOTION_LATCH", "1")]).is_err());
        assert!(
            parse(&[
                ("PARITY_DEBUG_DROP_BOUNDARY", "1"),
                ("PARITY_DEBUG_DROP_OWNER", "pc:1"),
                ("PARITY_DEBUG_DROP_FROM", "2"),
                ("PARITY_DEBUG_DROP_UNTIL", "1")
            ])
            .is_err()
        );
    }

    #[test]
    fn goal_owner_matches_each_supported_kind() {
        for (kind, owner) in [
            ("pc:3", EntityId::Pc(crate::entity_id::PcId(3))),
            (
                "soldier:3",
                EntityId::Soldier(crate::entity_id::SoldierId(3)),
            ),
            (
                "civilian:3",
                EntityId::Civilian(crate::entity_id::CivilianId(3)),
            ),
        ] {
            let config = parse(&[
                ("PARITY_DEBUG_GOAL_OWNER_HANDOFF", "1"),
                ("PARITY_DEBUG_GOAL_OWNER", kind),
                ("PARITY_DEBUG_GOAL_OWNER_FRAME", "4"),
            ])
            .unwrap();
            assert!(config.goal_owner_matches(4, owner));
            assert!(!config.goal_owner_matches(5, owner));
        }
    }

    #[test]
    fn exact_and_optional_creation_filters_are_preserved() {
        let config = parse(&[
            ("PARITY_DEBUG_ATTENTIVE_OWNER_HANDOFF", "1"),
            ("PARITY_DEBUG_ATTENTIVE_OWNER_FRAME", "6"),
            ("PARITY_DEBUG_ATTENTIVE_OWNER_CREATION_ORDER", "8"),
            ("PARITY_DEBUG_PATH_OWNER_LIFECYCLE", "1"),
            ("PARITY_DEBUG_PATH_OWNER_FRAME", "6"),
        ])
        .unwrap();
        let exact = config.attentive_owner.unwrap();
        assert_eq!((exact.frame, exact.creation_order), (6, 8));
        let path = config.path_owner.unwrap();
        assert_eq!(path.frame, Some(6));
        assert_eq!(path.creation_order, None);
    }

    #[cfg(unix)]
    #[test]
    fn supplied_non_unicode_filter_is_not_silently_absent() {
        use std::os::unix::ffi::OsStringExt;
        let result = DiagnosticConfig::parse(|name| match name {
            "PARITY_DEBUG_PATH_OWNER_LIFECYCLE" => Some("1".into()),
            "PARITY_DEBUG_PATH_OWNER_FRAME" => Some(OsString::from_vec(vec![0xff])),
            _ => None,
        });
        assert!(
            result
                .unwrap_err()
                .contains("PARITY_DEBUG_PATH_OWNER_FRAME")
        );
    }
}
