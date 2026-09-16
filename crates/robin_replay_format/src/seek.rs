//! Derived replay seek artifacts. The signed replay remains unchanged.
use robin_engine::{
    engine::{Engine, HostEffects, HostSignal, SimulationFrameOutput},
    replay::{ReplayData, state_hash},
};
use serde::{Deserialize, Serialize};
use std::io::Read;

pub const INTERVAL: u32 = 250;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_DECODED_BYTES: usize = 128 * 1024 * 1024;
pub const MEDIA_TYPE: &str = "application/x-robin-rhseek";
const MAGIC: &[u8] = b"RHSEEK\x01";

#[derive(Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct SeekSnapshot {
    pub ordinal: u32,
    pub timeline: u32,
    pub state_hash: u64,
    pub engine: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct SeekEffects {
    pub ordinal: u32,
    pub effects: HostEffects,
    pub draw_hidden: Option<bool>,
}

#[derive(Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ReplaySeekSidecar {
    pub replay_sha256: [u8; 32],
    pub engine_version: String,
    pub replay_schema: u32,
    pub frames: u32,
    pub checkpoints: Vec<SeekSnapshot>,
    pub saves: Vec<SeekSnapshot>,
    /// Sparse modal/presentation effects let the client rebuild host checkpoints
    /// without resimulating the prefix. Sound playback is reset on seeking.
    pub effects: Vec<SeekEffects>,
    #[serde(skip)]
    #[bitcode(skip)]
    capture_bytes: usize,
    #[serde(skip)]
    #[bitcode(skip)]
    exceeded_limit: bool,
}

impl ReplaySeekSidecar {
    pub fn validate_engines(&self) -> Result<(), String> {
        for snapshot in self.checkpoints.iter().chain(&self.saves) {
            let engine = Engine::decode_native_snapshot(&snapshot.engine)?;
            if state_hash(&engine) != snapshot.state_hash {
                return Err("seek snapshot hash mismatch".into());
            }
        }
        Ok(())
    }
    pub fn new(replay_sha256: [u8; 32], replay: &ReplayData) -> Self {
        Self {
            replay_sha256,
            engine_version: crate::ENGINE_VERSION_HASH.into(),
            replay_schema: replay.header().version,
            frames: replay.frame_count(),
            checkpoints: Vec::new(),
            saves: Vec::new(),
            effects: Vec::new(),
            capture_bytes: 0,
            exceeded_limit: false,
        }
    }

    pub fn observe(
        &mut self,
        replay: &ReplayData,
        ordinal: u32,
        engine: &Engine,
        output: Option<&SimulationFrameOutput>,
    ) {
        if self.exceeded_limit {
            return;
        }
        if let Some(output) = output {
            let mut effects = HostEffects::default();
            let mut draw_hidden = None;
            for event in std::iter::once(&output.events)
                .chain(std::iter::once(&output.post_boundary_events))
                .chain(output.post_initialize_events.iter())
            {
                effects.append(event.host_effects.clone());
                if event.set_draw_hidden.is_some() {
                    draw_hidden = event.set_draw_hidden;
                }
            }
            effects.background_blits.clear();
            effects.trade_receipts.clear();
            if !effects.pending_modal_kinds().is_empty()
                || effects.has_signal(HostSignal::MissionStatePopup)
                || draw_hidden.is_some()
            {
                let event = SeekEffects {
                    ordinal,
                    effects,
                    draw_hidden,
                };
                self.capture_bytes += bitcode::encode(&event).len();
                self.effects.push(event);
            }
        } else {
            let timeline = replay
                .frame(ordinal)
                .expect("verified replay frame")
                .timeline_before;
            if ordinal.is_multiple_of(INTERVAL) {
                self.checkpoints.push(SeekSnapshot {
                    ordinal,
                    timeline,
                    state_hash: state_hash(engine),
                    engine: engine.encode_native_snapshot(),
                });
                self.capture_bytes += self.checkpoints.last().unwrap().engine.len() + 64;
            }
            if replay.save_marker_for_frame(ordinal).is_some() {
                // Capture the same persisted projection used by ranked load-back.
                let saved = Engine::from_persisted_state(
                    engine
                        .capture_persisted_state()
                        .expect("verified save projection"),
                );
                self.saves.push(SeekSnapshot {
                    ordinal,
                    timeline,
                    state_hash: state_hash(&saved),
                    engine: saved.encode_native_snapshot(),
                });
                self.capture_bytes += self.saves.last().unwrap().engine.len() + 64;
            }
        }
        if self.capture_bytes > MAX_DECODED_BYTES - 4096 {
            self.exceeded_limit = true;
            self.checkpoints.clear();
            self.saves.clear();
            self.effects.clear();
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.exceeded_limit {
            return Err("seek sidecar capture exceeds decoded limit".into());
        }
        let raw = bitcode::encode(self);
        if raw.len() > MAX_DECODED_BYTES {
            return Err("seek sidecar exceeds decoded limit".into());
        }
        let mut bytes = MAGIC.to_vec();
        bytes.extend(zstd::encode_all(raw.as_slice(), 19).map_err(|e| e.to_string())?);
        if bytes.len() > MAX_BYTES {
            return Err("seek sidecar exceeds compressed limit".into());
        }
        Ok(bytes)
    }

    pub fn decode(
        bytes: &[u8],
        replay_sha256: [u8; 32],
        replay: &ReplayData,
    ) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("seek sidecar exceeds compressed limit".into());
        }
        let compressed = bytes
            .strip_prefix(MAGIC)
            .ok_or("invalid seek sidecar header")?;
        let decoder = zstd::stream::read::Decoder::new(compressed).map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        decoder
            .take(MAX_DECODED_BYTES as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|e| e.to_string())?;
        if raw.len() > MAX_DECODED_BYTES {
            return Err("seek sidecar exceeds decoded limit".into());
        }
        let sidecar: Self = bitcode::decode(&raw).map_err(|e| e.to_string())?;
        if sidecar.replay_sha256 != replay_sha256
            || sidecar.engine_version != crate::ENGINE_VERSION_HASH
            || sidecar.replay_schema != replay.header().version
            || sidecar.frames != replay.frame_count()
        {
            return Err("seek sidecar replay or engine identity mismatch".into());
        }
        for (snapshots, saves) in [(&sidecar.checkpoints, false), (&sidecar.saves, true)] {
            let mut previous = None;
            for snapshot in snapshots {
                if previous.is_some_and(|p| p >= snapshot.ordinal)
                    || snapshot.ordinal >= sidecar.frames
                    || (!saves && !snapshot.ordinal.is_multiple_of(INTERVAL))
                    || (saves && replay.save_marker_for_frame(snapshot.ordinal).is_none())
                    || replay
                        .frame(snapshot.ordinal)
                        .is_none_or(|f| f.timeline_before != snapshot.timeline)
                {
                    return Err("invalid seek checkpoint address".into());
                }
                previous = Some(snapshot.ordinal);
            }
        }
        if sidecar.checkpoints.len() != sidecar.frames.div_ceil(INTERVAL) as usize
            || sidecar.saves.len()
                != (0..sidecar.frames)
                    .filter(|&f| replay.save_marker_for_frame(f).is_some())
                    .count()
            || sidecar
                .effects
                .windows(2)
                .any(|w| w[0].ordinal >= w[1].ordinal)
            || sidecar
                .effects
                .last()
                .is_some_and(|e| e.ordinal >= sidecar.frames)
        {
            return Err("incomplete or invalid seek sidecar index".into());
        }
        for event in &sidecar.effects {
            if event.effects.pending_modal_kinds().iter().any(|kind| {
                !matches!(
                    kind,
                    robin_engine::player_command::ModalKind::Dialog { .. }
                        | robin_engine::player_command::ModalKind::PopupText { .. }
                        | robin_engine::player_command::ModalKind::Debriefing { .. }
                        | robin_engine::player_command::ModalKind::SherwoodReport
                )
            }) {
                return Err("unsupported seek modal effect".into());
            }
        }
        Ok(sidecar)
    }
}
