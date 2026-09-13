use super::shuffle_sherwood_slots;
use crate::sim_rng::RngSite;

#[test]
fn returning_pc_placement_draws_precede_all_beam_me_shuffle_draws() {
    crate::sim_rng::with_seed(0xA036, |sim| {
        let (_, trace) = crate::sim_rng::with_draw_trace(|| {
            let _ = crate::engine::teleport::roll_sherwood_placement(sim);
            shuffle_sherwood_slots(sim, 4, |_, _| {});
        });
        assert_eq!(trace.len(), 203);
        assert_eq!(
            &trace[..3],
            &[
                RngSite::SherwoodReturningPcPlacement,
                RngSite::SherwoodReturningPcPlacement,
                RngSite::SherwoodReturningPcPlacement,
            ]
        );
        assert!(
            trace[3..]
                .iter()
                .all(|site| *site == RngSite::SherwoodBeamMeShuffle)
        );
    });
}
