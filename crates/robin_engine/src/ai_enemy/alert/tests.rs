use super::*;
use crate::ai_enemy::SeekFlags;

#[test]
fn alert_soldiers_transfers_the_live_query_and_failure_branch() {
    let mut ai = EnemyAi::new(99);
    let center = Position {
        x: 14.0,
        y: 27.0,
        ..Position::default()
    };
    let sim = crate::sim_rng::test_context();
    let ctx = AiContext::test_fixture();
    let tick = AiPerTickData::stub();
    let call = ai
        .alert_soldiers(
            center,
            SeekFlags::BODY_SEEK.bits(),
            ThinkEnv::new(&sim, &ctx, &tick, None),
            AlertSoldiersFailureContinuation::ReturnToDuty,
        )
        .unwrap_err();
    assert!(matches!(call.tail, crate::ai::DutyTail::AlertSoldiers {
        center: actual, flags, failure: AlertSoldiersFailureContinuation::ReturnToDuty
    } if actual == center && flags == SeekFlags::BODY_SEEK.bits()));
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
}
