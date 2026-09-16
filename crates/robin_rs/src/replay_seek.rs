//! A browser download handed off once to its matching replay session.
use robin_engine::replay::{ReplayData, ReplayFile};
use robin_replay_format::seek::ReplaySeekSidecar;
use sha2::{Digest, Sha256};

thread_local! {
    static PENDING: std::cell::RefCell<Option<([u8; 32], ReplaySeekSidecar)>> = const { std::cell::RefCell::new(None) };
}

pub fn stage(compact: &[u8], sidecar: &[u8]) -> Result<(), String> {
    PENDING.with(|pending| pending.borrow_mut().take());
    let (_, replay) = crate::replay_format::decode_compact_for_local_playback(compact)
        .map_err(|e| e.to_string())?;
    let sidecar = ReplaySeekSidecar::decode(sidecar, Sha256::digest(compact).into(), &replay)?;
    let identity = Sha256::digest(bitcode::encode(&ReplayFile::from(&replay))).into();
    PENDING.with(|pending| *pending.borrow_mut() = Some((identity, sidecar)));
    Ok(())
}

pub(crate) fn take(replay: &ReplayData) -> Option<ReplaySeekSidecar> {
    PENDING.with(|pending| {
        let (expected, sidecar) = pending.borrow_mut().take()?;
        let actual: [u8; 32] = Sha256::digest(bitcode::encode(&ReplayFile::from(replay))).into();
        if actual == expected {
            Some(sidecar)
        } else {
            tracing::warn!("discarding seek sidecar for a different replay");
            None
        }
    })
}
