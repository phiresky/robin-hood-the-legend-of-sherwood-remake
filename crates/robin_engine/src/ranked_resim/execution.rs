//! Verifier-owned rules, supplied afresh for every engine transaction.
use crate::engine::{
    Engine, FrameAdvanceError, LevelAssets, RankedSimulationPolicy, SimConfig,
    SimulationCommandPhase, SimulationFrameInput, SimulationFrameOutput,
};
use serde::{Deserialize, Serialize};

/// Execution rules selected from trusted board configuration. They are not
/// engine state and therefore survive engine replacement without entering saves.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RankedExecutionContext {
    policy: RankedSimulationPolicy,
}

impl RankedExecutionContext {
    pub const fn new(policy: RankedSimulationPolicy) -> Self {
        Self { policy }
    }

    pub fn advance_frame(
        &self,
        engine: &mut Engine,
        assets: &LevelAssets,
        frame: SimulationFrameInput,
    ) -> Result<SimulationFrameOutput, FrameAdvanceError> {
        engine.advance_frame_with_execution(assets, frame, self)
    }

    pub(crate) fn validate_config(&self, observed: SimConfig) -> Result<(), FrameAdvanceError> {
        self.policy
            .validate_config(observed)
            .map_err(|error| match error {
                crate::engine::RankedSimulationPolicyError::ConfigMismatch { field } => {
                    tracing::error!(
                        field = field.config_field(),
                        "ranked frame rejected because its immutable simulation config drifted"
                    );
                    FrameAdvanceError::RankedSimulationConfigViolation { field }
                }
                error @ (crate::engine::RankedSimulationPolicyError::InvalidIdentity(_)
                | crate::engine::RankedSimulationPolicyError::MissingCustomConfiguration
                | crate::engine::RankedSimulationPolicyError::InvalidCustomConfiguration(
                    _,
                )) => {
                    unreachable!("verifier policy was already validated: {error}")
                }
            })
    }

    pub(crate) fn validate_commands(
        commands: &[crate::engine::SimCommand],
        phase: SimulationCommandPhase,
    ) -> Result<(), FrameAdvanceError> {
        for (index, command) in commands.iter().enumerate() {
            let Some(field) = command
                .player_input()
                .command
                .ranked_simulation_setting_mutation()
            else {
                continue;
            };
            tracing::error!(
                ?phase,
                index,
                player_id = command.player_input().player_id.0,
                command = ?command.player_input().command,
                field = field.config_field(),
                "rejected command which attempted to edit an immutable ranked setting"
            );
            return Err(FrameAdvanceError::RankedSimulationSettingCommandRejected {
                phase,
                index,
                field,
            });
        }
        Ok(())
    }

    /// Prove that a live host speech completion came from the sealed timing
    /// catalog admitted before ranked engine construction. The concrete audio
    /// sample is presentation-only; the duration is the only selected value
    /// that enters simulation state. Explicit variants therefore bind one
    /// exact ordered catalog entry; random playback uses the group's longest
    /// English duration, independent of the local audio variant.
    pub(crate) fn validate_speech_resolution(
        &self,
        assets: &LevelAssets,
        pending: &crate::sound::PendingExclamation,
        resolution: &crate::sound::ResolvedExclamation,
    ) -> Result<(), String> {
        let identifier = (pending.profile_id & 0xFFFF_0000) | u32::from(pending.exclamation_id);
        let group = assets
            .audio
            .speech_timing_catalog
            .groups
            .get(&identifier)
            .ok_or_else(|| {
                format!("ranked sound resolution {identifier:#010x} has no sealed timing group")
            })?;
        if group.variants.is_empty() {
            return Err(format!(
                "ranked sound timing group {identifier:#010x} has no authored variants"
            ));
        }

        let duration_matches = match pending.variant {
            -1 => {
                group
                    .variants
                    .iter()
                    .filter_map(|variant| variant.duration_frames)
                    .max()
                    == Some(resolution.duration_frames)
            }
            explicit if explicit >= 0 => {
                let variant_index = usize::try_from(explicit).map_err(|_| {
                    format!(
                        "ranked sound variant {explicit} for {identifier:#010x} is not representable"
                    )
                })?;
                let variant = group.variants.get(variant_index).ok_or_else(|| {
                    format!(
                        "ranked sound variant {variant_index} is outside the {} authored variants for {identifier:#010x}",
                        group.variants.len()
                    )
                })?;
                let expected = variant.duration_frames.ok_or_else(|| {
                    format!(
                        "ranked sound variant {variant_index} for {identifier:#010x} has no authoritative duration"
                    )
                })?;
                expected == resolution.duration_frames
            }
            invalid => {
                return Err(format!(
                    "ranked sound request {identifier:#010x} has invalid variant {invalid}"
                ));
            }
        };
        if !duration_matches {
            return Err(format!(
                "ranked sound resolution {identifier:#010x} supplied unauthoritative duration {}",
                resolution.duration_frames
            ));
        }
        Ok(())
    }
}
