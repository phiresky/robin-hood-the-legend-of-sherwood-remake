//! Native admission uses the dedicated codec helper.
use super::ReplayLoadError;

pub(super) fn validate_in_native_child(text: &str) -> Result<(), ReplayLoadError> {
    use robin_replay_format::native_admission::AdmissionError;
    robin_replay_format::native_admission::validate_in_native_child(text).map_err(|error| {
        match error {
            AdmissionError::Compact(error) => ReplayLoadError::Compact(error),
            AdmissionError::AdmissionRejected(error) => ReplayLoadError::AdmissionRejected(error),
            AdmissionError::ResourceLimit { stage, detail } => {
                ReplayLoadError::ResourceLimit { stage, detail }
            }
            AdmissionError::ContainmentUnavailable(error) => {
                ReplayLoadError::ContainmentUnavailable(error)
            }
            AdmissionError::WorkerProtocol(error) => ReplayLoadError::WorkerProtocol(error),
        }
    })
}
