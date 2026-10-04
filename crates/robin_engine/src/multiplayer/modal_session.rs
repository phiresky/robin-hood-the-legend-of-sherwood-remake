//! Session-owned modal identities, acknowledgements and durable decisions.
//! Widgets are projections of this state; transport delivery and rendering
//! may run at different rates, including while the simulation is paused.

use super::*;

#[derive(Debug, Serialize, Deserialize)]
struct ModalOccurrenceState {
    kind: ModalKind,
    next_occurrence: u64,
    active: Option<ModalInstanceId>,
    pending: std::collections::VecDeque<ModalInstanceId>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ModalVotes {
    instance: ModalInstanceId,
    kind: ModalKind,
    seats: [bool; crate::coop::MAX_PLAYERS],
    host_result: Option<DialogResult>,
}

#[derive(Debug, Default)]
pub(super) struct ModalSyncState {
    pub(super) session_id: Option<MultiplayerSessionId>,
    occurrences: Vec<ModalOccurrenceState>,
    inbox: std::collections::VecDeque<NetEvent>,
    visible_requests: std::collections::VecDeque<VisibleModalRequest>,
    required_players: u8,
    votes: Vec<ModalVotes>,
    player_names: Vec<String>,
    announced: Vec<(ModalInstanceId, ModalKind)>,
    decisions: Vec<ModalDecision>,
    local_proposals: Vec<ModalProposal>,
    pending_openings: Vec<ModalProgress>,
    emitted: std::collections::BTreeMap<u32, Vec<ModalKind>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModalPublication {
    HostDecisionQueued,
    ClientProposalQueued,
    HostVoteQueued,
}

/// Process binding to the session journal, not persisted game state. The
/// deterministic effect-delivery boundary reserves identities before any widget
/// is created. Clones share the same owner; they do not copy its counters.
#[derive(Clone)]
pub struct ModalEffectAdmission {
    state: Arc<Mutex<ModalSyncState>>,
    frame: FrameCursor,
}

impl ModalEffectAdmission {
    pub fn discard_pending(&self) -> Result<(), String> {
        let mut sync = self.state.lock().map_err(|_| "modal state lock poisoned")?;
        for state in &mut sync.occurrences {
            state.pending.clear();
        }
        sync.emitted.clear();
        Ok(())
    }

    pub fn admit(&self, kinds: &[ModalKind]) -> Result<(), String> {
        let mut sync = self.state.lock().map_err(|_| "modal state lock poisoned")?;
        let frame = self.frame.load(Ordering::Relaxed);
        Self::admit_at(&mut sync, frame, kinds)
    }

    fn admit_at(sync: &mut ModalSyncState, frame: u32, kinds: &[ModalKind]) -> Result<(), String> {
        let session_id = sync.session_id.ok_or("modal session is not installed")?;
        for kind in kinds.iter().filter(|kind| is_shared_story_modal(kind)) {
            if !sync.occurrences.iter().any(|state| state.kind == *kind) {
                sync.occurrences.push(ModalOccurrenceState {
                    kind: kind.clone(),
                    next_occurrence: 0,
                    active: None,
                    pending: Default::default(),
                });
            }
            let state = sync
                .occurrences
                .iter_mut()
                .find(|state| state.kind == *kind)
                .unwrap();
            state.next_occurrence = state
                .next_occurrence
                .checked_add(1)
                .ok_or("modal occurrence overflow")?;
            state.pending.push_back(ModalInstanceId {
                session_id,
                opened_frame: frame,
                occurrence: state.next_occurrence,
            });
            sync.emitted.entry(frame).or_default().push(kind.clone());
        }
        Ok(())
    }

    /// Rebuilding engine history must not reopen already-admitted stories.
    /// Match occurrences across the corrected interval, not just within a
    /// frame: a late input can move the same script event to an earlier tick.
    /// Already-presented effects remain committed session facts. Recover only
    /// additional occurrences, preserving multiplicity for repeated text ids.
    pub fn reconcile_frames(
        &self,
        start: u32,
        end: u32,
        frames: &[(u32, Vec<ModalKind>)],
    ) -> Result<Vec<ModalKind>, String> {
        let mut sync = self.state.lock().map_err(|_| "modal state lock poisoned")?;
        if start > end {
            return Err("reversed modal reconciliation interval".into());
        }
        let mut previous: Vec<_> = sync
            .emitted
            .range(start..end)
            .flat_map(|(frame, kinds)| kinds.iter().cloned().map(move |kind| (*frame, kind)))
            .collect();
        let mut recovered = Vec::new();
        for (frame, kinds) in frames
            .iter()
            .filter(|(frame, _)| *frame >= start && *frame < end)
        {
            for kind in kinds.iter().filter(|kind| is_shared_story_modal(kind)) {
                if let Some(index) = previous.iter().position(|(_, old)| old == kind) {
                    let (old_frame, _) = previous.remove(index);
                    if old_frame != *frame {
                        let old = sync
                            .emitted
                            .get_mut(&old_frame)
                            .expect("matched effect frame");
                        let index = old
                            .iter()
                            .position(|old| old == kind)
                            .expect("matched effect occurrence");
                        old.remove(index);
                        sync.emitted.entry(*frame).or_default().push(kind.clone());
                    }
                } else {
                    Self::admit_at(&mut sync, *frame, std::slice::from_ref(kind))?;
                    recovered.push(kind.clone());
                }
            }
        }
        Ok(recovered)
    }
}

/// Transport recovery view of session control, delivered after the engine
/// snapshot on reconnect. Existing wire messages carry the state so save and
/// replay encodings remain unchanged. Keep the latest completed occurrence per
/// kind as a tombstone for a client whose old surface is still open.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ModalRecoveryState {
    active: Vec<ModalProgress>,
    completed: Vec<ModalDecision>,
}

impl ModalRecoveryState {
    pub fn observe_progress(&mut self, progress: ModalProgress) {
        if self.completed.iter().any(|decision| {
            decision.kind == progress.kind
                && decision.instance.occurrence >= progress.instance.occurrence
        }) {
            return;
        }
        if let Some(old) = self
            .active
            .iter_mut()
            .find(|old| old.kind == progress.kind && old.instance == progress.instance)
        {
            for (accepted, incoming) in old.accepted.iter_mut().zip(progress.accepted) {
                *accepted |= incoming;
            }
            old.player_names = progress.player_names;
        } else {
            self.active.push(progress);
        }
    }

    pub fn observe_decision(&mut self, decision: ModalDecision) {
        self.active
            .retain(|old| old.kind != decision.kind || old.instance != decision.instance);
        if self.completed.iter().any(|old| {
            old.kind == decision.kind && old.instance.occurrence >= decision.instance.occurrence
        }) {
            return;
        }
        self.completed.retain(|old| old.kind != decision.kind);
        self.completed.push(decision);
    }

    pub fn messages(&self) -> Vec<NetMsg> {
        self.completed
            .iter()
            .cloned()
            .map(NetMsg::ModalDecision)
            .chain(self.active.iter().cloned().map(NetMsg::ModalProgress))
            .collect()
    }
}

impl NetChannels {
    pub fn modal_effect_admission(&self) -> ModalEffectAdmission {
        ModalEffectAdmission {
            state: self.modal_sync.clone(),
            frame: self.frame_cursor.clone(),
        }
    }

    /// Submit a local UI/replay-adapter outcome to the session owner. A local
    /// click acknowledges a story; it cannot bypass the admitted roster.
    pub fn submit_modal_outcome(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
        result: DialogResult,
        is_host: bool,
    ) -> Result<ModalPublication, String> {
        if !is_host {
            self.propose_modal_dismiss(instance, kind.clone(), result)?;
            return Ok(ModalPublication::ClientProposalQueued);
        }
        if is_shared_story_modal(kind) {
            self.record_modal_vote(instance, kind, PlayerId::HOST, result)?;
            if self.resolve_modal_consensus(instance, kind)?.is_none() {
                self.publish_modal_progress(instance, kind)?;
                return Ok(ModalPublication::HostVoteQueued);
            }
        } else {
            self.decide_modal_dismiss(instance, kind.clone(), result)?;
        }
        Ok(ModalPublication::HostDecisionQueued)
    }

    /// Process control traffic independently of any modal widget. This is also
    /// callable from blocking menu adapters; unmatched simulation events remain
    /// available to the normal session drain.
    pub fn service_modal_session(&self, is_host: bool) -> Result<(), String> {
        let mut retained = Vec::new();
        let mut other = Vec::new();
        while let Ok(event) = self.try_recv_modal_event() {
            match event {
                NetEvent::ModalProposal { from, proposal } if is_host => {
                    if proposal.instance.session_id != self.session_id()? {
                        return Err("modal acknowledgement belongs to another session".into());
                    }
                    if self
                        .modal_decision(proposal.instance, &proposal.kind)?
                        .is_some()
                    {
                        continue; // An acknowledgement retransmitted after completion.
                    }
                    if !self.modal_instance_is_active(proposal.instance, &proposal.kind)? {
                        if !is_shared_story_modal(&proposal.kind) {
                            retained.push(NetEvent::ModalProposal { from, proposal });
                            continue;
                        }
                        return Err(
                            "modal acknowledgement does not name an admitted instance".into()
                        );
                    }
                    if is_shared_story_modal(&proposal.kind) {
                        self.record_modal_vote(
                            proposal.instance,
                            &proposal.kind,
                            from,
                            proposal.result,
                        )?;
                        if self
                            .unanimous_modal_result(proposal.instance, &proposal.kind)?
                            .is_none()
                        {
                            if let Err(error) =
                                self.publish_modal_progress(proposal.instance, &proposal.kind)
                            {
                                tracing::error!(%error, "modal progress publication failed; retaining acknowledgement");
                            }
                        }
                    }
                    self.record_visible_modal_request(VisibleModalRequest {
                        from,
                        instance: proposal.instance,
                        kind: proposal.kind,
                        result: proposal.result,
                        requested_frame: proposal.requested_frame,
                    })?;
                }
                NetEvent::ModalDecision(decision) => {
                    if is_host {
                        return Err("host received a remote authoritative modal decision".into());
                    }
                    self.retain_modal_decision(decision)?;
                }
                NetEvent::ModalProgress(progress) => {
                    if progress.instance.session_id != self.session_id()? {
                        return Err("modal progress belongs to another session".into());
                    }
                    if is_host {
                        // The transport supplies authenticated roster names.
                        self.set_modal_player_names(progress.player_names);
                    } else if self
                        .modal_decision(progress.instance, &progress.kind)?
                        .is_none()
                    {
                        self.apply_modal_progress(&progress)?;
                        // Admission consumes this opening once its boundary is reached.
                        if !self.modal_instance_is_active(progress.instance, &progress.kind)? {
                            let mut sync = self
                                .modal_sync
                                .lock()
                                .map_err(|_| "modal state lock poisoned")?;
                            sync.pending_openings.retain(|old| {
                                old.instance != progress.instance || old.kind != progress.kind
                            });
                            sync.pending_openings.push(progress);
                        }
                    }
                }
                event @ NetEvent::ModalProposal { .. } => retained.push(event),
                event => other.push(event),
            }
        }
        for event in retained {
            self.defer_modal_event(event)?;
        }
        self.defer_events(other);
        if is_host {
            let active: Vec<_> = self
                .modal_sync
                .lock()
                .map_err(|_| "modal state lock poisoned")?
                .occurrences
                .iter()
                .filter_map(|state| state.active.map(|id| (id, state.kind.clone())))
                .collect();
            for (instance, kind) in active {
                if is_shared_story_modal(&kind) {
                    if let Err(error) = self.resolve_modal_consensus(instance, &kind) {
                        tracing::error!(%error, "modal decision publication failed; retaining consensus for retry");
                    }
                }
            }
        }
        Ok(())
    }

    fn modal_instance_is_active(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
    ) -> Result<bool, String> {
        Ok(self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?
            .occurrences
            .iter()
            .any(|state| state.kind == *kind && state.active == Some(instance)))
    }

    /// Decisions outlive the surface that displays them, so an early close or a
    /// reconnect cannot lose the result merely because no widget was polling.
    pub fn modal_decision(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
    ) -> Result<Option<DialogResult>, String> {
        let sync = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?;
        Ok(sync
            .decisions
            .iter()
            .find(|decision| decision.instance == instance && decision.kind == *kind)
            .filter(|decision| {
                is_shared_story_modal(kind) || decision.decision_frame <= self.current_frame()
            })
            .map(|decision| decision.result))
    }

    fn retain_modal_decision(&self, decision: ModalDecision) -> Result<(), String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?;
        if Some(decision.instance.session_id) != sync.session_id
            || decision.decision_frame < decision.instance.opened_frame
        {
            return Err("invalid modal decision session or boundary".into());
        }
        if let Some(previous) = sync.decisions.iter().find(|previous| {
            previous.instance == decision.instance && previous.kind == decision.kind
        }) {
            if previous != &decision {
                return Err("conflicting authoritative modal decisions".into());
            }
            return Ok(());
        }
        sync.local_proposals.retain(|proposal| {
            proposal.instance != decision.instance || proposal.kind != decision.kind
        });
        sync.decisions.push(decision);
        Ok(())
    }

    pub fn resolve_modal_consensus(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
    ) -> Result<Option<DialogResult>, String> {
        if let Some(result) = self.modal_decision(instance, kind)? {
            return Ok(Some(result));
        }
        let Some(result) = self.unanimous_modal_result(instance, kind)? else {
            return Ok(None);
        };
        self.decide_modal_dismiss(instance, kind.clone(), result)?;
        Ok(Some(result))
    }

    /// Session scheduler transition from pending to presented. The widget only
    /// reads this binding and never creates an occurrence or sends its opening.
    pub fn present_modal(
        &self,
        kind: &ModalKind,
        is_host: bool,
    ) -> Result<ModalInstanceId, String> {
        let instance = self.open_modal_instance(kind)?;
        if is_host && is_shared_story_modal(kind) {
            self.announce_modal_instance(instance, kind)?;
        }
        Ok(instance)
    }

    pub fn current_modal_instance(&self, kind: &ModalKind) -> Result<ModalInstanceId, String> {
        self.modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?
            .occurrences
            .iter()
            .find(|state| state.kind == *kind)
            .and_then(|state| state.active)
            .ok_or_else(|| format!("no presented modal for {kind:?}"))
    }

    /// A batch may omit an unavailable page or abort its remaining pages.
    /// Retire exactly one unpresented reservation, leaving later batches intact.
    pub fn discard_pending_modal(&self, kind: &ModalKind) -> Result<(), String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?;
        let state = sync
            .occurrences
            .iter_mut()
            .find(|state| state.kind == *kind)
            .ok_or_else(|| format!("no reserved story for {kind:?}"))?;
        state
            .pending
            .pop_front()
            .ok_or_else(|| format!("no pending story for {kind:?}"))?;
        Ok(())
    }

    /// Bind a presentation to an admitted story occurrence. Only non-story
    /// choice screens allocate here; story identities belong to effect delivery.
    pub fn open_modal_instance(&self, kind: &ModalKind) -> Result<ModalInstanceId, String> {
        let opened_frame = self.frame_cursor.load(Ordering::Relaxed);
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let session_id = sync
            .session_id
            .ok_or_else(|| "multiplayer session identity is not installed".to_string())?;
        if let Some(state) = sync
            .occurrences
            .iter_mut()
            .find(|state| state.kind == *kind)
        {
            if let Some(instance) = state.active {
                return Ok(instance);
            }
            if let Some(instance) = state.pending.pop_front() {
                state.active = Some(instance);
                return Ok(instance);
            }
            if is_shared_story_modal(kind) {
                return Err(format!("story widget has no admitted event for {kind:?}"));
            }
            state.next_occurrence = state
                .next_occurrence
                .checked_add(1)
                .ok_or_else(|| "multiplayer modal occurrence counter overflowed".to_string())?;
            let instance = ModalInstanceId {
                session_id,
                opened_frame,
                occurrence: state.next_occurrence,
            };
            state.active = Some(instance);
            return Ok(instance);
        }
        if is_shared_story_modal(kind) {
            return Err(format!("story widget has no admitted event for {kind:?}"));
        }
        let instance = ModalInstanceId {
            session_id,
            opened_frame,
            occurrence: 1,
        };
        sync.occurrences.push(ModalOccurrenceState {
            kind: kind.clone(),
            next_occurrence: 1,
            active: Some(instance),
            pending: Default::default(),
        });
        Ok(instance)
    }

    pub fn complete_modal_instance(
        &self,
        kind: &ModalKind,
        instance: ModalInstanceId,
    ) -> Result<(), String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let decided = sync
            .decisions
            .iter()
            .any(|decision| decision.instance == instance && decision.kind == *kind);
        let state = sync
            .occurrences
            .iter_mut()
            .find(|state| state.kind == *kind)
            .ok_or_else(|| format!("no multiplayer modal occurrence exists for {kind:?}"))?;
        if state.active.is_none() && state.next_occurrence == instance.occurrence {
            return Ok(());
        }
        if state.active != Some(instance) {
            if decided {
                return Ok(());
            }
            return Err(format!(
                "multiplayer modal completion mismatch for {kind:?}: active={:?}, completed={instance:?}",
                state.active
            ));
        }
        state.active = None;
        sync.votes
            .retain(|vote| vote.instance != instance || vote.kind != *kind);
        Ok(())
    }

    /// Route a modal event out of the ordinary simulation drain and into the
    /// presentation-side modal inbox.
    pub fn defer_modal_event(&self, event: NetEvent) -> Result<(), String> {
        if !matches!(
            event,
            NetEvent::ModalProposal { .. }
                | NetEvent::ModalDecision { .. }
                | NetEvent::ModalProgress(_)
        ) {
            return Err("attempted to route a non-modal event into the modal inbox".to_string());
        }
        self.modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?
            .inbox
            .push_back(event);
        Ok(())
    }

    pub fn try_recv_modal_event(&self) -> Result<NetEvent, std::sync::mpsc::TryRecvError> {
        if self.command_worker_closed.load(Ordering::Acquire) {
            return Err(std::sync::mpsc::TryRecvError::Disconnected);
        }
        if let Some(event) = self
            .modal_sync
            .lock()
            .expect("multiplayer modal event queue poisoned")
            .inbox
            .pop_front()
        {
            return Ok(event);
        }
        self.try_recv_transport_event()
    }

    /// The admitted lobby roster must acknowledge story pages before the host closes them.
    pub fn set_modal_player_count(&self, count: u8) {
        assert!((1..=crate::coop::MAX_PLAYERS as u8).contains(&count));
        self.modal_sync
            .lock()
            .expect("modal state lock poisoned")
            .required_players = count;
    }

    pub fn record_modal_vote(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
        seat: PlayerId,
        result: DialogResult,
    ) -> Result<(), String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?;
        let count = sync.required_players.max(1);
        if seat.0 >= count {
            return Err(format!(
                "modal vote from seat {seat:?} outside admitted roster"
            ));
        }
        if !sync
            .votes
            .iter()
            .any(|vote| vote.instance == instance && vote.kind == *kind)
        {
            sync.votes.push(ModalVotes {
                instance,
                kind: kind.clone(),
                seats: [false; crate::coop::MAX_PLAYERS],
                host_result: None,
            });
        }
        let vote = sync
            .votes
            .iter_mut()
            .find(|vote| vote.instance == instance && vote.kind == *kind)
            .unwrap();
        vote.seats[seat.0 as usize] = true;
        if seat == PlayerId::HOST {
            vote.host_result = Some(result);
        }
        Ok(())
    }

    pub fn announce_modal_instance(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
    ) -> Result<(), String> {
        {
            let sync = self
                .modal_sync
                .lock()
                .map_err(|_| "modal state lock poisoned")?;
            if sync.required_players <= 1 || sync.announced.contains(&(instance, kind.clone())) {
                return Ok(());
            }
        }
        self.publish_modal_progress(instance, kind)?;
        tracing::info!(
            ?instance,
            ?kind,
            "multiplayer: announced story modal opening"
        );
        self.modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?
            .announced
            .push((instance, kind.clone()));
        Ok(())
    }

    /// Admit host-announced story UI once the client reaches its boundary.
    /// The engine can have crossed that boundary during silent reconstruction;
    /// the host's token, rather than the client's current frame, identifies it.
    pub fn take_ready_story_announcements(&self, frame: u32) -> Result<Vec<ModalKind>, String> {
        self.service_modal_session(false)?;
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?;
        let mut ready = Vec::new();
        let mut retained = Vec::new();
        for progress in std::mem::take(&mut sync.pending_openings) {
            if sync.decisions.iter().any(|decision| {
                decision.instance == progress.instance && decision.kind == progress.kind
            }) {
                continue;
            }
            if progress.instance.opened_frame > frame {
                retained.push(progress);
                continue;
            }
            match sync
                .occurrences
                .iter_mut()
                .find(|state| state.kind == progress.kind)
            {
                Some(state) if progress.instance.occurrence <= state.next_occurrence => continue,
                Some(state) if state.active.is_some() => {
                    retained.push(progress);
                    continue;
                }
                Some(state) => {
                    state.next_occurrence = progress.instance.occurrence;
                    state.active = Some(progress.instance);
                }
                None => sync.occurrences.push(ModalOccurrenceState {
                    kind: progress.kind.clone(),
                    next_occurrence: progress.instance.occurrence,
                    active: Some(progress.instance),
                    pending: Default::default(),
                }),
            }
            ready.push(progress.kind);
        }
        sync.pending_openings = retained;
        Ok(ready)
    }

    /// The session owns the pause barrier even before a UI surface is drawn.
    /// Control messages continue to be serviced while this holds simulation.
    pub fn story_barrier_pending(&self) -> bool {
        let sync = self.modal_sync.lock().expect("modal state lock poisoned");
        sync.occurrences.iter().any(|state| {
            is_shared_story_modal(&state.kind)
                && state.active.is_some_and(|instance| {
                    !sync.decisions.iter().any(|decision| {
                        decision.instance == instance && decision.kind == state.kind
                    })
                })
        })
    }

    pub fn set_modal_player_names(&self, names: Vec<String>) {
        self.modal_sync
            .lock()
            .expect("modal state lock poisoned")
            .player_names = names;
    }

    pub fn modal_waiting_names(&self, instance: ModalInstanceId, kind: &ModalKind) -> Vec<String> {
        let sync = self.modal_sync.lock().expect("modal state lock poisoned");
        let vote = sync
            .votes
            .iter()
            .find(|vote| vote.instance == instance && vote.kind == *kind);
        (0..sync.required_players.max(1) as usize)
            .filter(|&i| !vote.is_some_and(|vote| vote.seats[i]))
            .map(|i| {
                sync.player_names
                    .get(i)
                    .filter(|name| !name.is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("Player {}", i + 1))
            })
            .collect()
    }

    pub fn publish_modal_progress(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
    ) -> Result<(), String> {
        let sync = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?;
        let accepted = sync
            .votes
            .iter()
            .find(|vote| vote.instance == instance && vote.kind == *kind)
            .map(|vote| vote.seats)
            .unwrap_or([false; crate::coop::MAX_PLAYERS]);
        self.outgoing
            .send(NetOutbound::ModalProgress(ModalProgress {
                instance,
                kind: kind.clone(),
                accepted,
                player_names: sync.player_names.clone(),
            }))
            .map_err(|error| error.to_string())
    }

    pub fn apply_modal_progress(&self, progress: &ModalProgress) -> Result<(), String> {
        self.set_modal_player_names(progress.player_names.clone());
        for (seat, accepted) in progress.accepted.iter().enumerate() {
            if *accepted {
                self.record_modal_vote(
                    progress.instance,
                    &progress.kind,
                    PlayerId(seat as u8),
                    DialogResult::Completed,
                )?;
            }
        }
        Ok(())
    }

    pub fn unanimous_modal_result(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
    ) -> Result<Option<DialogResult>, String> {
        let sync = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?;
        let count = sync.required_players.max(1) as usize;
        Ok(sync
            .votes
            .iter()
            .find(|vote| vote.instance == instance && vote.kind == *kind)
            .filter(|vote| vote.seats[..count].iter().all(|ready| *ready))
            .and_then(|vote| vote.host_result))
    }

    pub fn record_visible_modal_request(&self, request: VisibleModalRequest) -> Result<(), String> {
        self.modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?
            .visible_requests
            .push_back(request);
        Ok(())
    }

    pub fn take_visible_modal_requests(
        &self,
        instance: ModalInstanceId,
        kind: &ModalKind,
    ) -> Result<Vec<VisibleModalRequest>, String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        let mut matched = Vec::new();
        let mut retained = std::collections::VecDeque::new();
        while let Some(request) = sync.visible_requests.pop_front() {
            if request.instance == instance && request.kind == *kind {
                matched.push(request);
            } else {
                retained.push_back(request);
            }
        }
        sync.visible_requests = retained;
        Ok(matched)
    }

    pub fn take_all_visible_modal_requests(&self) -> Result<Vec<VisibleModalRequest>, String> {
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "multiplayer modal state lock is poisoned".to_string())?;
        Ok(sync.visible_requests.drain(..).collect())
    }

    /// Submit a visible client request without changing local modal state.
    /// Channel closure is an authoritative session failure and is propagated.
    pub fn propose_modal_dismiss(
        &self,
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
    ) -> Result<(), String> {
        let proposal = ModalProposal {
            instance,
            kind,
            result,
            requested_frame: self.current_frame(),
        };
        self.outgoing
            .send(NetOutbound::ModalProposal(proposal.clone()))
            .map_err(|_| "multiplayer modal proposal channel is closed".to_string())?;
        let mut sync = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?;
        if !sync
            .local_proposals
            .iter()
            .any(|old| old.instance == proposal.instance && old.kind == proposal.kind)
        {
            sync.local_proposals.push(proposal);
        }
        Ok(())
    }

    /// A queued acknowledgement can be lost with its socket. Re-send the exact
    /// retained instance after reconnect, without asking the player to click a
    /// second time. Host vote admission and completed decisions are idempotent.
    pub fn resend_pending_modal_proposals(&self) -> Result<(), String> {
        let proposals = self
            .modal_sync
            .lock()
            .map_err(|_| "modal state lock poisoned")?
            .local_proposals
            .clone();
        for proposal in proposals {
            self.outgoing
                .send(NetOutbound::ModalProposal(proposal))
                .map_err(|_| "multiplayer modal proposal channel is closed".to_string())?;
        }
        Ok(())
    }

    /// Publish the host's sole authoritative result for an exact modal.
    /// Channel closure is returned so the caller keeps the modal open instead
    /// of applying a local-only result.
    pub fn decide_modal_dismiss(
        &self,
        instance: ModalInstanceId,
        kind: ModalKind,
        result: DialogResult,
    ) -> Result<(), String> {
        if self.modal_decision(instance, &kind)?.is_some() {
            return Ok(());
        }
        let decision = ModalDecision {
            instance,
            kind,
            result,
            decision_frame: self.current_frame(),
        };
        self.outgoing
            .send(NetOutbound::ModalDecision(decision.clone()))
            .map_err(|_| "multiplayer modal decision channel is closed".to_string())?;
        self.retain_modal_decision(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (NetChannels, Sender<NetEvent>, Receiver<NetOutbound>) {
        let (net, input, output, _, _) = NetChannels::new();
        net.install_session_id(MultiplayerSessionId([7; 32]))
            .unwrap();
        net.set_modal_player_count(2);
        (net, input, output)
    }

    fn story() -> ModalKind {
        ModalKind::PopupText { text_id: 17 }
    }

    fn forward(output: &Receiver<NetOutbound>, input: &Sender<NetEvent>, from: PlayerId) {
        for message in output.try_iter() {
            input
                .send(match message {
                    NetOutbound::ModalProgress(progress) => NetEvent::ModalProgress(progress),
                    NetOutbound::ModalDecision(decision) => NetEvent::ModalDecision(decision),
                    NetOutbound::ModalProposal(proposal) => {
                        NetEvent::ModalProposal { from, proposal }
                    }
                    other => panic!("unexpected session output {other:?}"),
                })
                .unwrap();
        }
    }

    #[test]
    fn effect_identity_precedes_rendering_and_repeated_kinds_remain_distinct() {
        let (host, _, _) = fixture();
        host.publish_frame(12);
        host.modal_effect_admission()
            .admit(&[story(), story()])
            .unwrap();
        host.publish_frame(90);
        let first = host.open_modal_instance(&story()).unwrap();
        assert_eq!(first.opened_frame, 12);
        assert_eq!(host.open_modal_instance(&story()).unwrap(), first);
        host.complete_modal_instance(&story(), first).unwrap();
        let second = host.open_modal_instance(&story()).unwrap();
        assert_eq!(second.opened_frame, 12);
        assert_eq!(second.occurrence, first.occurrence + 1);
        host.complete_modal_instance(&story(), second).unwrap();
        assert!(
            host.open_modal_instance(&story()).is_err(),
            "widgets cannot invent story events"
        );
    }

    #[test]
    fn delayed_client_and_consecutive_scrolls_complete_without_widget_polling() {
        let (host, host_in, host_out) = fixture();
        let (client, client_in, client_out) = fixture();
        host.modal_effect_admission()
            .admit(&[story(), story()])
            .unwrap();
        for expected_occurrence in 1..=2 {
            let instance = host.present_modal(&story(), true).unwrap();
            assert_eq!(instance.occurrence, expected_occurrence);
            host.record_modal_vote(instance, &story(), PlayerId::HOST, DialogResult::Completed)
                .unwrap();
            host.publish_modal_progress(instance, &story()).unwrap();
            // Loading or a suspended process delays all delivery. The simulation
            // stays frozen; the session still processes its control messages.
            forward(&host_out, &client_in, PlayerId::HOST);
            client.service_modal_session(false).unwrap();
            assert_eq!(
                client.take_ready_story_announcements(0).unwrap(),
                vec![story()]
            );
            assert!(client.take_ready_story_announcements(0).unwrap().is_empty());
            assert_eq!(client.current_modal_instance(&story()).unwrap(), instance);
            assert!(
                client.story_barrier_pending(),
                "session holds before a widget exists"
            );
            client
                .propose_modal_dismiss(instance, story(), DialogResult::Completed)
                .unwrap();
            // Repeated delivery must not cause a second decision or next scroll.
            client
                .propose_modal_dismiss(instance, story(), DialogResult::Completed)
                .unwrap();
            forward(&client_out, &host_in, PlayerId(1));
            host.service_modal_session(true).unwrap();
            assert_eq!(
                host.modal_decision(instance, &story()).unwrap(),
                Some(DialogResult::Completed)
            );
            forward(&host_out, &client_in, PlayerId::HOST);
            client.service_modal_session(false).unwrap();
            assert_eq!(
                client.modal_decision(instance, &story()).unwrap(),
                Some(DialogResult::Completed)
            );
            assert!(!host.story_barrier_pending());
            assert!(!client.story_barrier_pending());
            host.complete_modal_instance(&story(), instance).unwrap();
            client.complete_modal_instance(&story(), instance).unwrap();
        }
        assert_eq!(host.current_frame(), 0);
        assert_eq!(client.current_frame(), 0);
        assert!(host_out.try_recv().is_err());
    }

    #[test]
    fn reconnect_recovers_opening_and_dismissal_without_reopening_completed_scroll() {
        let (host, _, output) = fixture();
        host.modal_effect_admission()
            .admit(&[story(), story()])
            .unwrap();
        let first = host.present_modal(&story(), true).unwrap();
        let NetOutbound::ModalProgress(opening) = output.try_recv().unwrap() else {
            panic!("opening");
        };
        let mut recovery = ModalRecoveryState::default();
        recovery.observe_progress(opening.clone());
        // A newly created client never saw the original one-shot announcement.
        let (client, client_in, _) = fixture();
        for msg in recovery.messages() {
            let NetMsg::ModalProgress(progress) = msg else {
                panic!("active progress");
            };
            client_in.send(NetEvent::ModalProgress(progress)).unwrap();
        }
        client.service_modal_session(false).unwrap();
        assert_eq!(
            client.take_ready_story_announcements(0).unwrap(),
            vec![story()]
        );
        host.decide_modal_dismiss(first, story(), DialogResult::Completed)
            .unwrap();
        let NetOutbound::ModalDecision(close) = output.try_recv().unwrap() else {
            panic!("decision");
        };
        recovery.observe_decision(close);
        host.complete_modal_instance(&story(), first).unwrap();
        let second = host.present_modal(&story(), true).unwrap();
        let NetOutbound::ModalProgress(next) = output.try_recv().unwrap() else {
            panic!("next opening");
        };
        recovery.observe_progress(next);
        recovery.observe_progress(opening); // late duplicate cannot resurrect it
        for msg in recovery.messages() {
            client_in
                .send(match msg {
                    NetMsg::ModalDecision(decision) => NetEvent::ModalDecision(decision),
                    NetMsg::ModalProgress(progress) => NetEvent::ModalProgress(progress),
                    _ => unreachable!(),
                })
                .unwrap();
        }
        client.service_modal_session(false).unwrap();
        assert_eq!(
            client.modal_decision(first, &story()).unwrap(),
            Some(DialogResult::Completed)
        );
        client.complete_modal_instance(&story(), first).unwrap();
        assert_eq!(
            client.take_ready_story_announcements(0).unwrap(),
            vec![story()]
        );
        assert_eq!(client.current_modal_instance(&story()).unwrap(), second);
        assert!(client.take_ready_story_announcements(0).unwrap().is_empty());
    }

    #[test]
    fn rollback_preserves_acknowledged_occurrences_and_recovers_new_effects_once() {
        let (host, _input, _output) = fixture();
        host.publish_frame(12);
        let admission = host.modal_effect_admission();
        admission.admit(&[story()]).unwrap();
        let first = host.present_modal(&story(), true).unwrap();
        host.decide_modal_dismiss(first, story(), DialogResult::Completed)
            .unwrap();
        host.complete_modal_instance(&story(), first).unwrap();
        host.publish_frame(30);
        assert!(
            admission
                .reconcile_frames(12, 13, &[(12, [story()].to_vec())])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            admission
                .reconcile_frames(12, 13, &[(12, [story(), story()].to_vec())])
                .unwrap(),
            vec![story()]
        );
        assert!(
            admission
                .reconcile_frames(12, 13, &[(12, [story(), story()].to_vec())])
                .unwrap()
                .is_empty()
        );
        let recovered = host.present_modal(&story(), true).unwrap();
        assert_eq!(recovered.opened_frame, 12);
        assert_eq!(recovered.occurrence, first.occurrence + 1);
        assert_eq!(
            host.modal_decision(first, &story()).unwrap(),
            Some(DialogResult::Completed)
        );
        assert_eq!(host.modal_decision(recovered, &story()).unwrap(), None);
    }

    #[test]
    fn discarding_a_batch_does_not_reuse_its_identity_for_a_later_batch() {
        let (host, _input, _output) = fixture();
        host.modal_effect_admission()
            .admit(&[story(), story(), story()])
            .unwrap();
        let first = host.present_modal(&story(), true).unwrap();
        host.decide_modal_dismiss(first, story(), DialogResult::Aborted)
            .unwrap();
        host.complete_modal_instance(&story(), first).unwrap();
        host.discard_pending_modal(&story()).unwrap();
        let future = host.present_modal(&story(), true).unwrap();
        assert_eq!(future.occurrence, 3);
        host.complete_modal_instance(&story(), first).unwrap();
        assert_eq!(host.current_modal_instance(&story()).unwrap(), future);
    }

    #[test]
    fn reconnect_retransmits_a_lost_acknowledgement_until_a_decision_is_received() {
        let (host, host_in, host_out) = fixture();
        let (client, client_in, client_out) = fixture();
        host.modal_effect_admission().admit(&[story()]).unwrap();
        let instance = host.present_modal(&story(), true).unwrap();
        host.record_modal_vote(instance, &story(), PlayerId::HOST, DialogResult::Completed)
            .unwrap();
        forward(&host_out, &client_in, PlayerId::HOST);
        assert_eq!(
            client.take_ready_story_announcements(0).unwrap(),
            vec![story()]
        );
        client
            .propose_modal_dismiss(instance, story(), DialogResult::Completed)
            .unwrap();
        let lost = client_out.try_recv().unwrap();
        assert!(matches!(lost, NetOutbound::ModalProposal(_)));
        assert!(host.story_barrier_pending());
        client.resend_pending_modal_proposals().unwrap();
        forward(&client_out, &host_in, PlayerId(1));
        host.service_modal_session(true).unwrap();
        forward(&host_out, &client_in, PlayerId::HOST);
        client.service_modal_session(false).unwrap();
        assert!(!host.story_barrier_pending());
        assert!(!client.story_barrier_pending());
        client.resend_pending_modal_proposals().unwrap();
        assert!(
            client_out.try_recv().is_err(),
            "resolved acknowledgements must not be retried"
        );
    }

    #[test]
    fn rollback_moving_a_story_to_another_tick_does_not_open_it_twice() {
        let (host, _input, _output) = fixture();
        host.publish_frame(18);
        let admission = host.modal_effect_admission();
        admission.admit(&[story()]).unwrap();
        let instance = host.present_modal(&story(), true).unwrap();
        host.decide_modal_dismiss(instance, story(), DialogResult::Completed)
            .unwrap();
        host.complete_modal_instance(&story(), instance).unwrap();
        assert!(
            admission
                .reconcile_frames(10, 30, &[(12, vec![story()])])
                .unwrap()
                .is_empty()
        );
        assert!(
            admission
                .reconcile_frames(10, 30, &[(25, vec![story()])])
                .unwrap()
                .is_empty()
        );
        assert!(
            admission
                .reconcile_frames(20, 30, &[(24, vec![story()])])
                .unwrap()
                .is_empty(),
            "subsequent rollback uses the corrected event position"
        );
        assert_eq!(
            admission
                .reconcile_frames(10, 30, &[(12, vec![story()]), (25, vec![story()])])
                .unwrap(),
            vec![story()]
        );
        let additional = host.present_modal(&story(), true).unwrap();
        assert_eq!(additional.opened_frame, 25);
        assert_eq!(additional.occurrence, instance.occurrence + 1);
    }

    #[test]
    fn early_decision_is_retained_and_conflicting_decisions_are_rejected() {
        let (client, incoming, _) = fixture();
        let decision = ModalDecision {
            instance: ModalInstanceId {
                session_id: client.session_id().unwrap(),
                opened_frame: 0,
                occurrence: 1,
            },
            kind: story(),
            result: DialogResult::Completed,
            decision_frame: 1,
        };
        incoming
            .send(NetEvent::ModalDecision(decision.clone()))
            .unwrap();
        incoming
            .send(NetEvent::ModalDecision(decision.clone()))
            .unwrap();
        client.service_modal_session(false).unwrap();
        assert_eq!(
            client.modal_decision(decision.instance, &story()).unwrap(),
            Some(DialogResult::Completed)
        );
        incoming
            .send(NetEvent::ModalDecision(ModalDecision {
                result: DialogResult::Aborted,
                ..decision
            }))
            .unwrap();
        assert!(
            client
                .service_modal_session(false)
                .unwrap_err()
                .contains("conflicting")
        );
    }
}
