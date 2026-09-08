// Shared test-only resolver; no application crate dependency is introduced.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/original_data.rs"
));
