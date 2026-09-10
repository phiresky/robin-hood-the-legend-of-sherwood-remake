//! Shared admission workflow. Platform adapters own storage, mounting and waiting policy.
use crate::distributed_mod::{make_distributed_mod_offer, offer_trust_identity};
use crate::distributed_mod_admission::{
    AdmittedDistributedMod, OFFER_WAIT, TRANSFER_WAIT, append_chunk, finish_transfer,
    mount_validated_distributed_mod, resume_offset,
};
use crate::distributed_mod_policy::AdmissionState;
use crate::host::ApplicationContext;
use crate::multiplayer::{NetChannels, NetEvent};
use robin_engine::multiplayer::DistributedModOffer;

/// Admission retains at most 1024 unrelated transport events across its offer
/// and transfer phases. Exceeding this aborts admission explicitly; accepted
/// events are never silently dropped or reordered. Per-message wire bounds
/// remain the transport's responsibility.
const DEFERRED_EVENT_LIMIT: usize = 1024;

fn enforce_deadline(
    channels: &NetChannels,
    offer: &DistributedModOffer,
    budget: Option<std::time::Duration>,
    elapsed: std::time::Duration,
    phase: &str,
) -> Result<(), String> {
    if budget.is_some_and(|duration| elapsed >= duration) {
        let message = format!("{phase} timed out");
        channels.reject_content(offer.full_mod_sha256, message.clone());
        return Err(message);
    }
    Ok(())
}

fn defer_admission_event(
    channels: &NetChannels,
    offer: &DistributedModOffer,
    deferred: &mut Vec<NetEvent>,
    event: NetEvent,
) -> Result<(), String> {
    if deferred.len() >= DEFERRED_EVENT_LIMIT {
        let message = format!(
            "host-content admission exceeded its limit of {DEFERRED_EVENT_LIMIT} deferred transport events"
        );
        channels.reject_content(offer.full_mod_sha256, message.clone());
        return Err(message);
    }
    deferred.push(event);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DistributedModAdmissionPurpose {
    JoinSession,
    PrepareOnly,
}

/// Receive, validate, and mount one already-trusted exact offer.
///
/// Unknown hashes are rejected here instead of prompting from a loading
/// screen. Interactive joins perform the dedicated consent flow in the lobby,
/// then reconnect through this same function from mission bootstrap.
pub async fn admit_trusted_distributed_mod(
    application_context: &ApplicationContext,
    channels: &NetChannels,
    offer: &DistributedModOffer,
    purpose: DistributedModAdmissionPurpose,
) -> Result<AdmittedDistributedMod, String> {
    let (trust_key, _) = offer_trust_identity(offer)?;
    crate::distributed_mod_policy::check_consent(
        offer,
        application_context
            .with_active_profile(|profile| profile.gameplay_config.enable_spellforge_missions)?,
        application_context.is_spellforge_content_trusted(trust_key)?,
    )
    .inspect_err(|message| channels.reject_content(offer.full_mod_sha256, message.clone()))?;

    let resume_offset = resume_offset(application_context, offer)
        .await
        .inspect_err(|message| channels.reject_content(offer.full_mod_sha256, message.clone()))?;
    let mut admission =
        AdmissionState::new(offer.full_mod_sha256, offer.encoded_bytes, resume_offset)?;
    let mut deferred = Vec::new();
    await_authenticated_offer(channels, offer, &mut deferred, OFFER_WAIT).await?;
    channels.request_content(offer.full_mod_sha256, resume_offset)?;

    let mut durable_offset = resume_offset;
    let transfer_started = web_time::Instant::now();
    while durable_offset < offer.encoded_bytes {
        enforce_deadline(
            channels,
            offer,
            TRANSFER_WAIT,
            transfer_started.elapsed(),
            "host-content transfer",
        )?;
        match channels.try_recv_event() {
            Ok(NetEvent::ContentOffer(seen)) if &seen == offer => {
                // `ClientHandle::content_offer` and this event intentionally
                // expose the same prelude to synchronous and event-driven
                // consumers. Consume the event exactly once here.
            }
            Ok(NetEvent::ContentOffer(seen)) => {
                let message = format!(
                    "transport replaced content offer {} with {} during admission",
                    robin_engine::spellforge::hex_hash(&offer.full_mod_sha256),
                    robin_engine::spellforge::hex_hash(&seen.full_mod_sha256)
                );
                channels.reject_content(offer.full_mod_sha256, message.clone());
                return Err(message);
            }
            Ok(NetEvent::ContentChunk {
                full_mod_sha256,
                offset,
                total_bytes,
                bytes,
            }) => {
                if let Err(message) =
                    admission.check_chunk(full_mod_sha256, total_bytes, offset, bytes.len())
                {
                    channels.reject_content(offer.full_mod_sha256, message.clone());
                    return Err(message);
                }
                durable_offset = append_chunk(application_context, offer, offset, &bytes)
                    .await
                    .inspect_err(|message| {
                        channels.reject_content(offer.full_mod_sha256, message.clone())
                    })?;
                admission.committed(durable_offset)?;
            }
            Ok(NetEvent::Fatal(message)) => return Err(message),
            Ok(event) => defer_admission_event(channels, offer, &mut deferred, event)?,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                crate::window::sleep_ms(10).await;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err("multiplayer transport closed during mod transfer".to_owned());
            }
        }
    }

    // The final asynchronous durable write can itself cross the deadline.
    enforce_deadline(
        channels,
        offer,
        TRANSFER_WAIT,
        transfer_started.elapsed(),
        "host-content transfer",
    )?;

    let lease = finish_transfer(
        application_context,
        offer,
        resume_offset == offer.encoded_bytes,
    )
    .await;
    let lease = match lease {
        Ok(lease) => lease,
        Err(error) => {
            channels.reject_content(offer.full_mod_sha256, error.clone());
            return Err(error);
        }
    };

    let reconstructed = make_distributed_mod_offer(
        &lease.validated,
        lease.encoded().len() as u64,
        offer.host_endpoint_id.clone(),
    )
    .map_err(|error| format!("reconstruct downloaded content offer: {error}"))?;
    if &reconstructed != offer {
        let message =
            "downloaded package metadata does not exactly match the authenticated offer".to_owned();
        channels.reject_content(offer.full_mod_sha256, message.clone());
        return Err(message);
    }

    admission.validated()?;
    let mount = match mount_validated_distributed_mod(
        &lease.validated,
        application_context.preparation_files()?.clone(),
    ) {
        Ok(mount) => mount,
        Err(error) => {
            let message = format!("mount verified host content: {error}");
            channels.reject_content(offer.full_mod_sha256, message.clone());
            return Err(message);
        }
    };
    admission.mounted()?;
    channels.defer_events(deferred);
    admission.acknowledge()?;
    match purpose {
        DistributedModAdmissionPurpose::JoinSession => {
            channels.send_content_ready(offer.full_mod_sha256)
        }
        DistributedModAdmissionPurpose::PrepareOnly => {
            channels.send_content_prepared(offer.full_mod_sha256)
        }
    }?;
    Ok(AdmittedDistributedMod {
        mount,
        cache_lease: lease,
    })
}

/// Wait only according to the adapter's explicit offer delivery contract.
async fn await_authenticated_offer(
    channels: &NetChannels,
    offer: &DistributedModOffer,
    deferred: &mut Vec<NetEvent>,
    wait_budget: Option<std::time::Duration>,
) -> Result<(), String> {
    let started = web_time::Instant::now();
    loop {
        enforce_deadline(
            channels,
            offer,
            wait_budget,
            started.elapsed(),
            "authenticated content offer",
        )?;
        match channels.try_recv_event() {
            Ok(NetEvent::ContentOffer(seen)) if &seen == offer => break,
            Ok(NetEvent::ContentOffer(_)) => {
                let message =
                    "transport exposed a different authenticated content offer".to_owned();
                channels.reject_content(offer.full_mod_sha256, message.clone());
                return Err(message);
            }
            Ok(NetEvent::Fatal(message)) => return Err(message),
            Ok(event) => defer_admission_event(channels, offer, deferred, event)?,
            Err(std::sync::mpsc::TryRecvError::Empty) if wait_budget.is_some() => {
                crate::window::sleep_ms(10).await;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                return Err("transport did not expose its authenticated content offer within the platform admission window".to_owned());
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err("multiplayer transport closed before mod transfer".to_owned());
            }
        }
    }

    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn expired_offer_deadline_preempts_a_nonempty_event_flood() {
        let (channels, incoming, outgoing, _, _) = NetChannels::new();
        for index in 0..16 {
            incoming.send(NetEvent::Note(index.to_string())).unwrap();
        }
        incoming.send(NetEvent::ContentOffer(offer())).unwrap();
        let mut deferred = Vec::new();
        let error = pollster::block_on(await_authenticated_offer(
            &channels,
            &offer(),
            &mut deferred,
            Some(std::time::Duration::ZERO),
        ))
        .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(deferred.is_empty());
        assert!(matches!(channels.try_recv_event().unwrap(), NetEvent::Note(note) if note == "0"));
        assert!(matches!(
            outgoing.try_recv().unwrap(),
            robin_engine::multiplayer::NetOutbound::ContentReject { .. }
        ));
    }

    #[test]
    fn deadlines_use_elapsed_time_even_when_events_are_available() {
        for phase in ["authenticated content offer", "host-content transfer"] {
            let (channels, incoming, outgoing, _, _) = NetChannels::new();
            incoming.send(NetEvent::Note("queued".into())).unwrap();
            let budget = std::time::Duration::from_secs(15);
            enforce_deadline(
                &channels,
                &offer(),
                Some(budget),
                budget - std::time::Duration::from_nanos(1),
                phase,
            )
            .unwrap();
            assert!(enforce_deadline(&channels, &offer(), Some(budget), budget, phase).is_err());
            assert!(matches!(
                outgoing.try_recv().unwrap(),
                robin_engine::multiplayer::NetOutbound::ContentReject { .. }
            ));
            enforce_deadline(&channels, &offer(), None, std::time::Duration::MAX, phase).unwrap();
        }
    }

    #[test]
    fn unrelated_event_flood_rejects_without_silently_trimming_the_queue() {
        for budget in [None, Some(std::time::Duration::MAX)] {
            let (channels, incoming, outgoing, _, _) = NetChannels::new();
            for index in 0..=DEFERRED_EVENT_LIMIT {
                incoming.send(NetEvent::Note(index.to_string())).unwrap();
            }
            incoming.send(NetEvent::ContentOffer(offer())).unwrap();
            let mut deferred = Vec::new();
            let error = pollster::block_on(await_authenticated_offer(
                &channels,
                &offer(),
                &mut deferred,
                budget,
            ))
            .unwrap_err();
            assert!(error.contains("deferred transport events"));
            assert_eq!(deferred.len(), DEFERRED_EVENT_LIMIT);
            for (index, event) in deferred.iter().enumerate() {
                assert!(matches!(event, NetEvent::Note(note) if note == &index.to_string()));
            }
            assert!(matches!(
                outgoing.try_recv().unwrap(),
                robin_engine::multiplayer::NetOutbound::ContentReject { .. }
            ));
        }
    }

    #[test]
    fn deferred_budget_spans_both_phases_and_preserves_order_on_replay() {
        let (channels, incoming, _outgoing, _, _) = NetChannels::new();
        for index in 0..DEFERRED_EVENT_LIMIT - 1 {
            incoming.send(NetEvent::Note(index.to_string())).unwrap();
        }
        incoming.send(NetEvent::ContentOffer(offer())).unwrap();
        incoming
            .send(NetEvent::Note("after admission".into()))
            .unwrap();
        let mut deferred = Vec::new();
        pollster::block_on(await_authenticated_offer(
            &channels,
            &offer(),
            &mut deferred,
            None,
        ))
        .unwrap();
        // The transfer phase uses the same queue and the same bound.
        defer_admission_event(
            &channels,
            &offer(),
            &mut deferred,
            NetEvent::Note((DEFERRED_EVENT_LIMIT - 1).to_string()),
        )
        .unwrap();
        channels.defer_events(deferred);
        for index in 0..DEFERRED_EVENT_LIMIT {
            assert!(
                matches!(channels.try_recv_event().unwrap(), NetEvent::Note(note) if note == index.to_string())
            );
        }
        assert!(
            matches!(channels.try_recv_event().unwrap(), NetEvent::Note(note) if note == "after admission")
        );
    }

    fn offer() -> DistributedModOffer {
        DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: None,
            spellforge_vm_abi: None,
            encoded_bytes: 4,
            mission_basename: "mission".into(),
            mission_rhm_entry: "Data/Levels/mission.rhm".into(),
            map_filename: "map".into(),
            title: "title".into(),
            claimed_author: "author".into(),
            version: "1".into(),
            source_url: "https://example.invalid".into(),
            license: "CC0-1.0".into(),
            host_endpoint_id: "host".into(),
        }
    }

    #[test]
    fn both_offer_delivery_policies_drain_unrelated_events_until_the_exact_offer() {
        for budget in [None, Some(std::time::Duration::from_secs(15))] {
            let (channels, incoming, _outgoing, _, _) = NetChannels::new();
            incoming.send(NetEvent::Note("before".into())).unwrap();
            incoming.send(NetEvent::ContentOffer(offer())).unwrap();
            let mut deferred = Vec::new();
            pollster::block_on(await_authenticated_offer(
                &channels,
                &offer(),
                &mut deferred,
                budget,
            ))
            .unwrap();
            assert!(matches!(&deferred[..], [NetEvent::Note(note)] if note == "before"));
        }
    }

    #[test]
    fn unrelated_events_cannot_substitute_for_an_offer() {
        let (channels, incoming, _outgoing, _, _) = NetChannels::new();
        incoming.send(NetEvent::Note("before".into())).unwrap();
        assert!(
            pollster::block_on(await_authenticated_offer(
                &channels,
                &offer(),
                &mut Vec::new(),
                None
            ))
            .is_err()
        );
    }

    #[test]
    fn replaced_or_fatal_offer_preludes_fail_for_both_policies() {
        for budget in [None, Some(std::time::Duration::from_secs(15))] {
            for fatal in [false, true] {
                let (channels, incoming, _outgoing, _, _) = NetChannels::new();
                let mut replaced = offer();
                replaced.full_mod_sha256 = [2; 32];
                incoming
                    .send(if fatal {
                        NetEvent::Fatal("closed".into())
                    } else {
                        NetEvent::ContentOffer(replaced)
                    })
                    .unwrap();
                assert!(
                    pollster::block_on(await_authenticated_offer(
                        &channels,
                        &offer(),
                        &mut Vec::new(),
                        budget
                    ))
                    .is_err()
                );
            }
        }
    }
}
