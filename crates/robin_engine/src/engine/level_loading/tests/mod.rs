//! Level-loading test suites split out of `level_loading.rs`.
//!
//! The submodules were written as direct children of `level_loading` and
//! import its items through `use super::…`; the glob below re-exposes the
//! parent's items at this level so those imports keep resolving unchanged.
use super::*;

mod accessory_publication_tests;
mod all_sprite_ambiance_variant_tests;
mod animation_placement_tests;
mod legacy_grid_topology_tests;
mod lift_endpoint_tests;
mod mission_level_builder_tests;
mod mission_start_sprite_tests;
mod rng_order_tests;
