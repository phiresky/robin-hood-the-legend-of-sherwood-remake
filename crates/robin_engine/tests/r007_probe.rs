use robin_engine::level_data::load_level;

#[test]
fn dump_r007_lift_geometry() {
    let loaded = load_level(
        "H01_Lin_VL",
        "Lincoln",
        "/home/phire/robinhood/datadirs/fullgame_linux/Data/Levels",
        &|profile_id| profile_id == 1,
        &mut |_| {},
    )
    .expect("load H01_Lin_VL");

    for (lift_index, lift) in loaded.proto.lifts.iter().enumerate() {
        for (door_index, door) in lift.doors.iter().enumerate() {
            let nearby = [door.point_out, door.point_mid, door.point_in]
                .into_iter()
                .any(|(x, y)| (1600..=2050).contains(&x) && (900..=1250).contains(&y));
            if nearby {
                eprintln!(
                    "lift={lift_index} motion_area={} type={} door={door_index} dir={} active={} polygon={:?} out={:?}/sector{}/layer{} mid={:?} in={:?}/sector{}/layer{}",
                    lift.motion_area_index,
                    lift.lift_type,
                    lift.direction,
                    door.active,
                    door.door_sector.points,
                    door.point_out,
                    door.sector_out,
                    door.layer_out,
                    door.point_mid,
                    door.point_in,
                    door.sector_in,
                    door.layer_in,
                );
            }
        }
    }

    for (pair_index, pair) in loaded.proto.jump_line_pairs.iter().enumerate() {
        let points = [
            pair.line1.point_a,
            pair.line1.point_b,
            pair.line2.point_a,
            pair.line2.point_b,
        ];
        if points
            .into_iter()
            .any(|(x, y, _)| (1600..=2050).contains(&x) && (900..=1250).contains(&y))
        {
            eprintln!(
                "jump_pair={pair_index} long={} line1={:?} zone1={} line2={:?} zone2={}",
                pair.jump_long,
                (pair.line1.point_a, pair.line1.point_b),
                pair.line1.jump_zone_index,
                (pair.line2.point_a, pair.line2.point_b),
                pair.line2.jump_zone_index
            );
        }
    }

    for (zone_index, zone) in loaded.proto.jump_zones.iter().enumerate().take(2) {
        eprintln!(
            "jump_zone={zone_index} sector={} layer={} helper={} polygon={:?}",
            zone.sector, zone.layer, zone.helper_needed, zone.polygon.points
        );
    }
}
