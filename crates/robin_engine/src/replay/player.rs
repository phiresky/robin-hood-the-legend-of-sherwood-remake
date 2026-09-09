use super::*;

/// Plays back a recorded replay, yielding complete inputs frame by frame.
pub struct ReplayPlayer {
    data: ReplayData,
    current_frame: u32,
}

impl ReplayPlayer {
    pub fn new(data: ReplayData) -> Self {
        Self {
            data,
            current_frame: 0,
        }
    }

    /// Header metadata (mission ID, seed, version).
    pub fn header(&self) -> &ReplayHeader {
        &self.data.header
    }

    /// Whether playback has reached the end.
    pub fn is_finished(&self) -> bool {
        self.current_frame >= self.data.frame_count()
    }

    /// Get the complete authoritative input for the current frame and advance.
    pub fn next_frame(&mut self) -> &ReplayFrame {
        let frame = self
            .data
            .frame(self.current_frame)
            .unwrap_or_else(|| panic!("replay frame {} is absent", self.current_frame));
        self.current_frame += 1;
        frame
    }

    /// Current frame index (before next_frame advances it).
    pub fn current_frame(&self) -> u32 {
        self.current_frame
    }

    /// Seek by dense replay host-frame ordinal.
    pub fn seek_ordinal(&mut self, ordinal: ReplayFrameOrdinal) {
        self.current_frame = ordinal.number().min(self.data.frame_count());
    }

    /// Seek to the first persisted host transaction at or after a lockstep
    /// boundary in the current linear load-back segment. This deliberately
    /// selects the first exact duplicate so meaningful skipped-hourglass
    /// admissions at that boundary are replayed without jumping onto an older
    /// pre-load branch.
    pub fn seek_timeline_frame(
        &mut self,
        timeline_frame: TimelineFrame,
    ) -> Result<ReplayFrameOrdinal, String> {
        let timeline_frame = timeline_frame.number();
        let segment_start = self
            .data
            .load_backs
            .range(..self.current_frame)
            .next_back()
            .map_or(0, |(&ordinal, _)| ordinal);
        let ordinal = self
            .data
            .frames
            .range(segment_start..self.current_frame)
            .find_map(|(&ordinal, frame)| {
                (frame.timeline_before == timeline_frame).then_some(ordinal)
            })
            .or_else(|| {
                self.data
                    .frames
                    .range(segment_start..self.current_frame)
                    .next_back()
                    .and_then(|(_, frame)| {
                        (frame.timeline_after == timeline_frame).then_some(self.current_frame)
                    })
            })
            .ok_or_else(|| {
                format!("replay has no host transaction at timeline frame {timeline_frame}")
            })?;
        self.current_frame = ordinal;
        Ok(ReplayFrameOrdinal::from_wire(ordinal))
    }

    /// Expected engine-state hash at the start of `frame`, if the
    /// recording carries one for that frame.
    pub fn hash_for_frame(&self, frame: u32) -> Option<u64> {
        self.data.hash_for_frame(frame)
    }

    /// Save marker at `frame`: the state hash an in-mission save captured
    /// at that frame's pre-command boundary, if one was recorded.
    pub fn save_marker_for_frame(&self, frame: u32) -> Option<ReplaySaveMarker> {
        self.data.save_marker_for_frame(frame)
    }

    /// Load-back at `frame`: the earlier save-marker frame whose captured
    /// state replaced the engine at that frame's boundary, if recorded.
    pub fn load_back_for_frame(&self, frame: u32) -> Option<&ReplayLoadBack> {
        self.data.load_back_for_frame(frame)
    }

    /// Total frames in the replay.
    pub fn total_frames(&self) -> u32 {
        self.data.frame_count()
    }
}
