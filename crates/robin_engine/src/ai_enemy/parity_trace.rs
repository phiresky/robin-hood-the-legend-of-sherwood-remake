//! Enemy AI parity stderr payloads. Gates and evaluation remain at their callers.
//! These cold formatters preserve the byte-level diagnostic protocol.

use crate::ai::parity_trace::trace_event;

trace_event! {
    aidecision_reactiontime_running_event(
        frame: Display,
        owner: Display,
        co: Debug,
        state: Debug,
        value: Debug,
        primary: Debug,
        rider: Display,
        couldnt: Display,
        already: Display,
        owner_work_before: Debug;
        stimulus_type: Debug
    ) => "AIDECISION frame={} owner={} co={:?} stage=reactiontime_running_event stimulus={stimulus_type:?} state={:?}/{:?} primary={:?} rider={} couldnt={} already={} owner_work_before={:?}"
}

trace_event! {
    aidecision_reactiontime_running_done(
        frame: Display,
        owner: Display,
        state: Debug,
        value: Debug,
        primary: Debug,
        couldnt: Display,
        already: Display,
        owner_work_after: Debug;
    ) => "AIDECISION frame={} owner={} stage=reactiontime_running_done state={:?}/{:?} primary={:?} couldnt={} already={} owner_work_after={:?}"
}

trace_event! {
    reconsider_stimulus(
        frame: Display,
        owner: Display,
        creation_order: Debug;
        stimulus_type: Debug
    ) => "[RECONSIDER_STIMULUS] frame={} owner={} creation_order={:?} stimulus={stimulus_type:?}"
}

trace_event! {
    them_timer_entry_bow_behind_shield(
        frame: Display,
        co: Debug,
        me: Display,
        state: Debug,
        substate: Debug;
    ) => "[THEM frame={} co={:?} me={} phase=timer_entry state={:?} substate={:?} route=bow_behind_shield]"
}

trace_event! {
    them_timer_entry_officer_orders_waiting(
        frame: Display,
        co: Debug,
        me: Display,
        state: Debug,
        substate: Debug;
    ) => "[THEM frame={} co={:?} me={} phase=timer_entry state={:?} substate={:?} route=officer_orders_waiting]"
}

trace_event! {
    shield_timer(
        frame: Display,
        me: Display,
        action: Debug,
        shield: Display,
        left: Debug,
        right: Debug,
        archer_behind: Debug,
        target: Debug,
        target_action: Debug;
    ) => "SHIELD_TIMER frame={} me={} action={:?} shield={} left={:?} right={:?} archer_behind={:?} target={:?} target_action={:?}"
}

trace_event! {
    phalanx_timer(
        frame: Display,
        me: Display,
        action: Debug,
        left: Debug,
        right: Debug,
        target: Debug,
        archer_behind: Debug;
    ) => "PHALANX_TIMER frame={} me={} action={:?} left={:?} right={:?} target={:?} archer_behind={:?}"
}

trace_event! {
    phalanx_timer_exit(
        frame: Display,
        me: Display,
        substate: Debug;
    ) => "PHALANX_TIMER_EXIT frame={} me={} substate={:?}"
}

trace_event! {
    seekarea_caller_couldnt_reach_emergency(
        frame: Display,
        owner_handle: Display,
        owner_creation_order: Display;
    ) => "SEEKAREA_CALLER {{\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"caller\":\"couldnt_reach_emergency\",\"stimulus\":\"event_couldnt_reach_point\"}}"
}

trace_event! {
    good_strike_think_entry(
        frame: Display,
        owner: Display,
        owner_co: Debug,
        state: Debug,
        substate: Debug,
        will_say: Display,
        vip: Display;
    ) => "[GOOD_STRIKE frame={} owner={} owner_co={:?} phase=think_entry state={:?} substate={:?} will_say={} vip={}]"
}

trace_event! {
    good_strike_say_queued(
        frame: Display,
        owner: Display,
        owner_co: Debug,
        remark: Debug;
    ) => "[GOOD_STRIKE frame={} owner={} owner_co={:?} phase=say_queued remark={:?}]"
}

trace_event! {
    seekarea_point_dump(
        frame: Display,
        index: Display,
        id: Display,
        x: Display,
        y: Display,
        level: Display,
        center: Display,
        value: Display,
        value_2: Display,
        norm: Display,
        norm_bits: Display,
        near: Display,
        frame_when_full_interest: Display;
    ) => "SEEKAREA {{\"event\":\"point_dump\",\"frame\":{},\"index\":{},\"id\":{},\"x\":{},\"y\":{},\"level\":{},\"center\":[{},{},{}],\"norm\":{},\"norm_bits\":{},\"near\":{},\"frame_when_full_interest\":{}}}"
}

trace_event! {
    seekarea_phase4_candidate(
        frame: Display,
        owner_handle: Display,
        owner_creation_order: Display,
        candidate_ordinal: Display,
        point_id: Display,
        point_index: Display,
        norm: Display,
        norm_bits: Display,
        frame_when_full_interest: Display,
        interest: Display,
        attempt_raw: Display,
        attempt_mod: Display,
        attempt_result: Display,
        insertion_raw: Display,
        insertion_index: Display,
        accumulator_before_bits: Display,
        accumulator_after_bits: Display;
    ) => "SEEKAREA {{\"event\":\"phase4_candidate\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"candidate_ordinal\":{},\"point_id\":{},\"point_index\":{},\"norm\":{},\"norm_bits\":{},\"frame_when_full_interest\":{},\"interest\":{},\"attempt_raw\":{},\"attempt_mod\":{},\"attempt_result\":{},\"insertion_raw\":{},\"insertion_index\":{},\"accumulator_before_bits\":{},\"accumulator_after_bits\":{}}}"
}

trace_event! {
    seekarea_selection_summary(
        frame: Display,
        owner_handle: Display,
        owner_creation_order: Debug,
        center: Display,
        value: Display,
        standard_radius: Display,
        near_points: Display,
        expected_for_one: Display,
        visible_friends: Display,
        clears_help: Display,
        expected_before_help_random: Display,
        expected_points: Display,
        phase4_attempts: Display,
        phase4_accepts: Display,
        preselection_rng_draws: Display,
        phase4_rng_draws: Display,
        selection_rng_draws: Display,
        accepted_interest_sum: Display;
    ) => "SEEKAREA {{\"event\":\"selection_summary\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{:?},\"center\":[{},{}],\"standard_radius\":{},\"near_points\":{},\"expected_for_one\":{},\"visible_friends\":{},\"clears_help\":{},\"expected_before_help_random\":{},\"expected_points\":{},\"phase4_attempts\":{},\"phase4_accepts\":{},\"preselection_rng_draws\":{},\"phase4_rng_draws\":{},\"selection_rng_draws\":{},\"accepted_interest_sum\":{}}}"
}

trace_event! {
    seekarea_selection_extra(
        frame: Display,
        owner_creation_order: Debug,
        flags: Display,
        seek_direction: Display,
        center_level: Display,
        obligatory: Debug,
        obligatory2: Debug,
        selected_random: Debug;
    ) => "SEEKAREA {{\"event\":\"selection_extra\",\"frame\":{},\"owner_creation_order\":{:?},\"flags\":{},\"seek_direction\":{},\"center_level\":{},\"obligatory\":{:?},\"obligatory2\":{:?},\"selected_random\":{:?}}}"
}

trace_event! {
    seekarea_phase6_before(
        frame: Display,
        owner_handle: Display,
        owner_creation_order: Display,
        state: Display,
        substate: Display,
        flags: Display,
        seek_direction: Display,
        list_size: Display,
        list_empty: Display,
        location_first: Display,
        location_end: Display,
        personal1_constructor: Display;
    ) => "SEEKAREA {{\"event\":\"phase6_before\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"state\":{},\"substate\":{},\"flags\":{},\"seek_direction\":{},\"list_size\":{},\"list_empty\":{},\"location_first\":{},\"location_end\":{},\"personal1_constructor\":\"{}\"}}"
}

trace_event! {
    seekarea_phase6_center(
        frame: Display,
        owner_creation_order: Debug,
        center: Display,
        value: Display,
        seek_position: Display,
        value_2: Display;
    ) => "SEEKAREA {{\"event\":\"phase6_center\",\"frame\":{},\"owner_creation_order\":{:?},\"center\":[{},{}],\"seek_position\":[{},{}]}}"
}

trace_event! {
    seekarea_phase6_personal1(
        frame: Display,
        owner_creation_order: Display,
        constructor: Display,
        list_size: Display;
    ) => "SEEKAREA {{\"event\":\"phase6_personal1\",\"frame\":{},\"owner_creation_order\":{},\"constructor\":\"{}\",\"inserted_id\":1111,\"list_size\":{}}}"
}

trace_event! {
    seekarea_phase6_after(
        frame: Display,
        owner_creation_order: Display,
        personal2_inserted: Display,
        personal2_constructor: Display,
        list_size: Display;
    ) => "SEEKAREA {{\"event\":\"phase6_after\",\"frame\":{},\"owner_creation_order\":{},\"personal2_inserted\":{},\"personal2_constructor\":\"{}\",\"list_size\":{}}}"
}

trace_event! {
    seekarea_next_point_locked(
        frame: Display,
        owner_handle: Display,
        owner_creation_order: Debug,
        point_id: Display;
    ) => "SEEKAREA {{\"event\":\"next_point_locked\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{:?},\"point_id\":{}}}"
}

trace_event! {
    seekarea_next_point_roll(
        frame: Display,
        owner_handle: Display,
        owner_creation_order: Debug,
        point_id: Display,
        interest: Display,
        roll: Display,
        accepted: Display,
        remaining: Debug;
    ) => "SEEKAREA {{\"event\":\"next_point_roll\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{:?},\"point_id\":{},\"interest\":{},\"roll\":{},\"accepted\":{},\"remaining\":{:?}}}"
}

trace_event! {
    shield_bearer_candidate(
        frame: Display,
        me: Display,
        candidate: Display,
        substate: Display,
        archer_behind: Debug;
        dist: Display,
        min_distance: Display
    ) => "SHIELD_BEARER_CANDIDATE frame={} me={} candidate={} substate={} archer_behind={:?} dist={dist} min={min_distance}"
}

trace_event! {
    shield_bearer_result(
        frame: Display,
        me: Display,
        registry: Display;
        best: Debug,
        shield_bearers: Debug
    ) => "SHIELD_BEARER_RESULT frame={} me={} best={best:?} registry={} bearers={shield_bearers:?}"
}

trace_event! {
    cover_pos(
        frame: Display,
        me: Display,
        bearer: Display,
        sub: Display,
        bearer_pos: Debug,
        bearer_dir: Display,
        bearer_raw: Debug,
        behind: Debug;
        ok: Display
    ) => "COVER_POS frame={} me={} bearer={} sub={} bearer_pos={:?} bearer_dir={} bearer_raw={:?} behind={:?} straight_ok={ok}"
}

trace_event! {
    archer_protection_scan(
        frame: Display,
        owner: Display,
        cand: Display;
    ) => "ARCHER_PROTECTION_SCAN frame={} owner={} cand={} skip=not_friendly"
}

trace_event! {
    archer_protection_scan_2(
        frame: Display,
        owner: Display,
        cand: Display,
        sq: Display,
        state: Debug,
        sub: Display,
        archer: Display,
        tower: Display,
        sbb: Debug,
        own_abm: Debug,
        abm: Debug,
        shield: Display,
        pos: Debug,
        elev: Display;
    ) => "ARCHER_PROTECTION_SCAN frame={} owner={} cand={} sq={} state={:?} sub={} archer={} tower={} sbb={:?} own_abm={:?} abm={:?} shield={} pos={:?} elev={}"
}

trace_event! {
    reconsider_phalanx(
        frame: Display,
        me: Display,
        nearest: Display,
        sq: Display,
        thr: Display,
        left: Debug,
        right: Debug;
    ) => "RECONSIDER_PHALANX frame={} me={} nearest={} sq={} thr={} left={:?} right={:?}"
}

trace_event! {
    arrow_protection_reject_advancing_hourglass(
        frame: Display,
        owner: Display,
        substate: Debug;
    ) => "ARROW_PROTECTION frame={} owner={} result=reject reason=advancing_hourglass substate={:?}"
}

trace_event! {
    arrow_protection_reject_substate(
        frame: Display,
        owner: Display,
        substate: Debug,
        from_hourglass: Display;
    ) => "ARROW_PROTECTION frame={} owner={} result=reject reason=substate substate={:?} from_hourglass={}"
}

trace_event! {
    arrow_protection_reject_shield_bearer(
        frame: Display,
        owner: Display,
        owner_present: Display,
        is_shield_bearer: Debug;
    ) => "ARROW_PROTECTION frame={} owner={} result=reject reason=shield_bearer owner_present={} is_shield_bearer={:?}"
}

trace_event! {
    arrow_protection_reject_no_nearest_enemy(
        frame: Display,
        owner: Display,
        list_them: Debug;
    ) => "ARROW_PROTECTION frame={} owner={} result=reject reason=no_nearest_enemy list_them={:?}"
}

trace_event! {
    arrow_protection_reject_enemy_near(
        frame: Display,
        owner: Display,
        nearest: Display,
        square_distance_bits: Display,
        threshold: Display;
    ) => "ARROW_PROTECTION frame={} owner={} result=reject reason=enemy_near nearest={} square_distance_bits={} threshold={}"
}

trace_event! {
    arrow_protection_seen(
        frame: Display,
        owner: Display,
        seen: Debug;
    ) => "ARROW_PROTECTION_SEEN frame={} owner={} seen={:?}"
}

trace_event! {
    arrow_protection_enemy(
        frame: Display,
        owner: Display,
        cand: Display,
        sq: Display,
        action: Debug,
        bow: Display;
    ) => "ARROW_PROTECTION_ENEMY frame={} owner={} cand={} sq={} action={:?} bow={}"
}

trace_event! {
    arrow_protection_reject_no_protection_target(
        frame: Display,
        owner: Display,
        nearest: Display,
        protectable: Display,
        nearby: Debug;
    ) => "ARROW_PROTECTION frame={} owner={} result=reject reason=no_protection_target nearest={} dangerous=0 protectable={} nearby={:?}"
}

trace_event! {
    arrow_protection_run_to_phalanx(
        frame: Display,
        owner: Display,
        nearest: Display,
        dangerous: Debug,
        target: Debug,
        run_pos: Debug,
        direction: Display,
        left: Debug,
        right: Debug,
        inherited_sector_identity_differs: Display;
    ) => "ARROW_PROTECTION frame={} owner={} result=run_to_phalanx nearest={} dangerous={:?} target={:?} run_pos={:?} direction={} left={:?} right={:?} inherited_sector_identity_differs={}"
}

trace_event! {
    arrow_protection_raise_shield(
        frame: Display,
        owner: Display,
        nearest: Display,
        dangerous: Debug,
        target: Debug,
        target_pos: Debug,
        target_elevation_bits: Display;
    ) => "ARROW_PROTECTION frame={} owner={} result=raise_shield nearest={} dangerous={:?} target={:?} target_pos={:?} target_elevation_bits={}"
}

trace_event! {
    reconsider_entry_entry(
        frame: Display,
        owner: Display,
        creation_order: Debug,
        rng: Debug,
        substate: Debug,
        primary: Debug,
        swordfighting: Display,
        enter_pending: Display,
        position: Debug,
        value: Debug,
        value_2: Debug,
        direction: Display,
        blood: Display,
        cheat: Display,
        trainer: Display;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} creation_order={:?} phase=entry rng={:?} substate={:?} primary={:?} swordfighting={} enter_pending={} position=({:?},{:?},{:?}) direction={} blood={} cheat={} trainer={}"
}

trace_event! {
    reconsider_entry_return_enter_pending(
        frame: Display,
        owner: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=enter_pending rng={:?}"
}

trace_event! {
    reconsider_entry_return_not_swordfighting(
        frame: Display,
        owner: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=not_swordfighting rng={:?}"
}

trace_event! {
    reconsider_entry_principal(
        frame: Display,
        owner: Display,
        primary: Debug,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=principal primary={:?} rng={:?}"
}

trace_event! {
    reconsider_entry_return_primary_friend(
        frame: Display,
        owner: Display,
        primary: Debug,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=primary_friend primary={:?} rng={:?}"
}

trace_event! {
    reconsider_entry_detection(
        frame: Display,
        owner: Display,
        primary: Debug,
        detected: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=detection primary={:?} detected={} rng={:?}"
}

trace_event! {
    reconsider_entry_return_missing_primary_snapshot(
        frame: Display,
        owner: Display,
        primary: Debug,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=missing_primary_snapshot primary={:?} rng={:?}"
}

trace_event! {
    reconsider_entry_facing(
        frame: Display,
        owner: Display,
        primary: Debug,
        target: Debug,
        value: Debug,
        value_2: Debug,
        direction: Display,
        facing: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=facing primary={:?} target=({:?},{:?},{:?}) direction={} facing={} rng={:?}"
}

trace_event! {
    reconsider_entry_lists(
        frame: Display,
        owner: Display,
        us: Debug,
        them: Debug,
        swordfighting_enemies: Display,
        nearest_friend_solo: Debug,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=lists us={:?} them={:?} swordfighting_enemies={} nearest_friend_solo={:?} rng={:?}"
}

trace_event! {
    reconsider_entry_return_merry_archer_flee(
        frame: Display,
        owner: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=merry_archer_flee rng={:?}"
}

trace_event! {
    reconsider_entry_rebalance_gate(
        frame: Display,
        owner: Display,
        primary: Debug,
        primary_opponents: Display,
        outnumbered: Display,
        nearest_friend_solo: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=rebalance_gate primary={:?} primary_opponents={} outnumbered={} nearest_friend_solo={:?}"
}

trace_event! {
    reconsider_entry_rebalance_maurice(
        frame: Display,
        owner: Display,
        maurice: Display,
        present: Display,
        opponents: Debug,
        nearby: Debug,
        nearest_enemy_of_solo: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=rebalance_maurice maurice={} present={} opponents={:?} nearby={:?} nearest_enemy_of_solo={:?}"
}

trace_event! {
    reconsider_entry_rebalance_pick(
        frame: Display,
        owner: Display,
        nearest_enemy_of_solo: Display,
        nearest_to_that_enemy: Debug,
        i_should_take_him: Display;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=rebalance_pick nearest_enemy_of_solo={} nearest_to_that_enemy={:?} i_should_take_him={}"
}

trace_event! {
    reconsider_entry_return_rebalance(
        frame: Display,
        owner: Display,
        target: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=rebalance target={} rng={:?}"
}

trace_event! {
    reconsider_entry_return_stupid_soldiers_cheat(
        frame: Display,
        owner: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=stupid_soldiers_cheat rng={:?}"
}

trace_event! {
    reconsider_entry_return_drunk_freeze(
        frame: Display,
        owner: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=drunk_freeze rng={:?}"
}

trace_event! {
    reconsider_entry_after_drunk(
        frame: Display,
        owner: Display,
        blood: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=after_drunk blood={} rng={:?}"
}

trace_event! {
    reconsider_entry_return_missing_refreshed_primary(
        frame: Display,
        owner: Display,
        primary: Debug,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=missing_refreshed_primary primary={:?} rng={:?}"
}

trace_event! {
    reconsider_entry_weak_charge_gate(
        frame: Display,
        owner: Display,
        enemy_weak: Display,
        rank: Debug,
        charge_dist: Display,
        flat_dist: Display,
        max_range: Display,
        ability: Display;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=weak_charge_gate enemy_weak={} rank={:?} charge_dist={} flat_dist={} max_range={} ability={}"
}

trace_event! {
    reconsider_entry_return_weak_enemy_charge(
        frame: Display,
        owner: Display,
        target: Debug,
        distance: Display,
        max_range: Debug,
        ability: Display,
        rng: Debug;
    ) => "[RECONSIDER_ENTRY] frame={} owner={} phase=return reason=weak_enemy_charge target={:?} distance={} max_range={:?} ability={} rng={:?}"
}

trace_event! {
    reconsider_position(
        frame: Display,
        owner: Display,
        creation_order: Debug,
        friends: Display,
        swordfighting_enemies: Display,
        combat_trainer: Display,
        eligible: Display,
        roll: Debug,
        do_reposition: Display;
    ) => "[RECONSIDER_POSITION] frame={} owner={} creation_order={:?} friends={} swordfighting_enemies={} combat_trainer={} eligible={} roll={:?} do_reposition={}"
}

trace_event! {
    reconsider_invoke(
        frame: Display,
        owner: Display,
        state: Debug,
        substate: Debug,
        fighters: Display;
    ) => "RECONSIDER {{\"event\":\"invoke\",\"frame\":{},\"owner\":{},\"state\":{:?},\"substate\":{:?},\"fighters\":{}}}"
}

trace_event! {
    reconsider_them_candidate(
        frame: Display,
        owner: Display,
        fighter: Display,
        friendly: Display,
        able: Display,
        distance: Display,
        result: Debug;
    ) => "RECONSIDER {{\"event\":\"them_candidate\",\"frame\":{},\"owner\":{},\"fighter\":{},\"friendly\":{},\"able\":{},\"distance\":{},\"result\":{:?}}}"
}

trace_event! {
    reconsider_them_candidate_2(
        frame: Display,
        owner: Display,
        fighter: Display,
        friendly: Display,
        able: Display,
        distance: Display;
    ) => "RECONSIDER {{\"event\":\"them_candidate\",\"frame\":{},\"owner\":{},\"fighter\":{},\"friendly\":{},\"able\":{},\"distance\":{},\"result\":\"radius\"}}"
}

trace_event! {
    reconsider_us_candidate(
        frame: Display,
        owner: Display,
        fighter: Display,
        friendly: Display,
        able: Display,
        distance_uword: Display,
        result: Debug;
    ) => "RECONSIDER {{\"event\":\"us_candidate\",\"frame\":{},\"owner\":{},\"fighter\":{},\"friendly\":{},\"able\":{},\"distance_uword\":{},\"result\":{:?}}}"
}

trace_event! {
    reconsider_position_2(
        frame: Display,
        owner: Display,
        candidate: Display,
        input: Debug,
        attacker: Debug,
        attacker_position: Debug,
        target: Debug,
        target_position: Debug,
        target_direction: Display,
        change_adversary: Display,
        change_position: Display,
        line_position: Display,
        left: Debug,
        right: Debug,
        bonus: Display,
        line_jump: Debug,
        score: Display;
    ) => "[RECONSIDER_POSITION] frame={} owner={} candidate={} input={:?} evaluated=(attacker={:?} attacker_position={:?} target={:?} target_position={:?} target_direction={} change_adversary={} change_position={} line_position={} left={:?} right={:?} bonus={} line_jump={:?}) score={}"
}

trace_event! {
    reconsider_position_3(
        frame: Display,
        owner: Display,
        chosen: Display,
        score: Display,
        attacker: Debug,
        attacker_position: Debug,
        target: Debug,
        target_position: Debug,
        target_direction: Display,
        change_adversary: Display,
        change_position: Display,
        line_position: Display,
        left: Debug,
        right: Debug,
        bonus: Display,
        line_jump: Debug;
    ) => "[RECONSIDER_POSITION] frame={} owner={} chosen={} score={} result=(attacker={:?} attacker_position={:?} target={:?} target_position={:?} target_direction={} change_adversary={} change_position={} line_position={} left={:?} right={:?} bonus={} line_jump={:?})"
}

trace_event! {
    them_battle_entry(
        frame: Display,
        co: Debug,
        me: Display,
        list: Debug;
    ) => "[THEM frame={} co={:?} me={} phase=battle_entry list={:?}]"
}

trace_event! {
    primary_swap_battle_primary_selected(
        frame: Display,
        co: Debug,
        owner: Display,
        list_them: Debug,
        selected: Debug;
    ) => "[PRIMARY_SWAP frame={} co={:?} owner={} phase=battle_primary_selected list_them={:?} selected={:?}]"
}

trace_event! {
    them_battle_cleanup_after(
        frame: Display,
        co: Debug,
        me: Display,
        visible_count: Display,
        list: Debug,
        unconscious: Debug;
    ) => "[THEM frame={} co={:?} me={} phase=battle_cleanup_after visible_count={} list={:?} unconscious={:?}]"
}

trace_event! {
    archer_decision(
        frame: Display,
        me: Display,
        tower: Display,
        sbb: Debug,
        shooting_point: Debug,
        too_near: Display,
        pos: Debug,
        primary: Debug,
        primary_pos: Debug;
    ) => "ARCHER_DECISION frame={} me={} tower={} sbb={:?} shooting_point={:?} too_near={} pos={:?} primary={:?} primary_pos={:?}"
}

trace_event! {
    battle_decision(
        frame: Display,
        me: Display,
        decision: Debug,
        old_substate: Debug,
        primary: Debug,
        seen: Display,
        friends_nearer: Display;
    ) => "BATTLE_DECISION frame={} me={} decision={:?} old_substate={:?} primary={:?} seen={} friends_nearer={}"
}

trace_event! {
    aidecision_reconsider_enter(
        frame: Display,
        owner: Display,
        co: Debug,
        reachpoint: Display,
        state: Debug,
        value: Debug,
        primary: Debug,
        seek: LowerHex,
        value_2: LowerHex,
        sector: Debug,
        level: Display,
        rider: Display,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} co={:?} stage=reconsider_enter reachpoint={} state={:?}/{:?} primary={:?} seek=({:08x},{:08x},sector={:?},level={}) rider={} couldnt={} already={} owner_work={:?}"
}

trace_event! {
    aidecision_reconsider_close_enough(
        frame: Display,
        owner: Display,
        working_distance_bits: LowerHex,
        working_distance: Display,
        sword_range: Display,
        run_distance: Display,
        b_charge: Display,
        b_first: Display,
        my_line_jump: Debug,
        target_in_lift: Display,
        working_target: Debug;
    ) => "AIDECISION frame={} owner={} stage=reconsider_close_enough working_distance_bits={:08x} working_distance={} sword_range={} run_distance={} b_charge={} b_first={} my_line_jump={:?} target_in_lift={} working_target={:?}"
}

trace_event! {
    aidecision_reconsider_deferred(
        frame: Display,
        owner: Display,
        state: Debug,
        value: Debug,
        primary: Debug,
        target_position: LowerHex,
        value_2: LowerHex,
        sector: Debug,
        level: Display,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} stage=reconsider_deferred state={:?}/{:?} primary={:?} target_position=({:08x},{:08x},sector={:?},level={}) couldnt={} already={} owner_work={:?}"
}

trace_event! {
    aidecision_reconsider_resume_enter(
        frame: Display,
        owner: Display,
        co: Debug,
        state: Debug,
        value: Debug,
        couldnt: Display,
        already: Display,
        target_position: LowerHex,
        value_2: LowerHex,
        sector: Debug,
        level: Display,
        avenger_wait: Debug,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} co={:?} stage=reconsider_resume_enter state={:?}/{:?} couldnt={} already={} target_position=({:08x},{:08x},sector={:?},level={}) avenger_wait={:?} owner_work={:?}"
}

trace_event! {
    aidecision_reconsider_resume_result_route_ok(
        frame: Display,
        owner: Display,
        state: Debug,
        value: Debug;
    ) => "AIDECISION frame={} owner={} stage=reconsider_resume_result result=route_ok state={:?}/{:?}"
}

trace_event! {
    aidecision_reconsider_resume_result_failed_without_wait_position(
        frame: Display,
        owner: Display;
    ) => "AIDECISION frame={} owner={} stage=reconsider_resume_result result=failed_without_wait_position couldnt=true"
}

trace_event! {
    aidecision_reconsider_resume_result_avenger_fallback(
        frame: Display,
        owner: Display,
        state: Debug,
        value: Debug,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} stage=reconsider_resume_result result=avenger_fallback state={:?}/{:?} couldnt={} already={} owner_work={:?}"
}

trace_event! {
    aidecision_rider_attack_enter(
        frame: Display,
        owner: Display,
        co: Debug,
        state: Debug,
        value: Debug,
        primary: Debug,
        rider: Display,
        position: LowerHex,
        value_2: LowerHex,
        sector: Debug,
        level: Display,
        direction: Display,
        list_them: Debug,
        fighters: Display;
    ) => "AIDECISION frame={} owner={} co={:?} stage=rider_attack_enter state={:?}/{:?} primary={:?} rider={} position=({:08x},{:08x},sector={:?},level={}) direction={} list_them={:?} fighters={}"
}

trace_event! {
    aidecision_rider_attack_result_no_destination(
        frame: Display,
        owner: Display,
        final_primary: Debug,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} stage=rider_attack_result result=no_destination final_primary={:?} couldnt={} already={} owner_work={:?}"
}

trace_event! {
    aidecision_rider_attack_result_accepted(
        frame: Display,
        owner: Display,
        target: Display,
        destination: LowerHex,
        value: LowerHex,
        sector: Debug,
        level: Display,
        begin_charge: Display,
        state: Debug,
        value_2: Debug,
        couldnt: Display,
        already: Display,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} stage=rider_attack_result result=accepted target={} destination=({:08x},{:08x},sector={:?},level={}) begin_charge={} state={:?}/{:?} couldnt={} already={} owner_work={:?}"
}

trace_event! {
    aidecision_rider_candidate_enter(
        frame: Display,
        owner: Display,
        candidate: Display,
        me: LowerHex,
        value: LowerHex,
        level: Display,
        enemy: LowerHex,
        value_2: LowerHex,
        level_2: Display,
        direction: Display,
        move_box: Debug;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_enter candidate={} me=({:08x},{:08x},level={}) enemy=({:08x},{:08x},level={}) direction={} move_box={:?}"
}

trace_event! {
    aidecision_rider_candidate_result_reject_behind(
        frame: Display,
        owner: Display,
        candidate: Display,
        forward_dot_bits: LowerHex;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_behind forward_dot_bits={:08x}"
}

trace_event! {
    aidecision_rider_candidate_result_reject_too_near(
        frame: Display,
        owner: Display,
        candidate: Display,
        norm_bits: LowerHex,
        sq_norm_bits: LowerHex;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_too_near norm_bits={:08x} sq_norm_bits={:08x}"
}

trace_event! {
    aidecision_rider_candidate_result_reject_zero_orthogonal(
        frame: Display,
        owner: Display,
        candidate: Display,
        ortho_len_bits: LowerHex;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_zero_orthogonal ortho_len_bits={:08x}"
}

trace_event! {
    aidecision_rider_candidate_result_reject_zero_hit_vector(
        frame: Display,
        owner: Display,
        candidate: Display,
        hp_len_bits: LowerHex;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_zero_hit_vector hp_len_bits={:08x}"
}

trace_event! {
    aidecision_rider_candidate_result_reject_zero_hit_norm(
        frame: Display,
        owner: Display,
        candidate: Display,
        hit_norm_bits: LowerHex;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_zero_hit_norm hit_norm_bits={:08x}"
}

trace_event! {
    aidecision_rider_candidate_result_reject_straight(
        frame: Display,
        owner: Display,
        candidate: Display,
        goal: LowerHex,
        value: LowerHex,
        forward_dot_bits: LowerHex,
        sq_norm_bits: LowerHex,
        cos_bits: LowerHex;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_straight goal=({:08x},{:08x}) forward_dot_bits={:08x} sq_norm_bits={:08x} cos_bits={:08x}"
}

trace_event! {
    aidecision_rider_candidate_result_reject_friendly(
        frame: Display,
        owner: Display,
        candidate: Display,
        friendly: Display,
        friendly_position: LowerHex,
        value: LowerHex,
        level: Display,
        goal: LowerHex,
        value_2: LowerHex;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_friendly friendly={} friendly_position=({:08x},{:08x},level={}) goal=({:08x},{:08x})"
}

trace_event! {
    aidecision_rider_candidate_result_accepted(
        frame: Display,
        owner: Display,
        candidate: Display,
        goal: LowerHex,
        value: LowerHex,
        sector: Debug,
        level: Display,
        forward_dot_bits: LowerHex,
        sq_norm_bits: LowerHex,
        cos_bits: LowerHex,
        sq_hit_bits: LowerHex,
        begin_charge: Display;
    ) => "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=accepted goal=({:08x},{:08x},sector={:?},level={}) forward_dot_bits={:08x} sq_norm_bits={:08x} cos_bits={:08x} sq_hit_bits={:08x} begin_charge={}"
}

trace_event! {
    archerstep_decision(
        frame: Display,
        co: Debug,
        me: Display,
        owner_pos: Debug,
        animation: Debug,
        action_state: Debug,
        reached_done: Display,
        timer_running: Display,
        timer_ring: Display,
        already_on_point: Display;
        old_substate: Debug,
        target: Display,
        enemy_pos: Debug,
        goal: Debug
    ) => "[ARCHERSTEP frame={} co={:?} me={} phase=decision old_substate={old_substate:?} target={target} owner_pos={:?} enemy_pos={enemy_pos:?} goal={goal:?} animation={:?} action_state={:?} reached_done={} timer_running={} timer_ring={} already_on_point={}]"
}

trace_event! {
    archerstep_after_goto(
        frame: Display,
        co: Debug,
        me: Display,
        state: Debug,
        substate: Debug,
        already_on_point: Display,
        couldnt_reachpoint: Display,
        halt: Display,
        additional_halts: Display,
        order_count: Display;
    ) => "[ARCHERSTEP frame={} co={:?} me={} phase=after_goto state={:?} substate={:?} already_on_point={} couldnt_reachpoint={} halt={} additional_halts={} order_count={}]"
}

trace_event! {
    cover_arm(
        frame: Display,
        me: Display,
        bearer: Display,
        cover: Debug,
        target: Debug,
        target_pos: Debug,
        sq: Display,
        sq_view: Display,
        grid: Display;
    ) => "COVER_ARM frame={} me={} bearer={} cover={:?} target={:?} target_pos={:?} sq={} sq_view={} grid={}"
}

trace_event! {
    cover_arm_2(
        frame: Display,
        me: Display,
        bearer: Display,
        grid: Display;
    ) => "COVER_ARM frame={} me={} bearer={} cover=None grid={}"
}

trace_event! {
    them_battle_friend_after_360(
        frame: Display,
        co: Debug,
        me: Display,
        friend: Display,
        state: Debug,
        substate: Debug,
        primary_target: Debug;
    ) => "[THEM frame={} co={:?} me={} phase=battle_friend_after_360 friend={} state={:?} substate={:?} primary_target={:?}]"
}

trace_event! {
    primary_swap_swap_stop_zero(
        frame: Display,
        co: Debug,
        owner: Display,
        friend: Debug;
    ) => "[PRIMARY_SWAP frame={} co={:?} owner={} phase=swap_stop_zero friend={:?}]"
}

trace_event! {
    primary_swap_swap_skip_same(
        frame: Display,
        co: Debug,
        owner: Display,
        friend: Debug,
        owner_target: Debug,
        friend_target: Debug;
    ) => "[PRIMARY_SWAP frame={} co={:?} owner={} phase=swap_skip_same friend={:?} owner_target={:?} friend_target={:?}]"
}

trace_event! {
    primary_swap_swap_test(
        frame: Display,
        co: Debug,
        owner: Display,
        friend: Debug,
        owner_target: Debug,
        friend_target: Debug,
        owner_pos: LowerHex,
        value: LowerHex,
        owner_target_pos: LowerHex,
        value_2: LowerHex,
        friend_pos: LowerHex,
        value_3: LowerHex,
        friend_target_pos: LowerHex,
        value_4: LowerHex,
        working_distance: LowerHex,
        me_to_friend_target: LowerHex,
        friend_to_my_target: LowerHex,
        friend_to_friend_target: LowerHex,
        left: LowerHex,
        right: LowerHex,
        swap: Display;
    ) => "[PRIMARY_SWAP frame={} co={:?} owner={} phase=swap_test friend={:?} owner_target={:?} friend_target={:?} owner_pos=({:08x},{:08x}) owner_target_pos=({:08x},{:08x}) friend_pos=({:08x},{:08x}) friend_target_pos=({:08x},{:08x}) working_distance={:08x} me_to_friend_target={:08x} friend_to_my_target={:08x} friend_to_friend_target={:08x} left={:08x} right={:08x} swap={}]"
}

trace_event! {
    primary_swap_swap_final(
        frame: Display,
        co: Debug,
        owner: Display,
        target: Debug,
        target_pos: LowerHex,
        value: LowerHex,
        distance: LowerHex,
        queued_swaps: Debug;
    ) => "[PRIMARY_SWAP frame={} co={:?} owner={} phase=swap_final target={:?} target_pos=({:08x},{:08x}) distance={:08x} queued_swaps={:?}]"
}

trace_event! {
    them_reinitialize_before(
        frame: Display,
        co: Debug,
        me: Display,
        state: Debug,
        substate: Debug,
        list: Debug,
        seen: Debug;
    ) => "[THEM frame={} co={:?} me={} phase=reinitialize_before state={:?} substate={:?} list={:?} seen={:?}]"
}

trace_event! {
    them_reinitialize_input(
        frame: Display,
        co: Debug,
        me: Display,
        target: Display;
    ) => "[THEM frame={} co={:?} me={} phase=reinitialize_input target={} missing=true]"
}

trace_event! {
    them_reinitialize_input_2(
        frame: Display,
        co: Debug,
        me: Display,
        target: Display,
        dead: Display,
        unconscious: Display,
        carried: Display,
        able: Display;
    ) => "[THEM frame={} co={:?} me={} phase=reinitialize_input target={} dead={} unconscious={} carried={} able={}]"
}

trace_event! {
    them_reinitialize_after(
        frame: Display,
        co: Debug,
        me: Display,
        list: Debug;
    ) => "[THEM frame={} co={:?} me={} phase=reinitialize_after list={:?}]"
}

trace_event! {
    aidecision_set_state(
        frame: Display,
        owner: Display,
        caller: Display,
        from: Debug,
        value: Debug,
        couldnt: Display,
        already: Display,
        owner_work_before: Debug;
        state: Debug,
        substate: Debug
    ) => "AIDECISION frame={} owner={} stage=set_state caller={} from={:?}/{:?} to={state:?}/{substate:?} couldnt={} already={} owner_work_before={:?}"
}

trace_event! {
    aidecision_set_state_done(
        frame: Display,
        owner: Display,
        now: Debug,
        value: Debug,
        couldnt: Display,
        already: Display,
        owner_work_after: Debug;
    ) => "AIDECISION frame={} owner={} stage=set_state_done now={:?}/{:?} couldnt={} already={} owner_work_after={:?}"
}

trace_event! {
    aidecision_think_enter(
        frame: Display,
        owner: Display,
        co: Debug,
        depth: Display,
        open: Display,
        stimulus: Debug,
        state: Debug,
        value: Debug,
        primary: Debug,
        rider: Display,
        couldnt: Display,
        already: Display,
        list_them: Debug,
        owner_work: Debug;
    ) => "AIDECISION frame={} owner={} co={:?} stage=think_enter depth={}/open={} stimulus={:?} state={:?}/{:?} primary={:?} rider={} couldnt={} already={} list_them={:?} owner_work={:?}"
}
