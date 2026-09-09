//! Ranked policy over validated replay storage.
use super::*;

impl ReplayData {
    pub fn contains_state_loads(&self) -> bool {
        !self.load_backs.is_empty()
    }

    /// Ranked recordings carry one pre-frame hash at frame zero and every
    /// lockstep hash interval thereafter, with no off-cadence extras.
    pub fn validate_ranked_hash_coverage(&self) -> Result<(), String> {
        let interval = crate::multiplayer::STATE_HASH_INTERVAL as usize;
        for ordinal in (0..self.header.total_frames).step_by(interval) {
            if !self.hashes.contains_key(&ordinal) {
                return Err(format!(
                    "ranked replay is missing state hash at ordinal {ordinal}"
                ));
            }
        }
        if self.hashes.keys().any(|ordinal| {
            *ordinal >= self.header.total_frames
                || !ordinal.is_multiple_of(crate::multiplayer::STATE_HASH_INTERVAL)
        }) {
            return Err("ranked replay contains an out-of-cadence state hash".to_owned());
        }
        Ok(())
    }

    /// Fold recorder evidence together with taints that can be reconstructed
    /// directly from deterministic commands and host actions. Omission loses:
    /// deleting an explicit marker cannot hide a console, cheat, load, or
    /// restart that is still visible in the replay itself.
    pub fn rankability(
        &self,
    ) -> Result<ReplayRankability, crate::replay_rankability::RankabilityEvidenceError> {
        self.header.rankability.validate()?;
        // A marker restore can be independently reconstructed from the root.
        // Embedded payloads cannot prove the gameplay that produced their state.
        let verified_load_history = self.contains_state_loads()
            && self.load_backs.values().all(|load| load.snapshot.is_none());
        let permitted_restore = |kind| {
            verified_load_history
                && matches!(
                    kind,
                    InputTaintKind::StateLoad | InputTaintKind::MissionRestart
                )
        };
        let mut rankability = ReplayRankability::rankable();
        rankability.include_all(
            self.header
                .rankability
                .taints()
                .iter()
                .copied()
                .filter(|taint| !permitted_restore(taint.kind)),
        );
        for (&frame, load) in self.load_backs.iter() {
            if load.snapshot.is_some() {
                rankability.taint(InputTaintKind::StateLoad, frame);
            }
        }
        for (&ordinal, frame) in self.frames.iter() {
            for kind in detected_input_taints(&frame.input, &frame.host_controls) {
                if !permitted_restore(kind) {
                    rankability.taint(kind, ordinal);
                }
            }
        }
        Ok(rankability)
    }

    pub fn ranked_submission_verdict(
        &self,
    ) -> Result<(), crate::replay_rankability::RankedIneligibilityReason> {
        self.rankability()
            .map_err(|_| crate::replay_rankability::RankedIneligibilityReason::MalformedEvidence)?
            .verdict()
    }

    pub fn validate_ranked_command_admission(
        &self,
        transcript: &robin_run_protocol::ReplaySessionTranscriptV1,
    ) -> Result<(), String> {
        self.validate_ranked_command_admission_inner(Some(transcript))
    }

    pub fn validate_canonical_ranked_command_admission(&self) -> Result<(), String> {
        self.validate_ranked_command_admission_inner(None)
    }

    fn validate_ranked_command_admission_inner(
        &self,
        transcript: Option<&robin_run_protocol::ReplaySessionTranscriptV1>,
    ) -> Result<(), String> {
        use crate::player_command::{PlayerCommand, PlayerId};
        use robin_run_protocol::{MAX_REPLAY_SEATS_V1, ReplaySeatLifecycleKindV1, Validate as _};

        let host_seat = u16::from(PlayerId::HOST.0);
        let mut occupied = BTreeSet::from([host_seat]);
        let mut seen = BTreeSet::from([host_seat]);
        let mut next_event = 1usize;
        if let Some(transcript) = transcript {
            transcript
                .validate()
                .map_err(|error| format!("invalid ranked session transcript: {error}"))?;
            if transcript
                .events
                .iter()
                .any(|event| event.replay_ordinal >= self.frame_count())
            {
                return Err("ranked session transcript event lies outside replay".to_owned());
            }
        }

        for ordinal in 0..self.frame_count() {
            let frame = self
                .frame(ordinal)
                .ok_or_else(|| format!("replay frame {ordinal} is absent"))?;
            for command in frame
                .input
                .commands
                .iter()
                .chain(&frame.input.post_commands)
            {
                let input = command.player_input();
                let seat = u16::from(input.player_id.0);
                match &input.command {
                    PlayerCommand::ConnectSeat { player_id, .. }
                    | PlayerCommand::DisconnectSeat { player_id } => {
                        if input.player_id != PlayerId::HOST {
                            return Err(format!(
                                "frame {ordinal}: non-host authored seat lifecycle"
                            ));
                        }
                        let target = u16::from(player_id.0);
                        if let Some(transcript) = transcript {
                            let event = transcript.events.get(next_event).ok_or_else(|| {
                                format!("frame {ordinal}: replay has extra lifecycle command")
                            })?;
                            if event.replay_ordinal != ordinal || event.seat != target {
                                return Err(format!(
                                    "frame {ordinal}: lifecycle command differs from transcript event {}",
                                    event.event_ordinal
                                ));
                            }
                            match (&input.command, event.lifecycle) {
                                (
                                    PlayerCommand::ConnectSeat { .. },
                                    ReplaySeatLifecycleKindV1::Connected { .. },
                                ) => {
                                    if !occupied.insert(target) {
                                        return Err(format!(
                                            "frame {ordinal}: connects occupied seat {target}"
                                        ));
                                    }
                                    seen.insert(target);
                                }
                                (
                                    PlayerCommand::DisconnectSeat { .. },
                                    ReplaySeatLifecycleKindV1::Disconnected,
                                ) => {
                                    if target == host_seat || !occupied.remove(&target) {
                                        return Err(format!(
                                            "frame {ordinal}: disconnects vacant/host seat {target}"
                                        ));
                                    }
                                }
                                _ => {
                                    return Err(format!(
                                        "frame {ordinal}: lifecycle kind differs from transcript"
                                    ));
                                }
                            }
                            next_event += 1;
                        } else {
                            match &input.command {
                                PlayerCommand::ConnectSeat { .. } => {
                                    if target >= MAX_REPLAY_SEATS_V1
                                        || (!seen.contains(&target)
                                            && usize::from(target) != seen.len())
                                        || !occupied.insert(target)
                                    {
                                        return Err(format!(
                                            "frame {ordinal}: non-canonical seat connection {target}"
                                        ));
                                    }
                                    seen.insert(target);
                                }
                                PlayerCommand::DisconnectSeat { .. } => {
                                    if target == host_seat || !occupied.remove(&target) {
                                        return Err(format!(
                                            "frame {ordinal}: invalid seat disconnection {target}"
                                        ));
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                    _ => {
                        if !occupied.contains(&seat) {
                            return Err(format!(
                                "frame {ordinal}: command uses unoccupied seat {seat}"
                            ));
                        }
                    }
                }
            }
            if transcript.is_some_and(|transcript| {
                transcript
                    .events
                    .get(next_event)
                    .is_some_and(|event| event.replay_ordinal == ordinal)
            }) {
                return Err(format!(
                    "frame {ordinal}: transcript lifecycle event has no replay command"
                ));
            }
        }
        if let Some(transcript) = transcript
            && next_event != transcript.events.len()
        {
            return Err("ranked transcript has unconsumed lifecycle events".to_owned());
        }
        Ok(())
    }
}
