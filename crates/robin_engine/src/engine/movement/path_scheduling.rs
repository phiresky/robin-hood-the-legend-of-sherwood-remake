//! Path request execution with only queue, graph, spatial, and sequence-read authority.

use super::{
    FailedPathRequest, PendingPathRequest, PendingPathRequestQueue, parity_path_request_state,
    retained_cancelled_path_result,
};
use crate::coordinates::MapPoint;

/// Disjoint owner borrows for the once-per-frame path scheduling barrier.
///
/// This context deliberately cannot reach scripts, campaign, player state, or
/// feedback. It only advances the pathfinder queue and classifies expired
/// failures; the root tick performs the cross-owner consequences (path order
/// installation and hero speech) immediately after each returned item.
pub(in crate::engine) struct PathScheduleContext<'a> {
    frame_counter: u32,
    entities: &'a crate::entities::Entities,
    fast_grid: &'a crate::fast_find_grid::FastFindGrid,
    pathfinder: &'a mut crate::pathfinder::PathFinder,
    pending_path_requests: &'a mut PendingPathRequestQueue,
    failed_path_requests: &'a mut Vec<FailedPathRequest>,
    sequence_manager: &'a crate::sequence::SequenceManager,
}

pub(in crate::engine) enum CompletedPathWork {
    Ready {
        request: PendingPathRequest,
        waypoints: Vec<MapPoint>,
    },
    Failed(PendingPathRequest),
}

pub(in crate::engine) struct ExpiredPathWork {
    pub(in crate::engine) request: FailedPathRequest,
    pub(in crate::engine) owner_is_pc: bool,
    pub(in crate::engine) age: u32,
}

impl<'a> PathScheduleContext<'a> {
    pub(in crate::engine) fn new(
        frame_counter: u32,
        entities: &'a crate::entities::Entities,
        fast_grid: &'a crate::fast_find_grid::FastFindGrid,
        pathfinder: &'a mut crate::pathfinder::PathFinder,
        pending_path_requests: &'a mut PendingPathRequestQueue,
        failed_path_requests: &'a mut Vec<FailedPathRequest>,
        sequence_manager: &'a crate::sequence::SequenceManager,
    ) -> Self {
        Self {
            frame_counter,
            entities,
            fast_grid,
            pathfinder,
            pending_path_requests,
            failed_path_requests,
            sequence_manager,
        }
    }

    /// Execute one path-request processing barrier.
    ///
    /// The pathfinder starts its successor before returning a completed head
    /// to the engine, including from the recursive synchronous `WAITING` branch
    /// in the original game. Keep completion
    /// classification and successor start inside one context borrow so result
    /// application cannot enqueue, cancel, or otherwise change which request
    /// becomes in-flight first.
    pub(in crate::engine) fn process_requests(
        &mut self,
        graph: &crate::pathfinder::PathGraph,
        synchronous_pathfinding: bool,
    ) -> Option<CompletedPathWork> {
        if self.pending_path_requests.has_in_flight() {
            let completed = self.take_completed();
            self.start_next(graph);
            return completed;
        }

        self.start_next(graph);
        if !synchronous_pathfinding {
            return None;
        }

        // Original's deterministic WAITING arm computes the first request,
        // recursively enters READY, starts/computes one successor, and only
        // then returns the first completion to the engine.
        let completed = self.take_completed();
        self.start_next(graph);
        completed
    }

    /// Take the one result made ready by the previous scheduling operation.
    /// Stale results are discarded without handing this barrier's completion
    /// slot to a later request.
    fn take_completed(&mut self) -> Option<CompletedPathWork> {
        let (processed, valid) = self.pending_path_requests.take_completed()?;
        let request = processed.request;
        if crate::pathfinder::parity_path_capture_is_active() {
            crate::pathfinder::record_parity_path_event(
                crate::pathfinder::ParityPathEvent::Completed {
                    request: parity_path_request_state(self.fast_grid, &request),
                    valid,
                    // Original records the raw path even when cancellation
                    // makes the delivery invalid. A failed A* request has an
                    // empty raw path but remains a valid delivery.
                    waypoints: processed.waypoints.clone().unwrap_or_default(),
                },
            );
        }
        if !valid {
            return None;
        }
        let still_live = self
            .sequence_manager
            .get_element(request.seq_id, request.elem_idx)
            .is_some_and(|elem| {
                elem.owner == Some(request.owner)
                    && elem.state == crate::sequence::SequenceState::InProgress
                    && elem.command == crate::element::Command::MoveWaiting
            });
        if !still_live {
            return None;
        }

        Some(match processed.waypoints {
            Some(waypoints) => CompletedPathWork::Ready { request, waypoints },
            None => CompletedPathWork::Failed(request),
        })
    }

    /// Start at most one queued request. Rust computes A* synchronously, but
    /// the result remains parked until the next scheduling operation consumes
    /// it (or the recursive deterministic `WAITING` arm above consumes it).
    fn start_next(&mut self, graph: &crate::pathfinder::PathGraph) {
        // Path processing never inspects the requesting
        // sequence element.
        // Every entry that is still in `mListPathRequests` is started and,
        // one call later, delivered — including entries whose element has
        // since been interrupted. Only explicit path cancellation removes an entry,
        // and it removes at most the first *later* request for the actor
        // while the logical head merely gets an ignore-next-path flag
        // in the original game. Skipping a request here
        // because its element died would hand the freed result slot to the
        // next queued request a frame early.
        let retained_cancelled_head = self.pending_path_requests.ignore_next_path;
        let Some(request) = self.pending_path_requests.pop_to_start() else {
            return;
        };
        // Original-game path-node search observes that flag and exits before
        // expanding its first node. The retained head therefore delivers an
        // invalid completion with an empty raw path; it must not calculate a
        // route merely because Rust runs pathfinding synchronously.
        let waypoints = retained_cancelled_path_result(retained_cancelled_head).or_else(|| {
            self.pathfinder.find_path(
                graph,
                self.fast_grid,
                request.layer,
                request.sector,
                request.half_diagonal_idx,
                request.source,
                request.dest,
                request.use_first_point,
            )
        });
        self.pending_path_requests.set_in_flight(request, waypoints);
    }

    /// Remove stale failures and return the next expired live entry.
    ///
    /// Returning one item at a time lets the root coordinator close hero
    /// speech, `element_impossible`, and the owner's synchronous condolation
    /// boundary before this method inspects the following entry, matching the
    /// mutable list walk used by the game.
    pub(in crate::engine) fn take_next_expired_failure(&mut self) -> Option<ExpiredPathWork> {
        let mut index = 0;
        while index < self.failed_path_requests.len() {
            let request = &self.failed_path_requests[index];
            let still_live = self
                .sequence_manager
                .get_element(request.seq_id, request.elem_idx)
                .is_some_and(|element| {
                    element.owner == Some(request.owner)
                        && element.state == crate::sequence::SequenceState::InProgress
                        && element.command == crate::element::Command::MoveWaiting
                });
            if !still_live {
                self.failed_path_requests.remove(index);
                continue;
            }

            let age = self.frame_counter.saturating_sub(request.first_fail_frame);
            if age <= 100 {
                index += 1;
                continue;
            }

            let owner_id = request.owner;
            let owner_is_pc = self
                .entities
                .get(owner_id)
                .unwrap_or_else(|| panic!(
                    "expired path request for {:?} retains a live sequence element but its owner entity is missing",
                    owner_id
                ))
                .is_pc();
            let request = self.failed_path_requests.remove(index);
            return Some(ExpiredPathWork {
                request,
                owner_is_pc,
                age,
            });
        }
        None
    }
}
