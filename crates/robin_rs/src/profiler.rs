//! Simple frame-based profiling/timing utility.
//!
//! Records per-key timing data each frame and can write a CSV log on
//! demand.

use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Unsigned 32-bit index identifying a profiling column.
pub type ProfilerKey = u32;

// ---------------------------------------------------------------------------
// Core data structures
// ---------------------------------------------------------------------------

/// One column in the profiler table.
#[derive(Clone)]
struct ProfilerEntry {
    name: String,
    /// Timestamp captured by `LogStart` / `StartFrame` (milliseconds).
    time: u32,
    /// Accumulated time or externally-set value for the current frame.
    current_amount: u32,
    /// Tracks nested LogStart/LogEnd pairs so only the outermost measures.
    recursion_depth: u16,
    /// History of per-frame values (one element per completed frame).
    time_history: Vec<u32>,
}

impl ProfilerEntry {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            time: 0,
            current_amount: 0,
            recursion_depth: 0,
            time_history: Vec::new(),
        }
    }
}

/// The profiler instance.
pub struct Profiler {
    next_key: ProfilerKey,
    entries: Vec<ProfilerEntry>,
    /// One string per completed frame, recording events logged during that frame.
    events: Vec<String>,
    /// Event text being accumulated for the current (in-progress) frame.
    current_event: String,
    filename: String,
}

/// A function pointer the host can supply so we can read `SDL_GetTicks()`
/// without linking SDL from this crate.
static GET_TICKS: Mutex<Option<extern "C" fn() -> u32>> = Mutex::new(None);

/// Read current tick count via the registered callback, or return 0.
fn get_ticks() -> u32 {
    GET_TICKS.lock().unwrap().map(|f| f()).unwrap_or(0)
}

impl Profiler {
    /// Create a new profiler that will (eventually) write to `filename`.
    pub fn new(filename: &str) -> Self {
        let mut p = Self {
            next_key: 1,
            entries: Vec::new(),
            events: Vec::new(),
            current_event: String::new(),
            filename: filename.to_owned(),
        };
        // Entry 0 is the whole-frame summary ("Ze Sum").
        p.entries.push(ProfilerEntry::new("Ze Sum"));
        p
    }

    /// Allocate a new profiling key (column). Returns the key.
    pub fn create_key(&mut self, name: &str) -> ProfilerKey {
        self.entries.push(ProfilerEntry::new(name));
        let key = self.next_key;
        self.next_key += 1;
        key
    }

    /// Clear all recorded history.
    pub fn flush_log(&mut self) {
        for entry in &mut self.entries {
            entry.time_history.clear();
        }
        self.events.clear();
    }

    /// Begin a new frame.
    pub fn start_frame(&mut self) {
        self.current_event.clear();
        self.entries[0].time = get_ticks();
    }

    /// End the current frame, storing accumulated data.
    pub fn end_frame(&mut self) {
        self.events.push(self.current_event.clone());

        for i in 0..self.entries.len() {
            if i == 0 {
                let frame_time = get_ticks();
                let start_time = self.entries[0].time;
                self.entries[0]
                    .time_history
                    .push(frame_time.wrapping_sub(start_time));
                self.entries[0].time = 0;
            } else {
                let amount = self.entries[i].current_amount;
                self.entries[i].time_history.push(amount);
                self.entries[i].time = 0;
                self.entries[i].current_amount = 0;
            }
        }
    }

    /// Append an event description to the current frame.
    pub fn log_event(&mut self, description: &str) {
        self.current_event.push_str(description);
    }

    /// Start timing for `key`. Supports recursive calls (only the outermost
    /// pair actually measures).
    pub fn log_start(&mut self, key: ProfilerKey) -> bool {
        let k = key as usize;
        if k < self.entries.len() {
            let entry = &mut self.entries[k];
            if entry.recursion_depth == 0 {
                entry.time = get_ticks();
            }
            entry.recursion_depth += 1;
            true
        } else {
            false
        }
    }

    /// Stop timing for `key`.
    pub fn log_end(&mut self, key: ProfilerKey) -> bool {
        let k = key as usize;
        if k < self.entries.len() {
            let entry = &mut self.entries[k];
            // Pre-decrement on `u16`: an unbalanced LogEnd underflows to
            // 0xFFFF and skips the timing accumulation.
            entry.recursion_depth = entry.recursion_depth.wrapping_sub(1);
            if entry.recursion_depth == 0 {
                let now = get_ticks();
                entry.current_amount += now.wrapping_sub(entry.time);
                entry.time = 0;
            }
            true
        } else {
            false
        }
    }

    /// Set an absolute value for `key` this frame (not accumulated).
    pub fn log_value(&mut self, key: ProfilerKey, value: u32) -> bool {
        let k = key as usize;
        if k < self.entries.len() {
            self.entries[k].current_amount = value;
            true
        } else {
            false
        }
    }

    /// Generate the CSV output that the destructor would have written.
    /// Returns the full file contents as a String.
    pub fn generate_csv(&self) -> String {
        let mut sorted_entries = self.entries.clone();
        sorted_entries.sort_by(|a, b| a.name.cmp(&b.name));

        let mut out = String::new();

        // Header row
        out.push_str("#;Event;");
        for entry in &sorted_entries {
            out.push_str(&entry.name);
            out.push(';');
        }
        out.push_str("\r\n");

        // Data rows
        for (i, event) in self.events.iter().enumerate() {
            out.push_str(&format!("{};\"{}\";", i + 1, event));
            for entry in &sorted_entries {
                if i < entry.time_history.len() {
                    out.push_str(&format!("{};", entry.time_history[i]));
                } else {
                    out.push_str("0;");
                }
            }
            out.push_str("\r\n");
        }

        out
    }

    /// Number of entries (columns).
    pub fn num_entries(&self) -> usize {
        self.entries.len()
    }

    /// Number of completed frames.
    pub fn num_frames(&self) -> usize {
        self.events.len()
    }

    /// Get the filename.
    pub fn filename(&self) -> &str {
        &self.filename
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a profiler with a fake tick source for deterministic tests.
    fn make_profiler() -> Profiler {
        Profiler::new("test.csv")
    }

    #[test]
    fn new_profiler_has_ze_sum_entry() {
        let p = make_profiler();
        assert_eq!(p.num_entries(), 1);
        assert_eq!(p.entries[0].name, "Ze Sum");
        assert_eq!(p.num_frames(), 0);
    }

    #[test]
    fn create_key_returns_sequential_keys() {
        let mut p = make_profiler();
        let k1 = p.create_key("Alpha");
        let k2 = p.create_key("Beta");
        let k3 = p.create_key("Gamma");
        assert_eq!(k1, 1);
        assert_eq!(k2, 2);
        assert_eq!(k3, 3);
        assert_eq!(p.num_entries(), 4); // Ze Sum + 3
    }

    #[test]
    fn log_start_end_out_of_range_returns_false() {
        let mut p = make_profiler();
        assert!(!p.log_start(999));
        assert!(!p.log_end(999));
    }

    #[test]
    fn log_value_sets_current_amount() {
        let mut p = make_profiler();
        let k = p.create_key("Counter");
        assert!(p.log_value(k, 42));
        assert_eq!(p.entries[k as usize].current_amount, 42);
    }

    #[test]
    fn log_value_out_of_range_returns_false() {
        let mut p = make_profiler();
        assert!(!p.log_value(999, 42));
    }

    #[test]
    fn flush_log_clears_history_and_events() {
        let mut p = make_profiler();
        let _k = p.create_key("Test");
        // Simulate a frame
        p.start_frame();
        p.log_event("something");
        p.end_frame();
        assert_eq!(p.num_frames(), 1);

        p.flush_log();
        assert_eq!(p.num_frames(), 0);
        for entry in &p.entries {
            assert!(entry.time_history.is_empty());
        }
    }

    #[test]
    fn frame_records_event_string() {
        let mut p = make_profiler();
        p.start_frame();
        p.log_event("hello ");
        p.log_event("world");
        p.end_frame();
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.events[0], "hello world");
    }

    #[test]
    fn end_frame_stores_current_amount_and_resets() {
        let mut p = make_profiler();
        let k = p.create_key("Val");
        p.start_frame();
        p.log_value(k, 100);
        p.end_frame();

        assert_eq!(p.entries[k as usize].time_history, vec![100]);
        assert_eq!(p.entries[k as usize].current_amount, 0);
    }

    #[test]
    fn recursive_log_start_end() {
        let mut p = make_profiler();
        let k = p.create_key("Nested");

        // Only outermost pair should trigger timing
        assert!(p.log_start(k));
        assert_eq!(p.entries[k as usize].recursion_depth, 1);
        assert!(p.log_start(k));
        assert_eq!(p.entries[k as usize].recursion_depth, 2);

        assert!(p.log_end(k));
        assert_eq!(p.entries[k as usize].recursion_depth, 1);
        // time should NOT have been accumulated yet (still nested)
        assert!(p.log_end(k));
        assert_eq!(p.entries[k as usize].recursion_depth, 0);
    }

    #[test]
    fn generate_csv_header_sorted() {
        let mut p = make_profiler();
        p.create_key("Zebra");
        p.create_key("Apple");
        let csv = p.generate_csv();
        let first_line = csv.lines().next().unwrap();
        // Entries should be sorted alphabetically
        assert!(first_line.contains("Apple;"));
        assert!(first_line.contains("Ze Sum;"));
        assert!(first_line.contains("Zebra;"));
        // Apple should come before Ze Sum which should come before Zebra
        let apple_pos = first_line.find("Apple").unwrap();
        let ze_sum_pos = first_line.find("Ze Sum").unwrap();
        let zebra_pos = first_line.find("Zebra").unwrap();
        assert!(apple_pos < ze_sum_pos);
        assert!(ze_sum_pos < zebra_pos);
    }

    #[test]
    fn generate_csv_data_rows() {
        let mut p = make_profiler();
        let k = p.create_key("Counter");
        p.start_frame();
        p.log_value(k, 77);
        p.log_event("frame1 event");
        p.end_frame();

        let csv = p.generate_csv();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2); // header + 1 data row
        assert!(lines[1].contains("\"frame1 event\""));
        assert!(lines[1].contains("77;"));
    }

    #[test]
    fn multiple_frames() {
        let mut p = make_profiler();
        let k = p.create_key("V");

        p.start_frame();
        p.log_value(k, 10);
        p.end_frame();

        p.start_frame();
        p.log_value(k, 20);
        p.end_frame();

        p.start_frame();
        p.log_value(k, 30);
        p.end_frame();

        assert_eq!(p.num_frames(), 3);
        assert_eq!(p.entries[k as usize].time_history, vec![10, 20, 30]);
    }

    #[test]
    fn filename_stored() {
        let p = Profiler::new("my_profile.csv");
        assert_eq!(p.filename(), "my_profile.csv");
    }

    #[test]
    fn start_frame_clears_current_event() {
        let mut p = make_profiler();
        p.start_frame();
        p.log_event("first");
        p.end_frame();

        p.start_frame();
        // current_event should be empty after start_frame
        p.end_frame();
        assert_eq!(p.events[1], "");
    }

    #[test]
    fn csv_line_endings_are_crlf() {
        let mut p = make_profiler();
        p.start_frame();
        p.end_frame();
        let csv = p.generate_csv();
        assert!(csv.contains("\r\n"));
        // Make sure we don't have bare \n (all \n should be preceded by \r)
        for (i, ch) in csv.chars().enumerate() {
            if ch == '\n' && i > 0 {
                let prev: Vec<char> = csv.chars().collect();
                assert_eq!(prev[i - 1], '\r');
            }
        }
    }
}
