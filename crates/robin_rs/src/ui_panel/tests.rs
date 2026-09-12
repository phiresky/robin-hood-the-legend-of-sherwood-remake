#[test]
fn picture_hit_mask_decodes_every_rgb565_word_and_validates_payloads() {
    let mut pic = Picture {
        width: 256,
        height: 256,
        pitch: 512,
        pixel_format: robin_assets::picture::PixelFormat::Rgb16,
        data: (0..=u16::MAX).flat_map(u16::to_le_bytes).collect(),
        palette: None,
    };
    for transparent in [0, crate::renderer::TRANSPARENT_COLOR_KEY_16, u16::MAX] {
        let mask = picture_hit_mask(&pic, transparent).unwrap();
        for pixel in 0..=u16::MAX {
            assert_eq!(
                mask.is_opaque(pixel % 256, pixel / 256),
                pixel != transparent
            );
        }
    }
    pic.data.pop();
    assert!(
        picture_hit_mask(&pic, 0)
            .unwrap_err()
            .to_string()
            .contains("incomplete pixel")
    );
    pic.data.pop();
    assert!(
        picture_hit_mask(&pic, 0)
            .unwrap_err()
            .to_string()
            .contains("dimensions")
    );
    pic.data.extend_from_slice(&[255, 255]);
    pic.pixel_format = robin_assets::picture::PixelFormat::Rgb24;
    assert!(
        picture_hit_mask(&pic, 0)
            .unwrap_err()
            .to_string()
            .contains("RGB565")
    );
}

fn queue_item(target: PortraitTarget, members: &[u32]) -> PortraitBarItem<'static> {
    let members = Cow::Owned(
        members
            .iter()
            .map(|id| EntityId::Soldier(robin_engine::entity_id::SoldierId(*id)))
            .collect(),
    );
    match target {
        PortraitTarget::AlliedGroup(id) => PortraitBarItem::AlliedGroup { id, members },
        PortraitTarget::AlliedSelection => PortraitBarItem::AlliedSelection(members),
        PortraitTarget::Pc(_) => panic!("queue_item fixture expects an allied target"),
    }
}

#[test]
fn overlapping_group_strips_have_independent_history_and_survive_reordering() {
    let first = queue_item(PortraitTarget::AlliedGroup(1), &[7, 8]);
    let second = queue_item(PortraitTarget::AlliedGroup(2), &[7, 9]);
    let selection = queue_item(PortraitTarget::AlliedSelection, &[7, 8]);
    let a = first.queue_strip_identity();
    let b = second.queue_strip_identity();
    let c = selection.queue_strip_identity();
    assert_ne!(a, b);
    assert_ne!(a, c);
    let mut animations = crate::host::QueueStripAnimations::default();
    let seat = PlayerId::HOST;
    animations.prepare_fixed_tick(seat, [(a.clone(), 4), (b.clone(), 8), (c.clone(), 4)]);
    // Reordering portrait positions cannot transfer another strip's offset.
    animations.prepare_fixed_tick(seat, [(b.clone(), 8), (c.clone(), 4), (a.clone(), 3)]);
    assert_eq!(animations.displayed_offset(seat, &a, 3), 10);
    assert_eq!(animations.displayed_offset(seat, &b, 8), 0);
    assert_eq!(animations.displayed_offset(seat, &c, 4), 0);
    for expected in [8, 6, 4, 2, 0, 0] {
        animations.prepare_fixed_tick(seat, [(a.clone(), 3), (b.clone(), 8), (c.clone(), 4)]);
        for _ in 0..20 {
            assert_eq!(animations.displayed_offset(seat, &a, 3), expected);
        }
    }
}

#[test]
fn hero_portrait_members_borrow_its_inline_entity_id() {
    let id = EntityId::Pc(robin_engine::entity_id::PcId(3));
    let item = PortraitBarItem::Pc(id);
    let PortraitBarItem::Pc(stored_id) = &item else {
        unreachable!();
    };
    assert_eq!(item.target(), PortraitTarget::Pc(id));
    assert_eq!(item.members(), &[id]);
    assert_eq!(item.members().as_ptr(), std::ptr::from_ref(stored_id));
    assert_eq!(
        item.queue_strip_identity(),
        crate::host::QueueStripIdentity::Pc(id)
    );
    assert_eq!(item.clone().members(), &[id]);
}

#[test]
fn borrowed_portrait_members_keep_order_while_identity_owns_a_sorted_copy() {
    let source = queue_item(PortraitTarget::AlliedSelection, &[8, 7]);
    let item = PortraitBarItem::AlliedSelection(Cow::Borrowed(source.members()));
    assert_eq!(item.members().as_ptr(), source.members().as_ptr());
    let copy = item.clone();
    assert!(matches!(
        &copy,
        PortraitBarItem::AlliedSelection(Cow::Borrowed(_))
    ));
    assert_eq!(copy.members().as_ptr(), source.members().as_ptr());
    let identity = item.queue_strip_identity();
    assert_eq!(
        identity,
        queue_item(PortraitTarget::AlliedSelection, &[7, 8]).queue_strip_identity()
    );
    assert_eq!(
        item.members(),
        queue_item(PortraitTarget::AlliedSelection, &[8, 7]).members()
    );
    drop(copy);
    drop(item);
    drop(source);
    // The key can outlive the borrowed portrait query.
    assert_eq!(
        identity,
        queue_item(PortraitTarget::AlliedSelection, &[7, 8]).queue_strip_identity()
    );
}

#[test]
fn selection_identity_ignores_member_order_but_not_membership() {
    let a = queue_item(PortraitTarget::AlliedSelection, &[7, 8]);
    let reordered = queue_item(PortraitTarget::AlliedSelection, &[8, 7]);
    let replaced = queue_item(PortraitTarget::AlliedSelection, &[7, 9]);
    assert_eq!(a.queue_strip_identity(), reordered.queue_strip_identity());
    assert_ne!(a.queue_strip_identity(), replaced.queue_strip_identity());
    let pinned = queue_item(PortraitTarget::AlliedGroup(1), &[7, 8]);
    let changed_pinned = queue_item(PortraitTarget::AlliedGroup(1), &[8]);
    assert_eq!(
        pinned.queue_strip_identity(),
        changed_pinned.queue_strip_identity()
    );
}

#[test]
fn removed_strips_and_changed_seats_start_with_fresh_history() {
    let key = queue_item(PortraitTarget::AlliedGroup(1), &[7]).queue_strip_identity();
    let mut animations = crate::host::QueueStripAnimations::default();
    let seat = PlayerId::HOST;
    assert_eq!(animations.displayed_offset(seat, &key, 2), 0);
    animations.prepare_fixed_tick(seat, [(key.clone(), 5)]);
    animations.prepare_fixed_tick(seat, []);
    animations.prepare_fixed_tick(seat, [(key.clone(), 2)]);
    assert_eq!(animations.displayed_offset(seat, &key, 2), 0);
    animations.prepare_fixed_tick(seat, [(key.clone(), 1)]);
    assert_eq!(animations.displayed_offset(seat, &key, 1), 10);
    let other_seat = PlayerId(2);
    assert_eq!(animations.displayed_offset(other_seat, &key, 1), 0);
    animations.prepare_fixed_tick(other_seat, [(key.clone(), 1)]);
    assert_eq!(animations.displayed_offset(other_seat, &key, 1), 0);
    animations.prepare_fixed_tick(seat, [(key.clone(), 1)]);
    assert_eq!(animations.displayed_offset(seat, &key, 1), 0);
}

#[test]
fn snapshot_reset_retires_queue_history_before_first_capture() {
    let mut host = crate::host::Host::scratch(640.0, 480.0);
    let key = queue_item(PortraitTarget::AlliedGroup(1), &[7]).queue_strip_identity();
    host.frontend
        .prepare_queue_strip_animations(PlayerId::HOST, [(key.clone(), 5)]);
    host.frontend
        .reset_interaction(crate::host::InteractionReset::SnapshotRestored);
    assert_eq!(
        host.frontend
            .queue_strip_animations()
            .displayed_offset(PlayerId::HOST, &key, 1),
        0
    );
    host.frontend
        .prepare_queue_strip_animations(PlayerId::HOST, [(key.clone(), 1)]);
    assert_eq!(
        host.frontend
            .queue_strip_animations()
            .displayed_offset(PlayerId::HOST, &key, 1),
        0
    );
}

#[test]
fn required_ui_assets_follow_the_supplied_preparation() {
    let make = |bytes: &[u8]| {
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset("Data/Interface/UI/isolated-test.png", bytes.to_vec())
            .unwrap();
        robin_engine::sbfile::SbFileSystem::new(vfs)
    };
    let first = make(b"first");
    let second = make(b"second");
    assert_eq!(
        super::read_ui_asset("isolated-test.png", &first).unwrap(),
        b"first"
    );
    assert_eq!(
        super::read_ui_asset("isolated-test.png", &second).unwrap(),
        b"second"
    );
    assert_eq!(
        super::read_ui_asset("isolated-test.png", &first).unwrap(),
        b"first"
    );
}
use super::*;

#[test]
fn action_icon_banks_keep_every_state_and_action_in_its_own_slot() {
    let mut cache = PortraitCache::new();
    let kind = CharacterKind::VARIANTS[0];
    cache.action_surfaces[kind.as_index()] = Some(std::array::from_fn(|state| {
        std::array::from_fn(|action| {
            Some(OwnedSurface::synthetic((state * 3 + action + 1) as u32).handle())
        })
    }));
    for (index, state) in [
        ActionButtonVisual::Disabled,
        ActionButtonVisual::Normal,
        ActionButtonVisual::Hover,
        ActionButtonVisual::Pressed,
        ActionButtonVisual::HoverPressed,
    ]
    .into_iter()
    .enumerate()
    {
        let icons = cache.action_icons(kind, state).unwrap();
        for (action, icon) in icons.iter().enumerate() {
            assert_eq!(
                *icon,
                Some(OwnedSurface::synthetic((index * 3 + action + 1) as u32).handle())
            );
        }
    }
}

#[test]
fn portrait_pages_match_rotate_then_truncate_without_copying_group_members() {
    use robin_engine::entity_id::{PcId, SoldierId};
    let pcs = [EntityId::Pc(PcId(1)), EntityId::Pc(PcId(2))];
    let soldiers = [
        EntityId::Soldier(SoldierId(7)),
        EntityId::Soldier(SoldierId(8)),
    ];
    let groups = [
        TacticalPinnedGroup {
            id: 3,
            members: soldiers.to_vec(),
        },
        TacticalPinnedGroup {
            id: 4,
            members: vec![soldiers[1]],
        },
    ];
    let reversed = [soldiers[1], soldiers[0]];
    for pc_count in 0..=pcs.len() {
        for group_count in 0..=groups.len() {
            let pcs = &pcs[..pc_count];
            let groups = &groups[..group_count];
            for selection in [&[][..], &soldiers[..], &reversed[..], &soldiers[1..]] {
                let mut all: Vec<_> = pcs
                    .iter()
                    .map(|&pc| (PortraitTarget::Pc(pc), vec![pc]))
                    .collect();
                all.extend(
                    groups.iter().map(|group| {
                        (PortraitTarget::AlliedGroup(group.id), group.members.clone())
                    }),
                );
                if !selection.is_empty() && !groups.iter().any(|group| group.members == selection) {
                    all.push((PortraitTarget::AlliedSelection, selection.to_vec()));
                }
                for capacity in 0..=6 {
                    for first in [0, 1, 2, 3, 4, 5, 6, usize::MAX] {
                        let mut expected = all.clone();
                        let expected_paged = expected.len() > capacity;
                        if expected_paged {
                            let offset = first % expected.len();
                            expected.rotate_left(offset);
                            expected.truncate(capacity);
                        }
                        let (actual, paged) =
                            build_portrait_page(pcs, groups, selection, capacity, first);
                        assert_eq!(paged, expected_paged);
                        let values: Vec<_> = actual
                            .iter()
                            .map(|item| (item.target(), item.members().to_vec()))
                            .collect();
                        assert_eq!(values, expected);
                        for item in actual {
                            let source = match item.target() {
                                PortraitTarget::Pc(_) => continue,
                                PortraitTarget::AlliedGroup(id) => groups
                                    .iter()
                                    .find(|group| group.id == id)
                                    .unwrap()
                                    .members
                                    .as_slice(),
                                PortraitTarget::AlliedSelection => selection,
                            };
                            assert!(matches!(
                                &item,
                                PortraitBarItem::AlliedSelection(Cow::Borrowed(_))
                                    | PortraitBarItem::AlliedGroup {
                                        members: Cow::Borrowed(_),
                                        ..
                                    }
                            ));
                            assert_eq!(item.members().as_ptr(), source.as_ptr());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn localized_name_reload_moves_translations_but_retains_existing_generated_names() {
    for existing in [false, true] {
        for incoming in [false, true] {
            let old: [Option<String>; CharacterKind::COUNT] = std::array::from_fn(|index| {
                existing.then(|| {
                    if index == CharacterKind::MerryManB.as_index() {
                        String::new()
                    } else {
                        format!("old {index}")
                    }
                })
            });
            let new: [Option<String>; CharacterKind::COUNT] =
                std::array::from_fn(|index| incoming.then(|| format!("translated {index}")));
            let old_pointers = old
                .each_ref()
                .map(|name| name.as_ref().map(|name| name.as_ptr()));
            let new_pointers = new
                .each_ref()
                .map(|name| name.as_ref().map(|name| name.as_ptr()));
            let expected_old = old.clone();
            let expected_new = new.clone();
            let mut cache = PortraitCache::new();
            cache.install_localized_names(old);
            cache.reload_localized_names_preserving_generated(new);
            for kind in CharacterKind::VARIANTS {
                let index = kind.as_index();
                let preserve = existing
                    && matches!(
                        kind,
                        CharacterKind::MerryManA
                            | CharacterKind::MerryManB
                            | CharacterKind::MerryManC
                    );
                let (expected, pointer) = if preserve {
                    (&expected_old[index], old_pointers[index])
                } else {
                    (&expected_new[index], new_pointers[index])
                };
                assert_eq!(&cache.localized_names[index], expected, "{kind:?}");
                assert_eq!(
                    cache.localized_names[index]
                        .as_ref()
                        .map(|name| name.as_ptr()),
                    pointer,
                    "{kind:?}"
                );
            }
        }
    }
}

#[test]
fn portrait_capacity_tracks_available_width() {
    assert_eq!(portrait_capacity(640), 5);
    assert_eq!(portrait_capacity(800), 6);
    assert_eq!(portrait_capacity(1024), 8);
    assert_eq!(portrait_capacity(1280), 10);
}

#[test]
fn slot_left_positions_at_800() {
    // Each slot center should contain the 112px element
    let sw = 800u16;
    for slot in 0..5 {
        let left = slot_left_x(sw, slot, portrait_slot_count(sw, 5));
        let right = left + ELEMENT_WIDTH;
        assert!(
            left >= MARGIN || slot == 0,
            "slot {} starts before margin",
            slot
        );
        assert!(
            right <= sw - MARGIN || slot == 4,
            "slot {} extends past margin",
            slot
        );
    }
}

#[test]
fn few_portraits_spread_across_bar() {
    // With five or fewer portraits the bar keeps the original
    // five-slot layout: slots span the full width instead of
    // packing element-width slots into the left corner.
    for num_items in 1..=5 {
        assert_eq!(portrait_slot_count(800, num_items), 5);
    }
    let slot_width = slot_left_x(800, 1, 5) - slot_left_x(800, 0, 5);
    assert!(
        slot_width > ELEMENT_WIDTH,
        "five-slot layout should leave gaps between portraits"
    );
    // More items than the minimum divide the bar by the item count.
    assert_eq!(portrait_slot_count(800, 6), 6);
    assert_eq!(portrait_slot_count(1024, 7), 7);
}

#[test]
fn portrait_total_height() {
    // 3 + 23 + 35 + 50 + 23 = 134
    assert_eq!(PORTRAIT_TOTAL_HEIGHT, 134);
}

#[test]
fn position_stack() {
    // Verify the position constants stack correctly from bottom
    assert_eq!(POSITION_BOTTOM_SCROLL, 26); // 3 + 23
    assert_eq!(POSITION_ACTION, 61); // 26 + 35
    assert_eq!(POSITION_VISAGE, 111); // 61 + 50
    assert_eq!(POSITION_TOP_SCROLL, 134); // 111 + 23
}

#[test]
fn bbox_construction() {
    let b = bbox(10, 20, 30, 40);
    assert_eq!(b.min.x, 10.0);
    assert_eq!(b.min.y, 20.0);
    assert_eq!(b.max.x, 30.0);
    assert_eq!(b.max.y, 40.0);
}

#[test]
fn action_button_visual_matches_widget_priority() {
    assert_eq!(
        action_button_visual(false, false, false),
        ActionButtonVisual::Normal
    );
    assert_eq!(
        action_button_visual(false, false, true),
        ActionButtonVisual::Hover
    );
    assert_eq!(
        action_button_visual(false, true, true),
        ActionButtonVisual::Disabled
    );
    assert_eq!(
        action_button_visual(true, false, true),
        ActionButtonVisual::HoverPressed
    );
    assert_eq!(
        action_button_visual(true, true, true),
        ActionButtonVisual::Disabled
    );
}

#[test]
fn allied_villain_profiles_select_named_visages() {
    for (filename, expected) in [
        ("Guisbourne", AlliedVisageKind::Guisbourne),
        ("Longchamp", AlliedVisageKind::Longchamp),
        ("PrinceJohn", AlliedVisageKind::PrinceJohn),
        ("Scatlock", AlliedVisageKind::Scathlock),
        ("sherif", AlliedVisageKind::Sheriff),
        ("Sherif", AlliedVisageKind::Sheriff),
        ("Soldier A03", AlliedVisageKind::Generic),
    ] {
        assert_eq!(AlliedVisageKind::from_profile_filename(filename), expected);
    }
}

#[test]
fn action_button_sub_ids_cover_classic_radio_and_rdo_hover() {
    assert_eq!(ACTION_SUB_ID_DISABLED, 0);
    assert_eq!(ACTION_SUB_ID_UNSELECTED, 1);
    assert_eq!(ACTION_SUB_ID_FOCUSED, 2);
    assert_eq!(ACTION_SUB_ID_SELECTED, 3);
    assert_eq!(ACTION_SUB_ID_FOCUSED_SELECTED, 4);
}

#[test]
fn allied_action_row_has_three_full_height_hit_columns() {
    assert_eq!(allied_action_index(0.0), 0);
    assert_eq!(allied_action_index(37.2), 0);
    assert_eq!(allied_action_index(37.4), 1);
    assert_eq!(allied_action_index(74.5), 1);
    assert_eq!(allied_action_index(74.7), 2);
    assert_eq!(allied_action_index(112.0), 2);
}

#[test]
fn portrait_action_tooltip_appears_quickly_and_resets_between_cells() {
    let mut tracker = PcActionTooltipTracker::new();
    for _ in 0..PC_ACTION_TOOLTIP_DELAY_TICKS {
        tracker.update(Some((2, 1)));
    }
    assert_eq!(tracker.ready_button(), Some((2, 1)));
    tracker.update(Some((2, 2)));
    assert_eq!(tracker.ready_button(), None);
}

#[test]
fn stone_preview_controls_direct_hit_and_noise_explanations_independently() {
    use robin_engine::gameplay_config::{ItemGameplayConfig, ItemPreviewConfig};
    use robin_engine::profiles::Action;

    let direct_only = ItemPreviewConfig {
        stone_direct_effect: true,
        ..ItemPreviewConfig::classic()
    };
    let (_, direct_text) =
        item_action_tooltip_extension(Action::Stone, ItemGameplayConfig::default(), direct_only)
            .expect("direct stone explanation");
    assert!(direct_text.contains("Direct hit"));
    assert!(!direct_text.contains("noise"));

    let noise_only = ItemPreviewConfig {
        stone_distraction_area: true,
        ..ItemPreviewConfig::classic()
    };
    let (_, noise_text) =
        item_action_tooltip_extension(Action::Stone, ItemGameplayConfig::default(), noise_only)
            .expect("stone noise explanation");
    assert!(noise_text.contains("Ground noise"));
    assert!(!noise_text.contains("Direct hit"));
}

#[test]
fn net_preview_controls_capture_area_and_crumple_explanations_independently() {
    use robin_engine::gameplay_config::{ItemGameplayConfig, ItemPreviewConfig};
    use robin_engine::profiles::Action;

    let capture_only = ItemPreviewConfig {
        net_capture_area: true,
        ..ItemPreviewConfig::classic()
    };
    let (_, capture_text) =
        item_action_tooltip_extension(Action::Net, ItemGameplayConfig::classic(), capture_only)
            .expect("net capture explanation");
    assert!(capture_text.contains("within 40"));
    assert!(!capture_text.contains("crumple"));

    let crumple_only = ItemPreviewConfig {
        net_crumple_prediction: true,
        ..ItemPreviewConfig::classic()
    };
    let (_, crumple_text) =
        item_action_tooltip_extension(Action::Net, ItemGameplayConfig::classic(), crumple_only)
            .expect("net crumple explanation");
    assert!(crumple_text.contains("crumple"));
    assert!(!crumple_text.contains("within 40"));
}

#[test]
fn item_tooltips_explain_selective_net_and_reliable_ale_without_changing_previews() {
    use robin_engine::gameplay_config::{ItemGameplayConfig, ItemPreviewConfig};
    use robin_engine::profiles::Action;

    let net_preview = ItemPreviewConfig {
        net_capture_area: true,
        net_crumple_prediction: true,
        ..ItemPreviewConfig::classic()
    };
    let (_, net_text) =
        item_action_tooltip_extension(Action::Net, ItemGameplayConfig::default(), net_preview)
            .expect("selective net explanation");
    assert!(net_text.contains("skipped"));
    assert!(net_text.contains("terrain can crumple"));
    assert!(!net_text.contains("people can crumple"));

    let ale_preview = ItemPreviewConfig {
        ale_effect: true,
        ..ItemPreviewConfig::classic()
    };
    let (_, reliable_text) =
        item_action_tooltip_extension(Action::Ale, ItemGameplayConfig::default(), ale_preview)
            .expect("reliable ale explanation");
    let (_, classic_text) =
        item_action_tooltip_extension(Action::Ale, ItemGameplayConfig::classic(), ale_preview)
            .expect("classic ale explanation");
    assert!(reliable_text.contains("potency 20"));
    assert!(classic_text.contains("authored beer interest"));
}

#[test]
fn embedded_png_rejects_unsupported_headers_before_decoding_pixels() {
    for (width, height, color, depth, expected) in [
        (
            65_536,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            "width exceeds u16",
        ),
        (
            1,
            65_536,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            "height exceeds u16",
        ),
        (
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Sixteen,
            "unsupported bit depth",
        ),
        (
            1,
            1,
            png::ColorType::Rgba,
            png::BitDepth::Sixteen,
            "unsupported bit depth",
        ),
        (
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            "unsupported color type",
        ),
    ] {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap();
        // An invalid pixel body proves header validation happens first.
        writer.write_chunk(png::chunk::IDAT, &[]).unwrap();
        drop(writer);
        let error = decode_embedded_png_rgba(&bytes).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn embedded_png_preserves_rgba_and_expands_rgb() {
    for (color, source, expected) in [
        (
            png::ColorType::Rgb,
            vec![1, 2, 3, 4, 5, 6],
            vec![1, 2, 3, 255, 4, 5, 6, 255],
        ),
        (
            png::ColorType::Rgba,
            vec![1, 2, 3, 0, 4, 5, 6, 127],
            vec![1, 2, 3, 0, 4, 5, 6, 127],
        ),
    ] {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&source).unwrap();
        writer.finish().unwrap();
        assert_eq!(decode_embedded_png_rgba(&bytes).unwrap(), (2, 1, expected));
    }
    assert!(decode_embedded_png_rgba(b"not a PNG").is_err());
}

#[test]
fn embedded_allied_portrait_layers_preserve_full_alpha() {
    let mut found_partial_alpha = false;
    for bytes in [
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_portrait_background.png"
        )) as &[u8],
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_portrait_foreground.png"
        )),
    ] {
        let (width, height, pixels) = decode_embedded_png_rgba(bytes).unwrap();
        assert_eq!((width, height), (ELEMENT_WIDTH, PORTRAIT_TOTAL_HEIGHT));
        assert_eq!(pixels.len(), usize::from(width) * usize::from(height) * 4);
        assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0));
        found_partial_alpha |= pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| (1..=254).contains(&pixel[3]));
    }
    assert!(found_partial_alpha);
}

#[test]
fn embedded_allied_pin_icons_decode_with_transparency() {
    for bytes in [
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_pin_unpinned.png"
        )) as &[u8],
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_pin_pinned.png"
        )),
    ] {
        let (width, height, pixels) = decode_embedded_png_rgba(bytes).unwrap();
        assert_eq!(
            (width, height),
            (ALLIED_PIN_ICON_SIZE, ALLIED_PIN_ICON_SIZE)
        );
        let pixels = pixels.as_chunks::<4>().0;
        assert!(pixels.iter().any(|pixel| pixel[3] == 0));
        assert!(pixels.iter().any(|pixel| pixel[3] == 255));
        assert!(pixels.iter().any(|pixel| (1..=254).contains(&pixel[3])));
    }
}

#[test]
fn embedded_allied_state_icons_decode_with_transparency() {
    for bytes in [
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_stance_hold.png"
        )) as &[u8],
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_stance_defensive.png"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_stance_aggressive.png"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_patrol_off.png"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_patrol_on.png"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_formation_line.png"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_formation_box.png"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_formation_staggered.png"
        )),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/core-datadir/Data/Interface/UI/allied_formation_flank.png"
        )),
    ] {
        let (width, height, pixels) = decode_embedded_png_rgba(bytes).unwrap();
        assert_eq!(
            (width, height),
            (ALLIED_ACTION_ICON_WIDTH, ALLIED_ACTION_ICON_HEIGHT)
        );
        let pixels = pixels.as_chunks::<4>().0;
        assert!(pixels.iter().any(|pixel| pixel[3] == 0));
        assert!(pixels.iter().any(|pixel| pixel[3] == 255));
        assert!(pixels.iter().any(|pixel| (1..=254).contains(&pixel[3])));
    }
}

// The CharacterKind resource-lookup and sub-id tests live in
// `robin_engine::character_kind`; the UI-panel side just delegates to
// those methods, so no duplicate tests are needed here.

#[test]
fn hit_test_outside_panel() {
    // Click above the panel area
    assert_eq!(hit_test_portrait(800, 600, 400.0, 100.0, 3), None);
}

#[test]
fn hit_test_on_portrait_slot() {
    // screen 800x600, slot 0 starts at slot_left_x(800, 0)
    let x = slot_left_x(800, 0, portrait_slot_count(800, 3)) as f32 + 10.0;
    let y = 600.0 - 50.0; // within the panel area
    assert_eq!(hit_test_portrait(800, 600, x, y, 3), Some(0));
}

#[test]
fn hit_test_empty_slots() {
    // No PCs means no hits even inside the panel
    let x = slot_left_x(800, 0, portrait_slot_count(800, 0)) as f32 + 10.0;
    let y = 600.0 - 50.0;
    assert_eq!(hit_test_portrait(800, 600, x, y, 0), None);
}

#[test]
fn hit_test_between_slots() {
    // Click between slot boundaries (in the gap)
    let x0_right =
        slot_left_x(800, 0, portrait_slot_count(800, 3)) as f32 + ELEMENT_WIDTH as f32 + 5.0;
    let y = 600.0 - 50.0;
    let x1_left = slot_left_x(800, 1, portrait_slot_count(800, 3)) as f32;
    // Only a gap hit if the click is truly between elements
    if x0_right < x1_left {
        assert_eq!(hit_test_portrait(800, 600, x0_right, y, 3), None);
    }
}

#[test]
fn hit_test_requirements_bar_maps_screen_coords_to_slots() {
    use crate::widget::requirements::{RequirementSlot, RequirementStatus, RequirementsState};
    use robin_engine::profiles::Action;
    let state = RequirementsState {
        slots: vec![
            RequirementSlot::RequiredCharacter {
                character_profile_idx: engine_profiles::CharacterProfileIdx(1),
                status: RequirementStatus::Fulfilled,
                selected: false,
            },
            RequirementSlot::RequiredAction {
                action: Action::Bow,
                status: RequirementStatus::Fulfilled,
                selected: false,
            },
        ],
        all_fulfilled: true,
    };
    // Strip is centered in the box (40, screen_w - 40).  For 800px:
    // box_w = 720, needed_w = 2*(40+10) - 10 = 90, start_x =
    // 40 + (720 - 90)/2 = 355.  Step = 50px between slot origins.
    let step = (REQ_BAR_ICON_MARGIN + REQ_BAR_ICON_W) as i32;
    let start_x = requirements_bar_start_x(800, state.slots.len()).unwrap();
    let y_in = (REQ_BAR_Y + REQ_BAR_ICON_H / 2) as i32;
    let slot0_cx = start_x + REQ_BAR_ICON_W as i32 / 2;
    let slot1_cx = slot0_cx + step;
    assert_eq!(start_x, 355);
    assert_eq!(
        hit_test_requirements_bar(800, &state, ScreenPoint::new(slot0_cx as f32, y_in as f32)),
        Some(0)
    );
    assert_eq!(
        hit_test_requirements_bar(800, &state, ScreenPoint::new(slot1_cx as f32, y_in as f32)),
        Some(1)
    );
    // In the margin between slot 0 and slot 1.
    let gap_x = start_x + REQ_BAR_ICON_W as i32 + 1;
    assert_eq!(
        hit_test_requirements_bar(800, &state, ScreenPoint::new(gap_x as f32, y_in as f32)),
        None
    );
    // Below the bar.
    assert_eq!(
        hit_test_requirements_bar(800, &state, ScreenPoint::new(slot0_cx as f32, 200.0)),
        None
    );
}

#[test]
fn requirements_bar_centered_in_box() {
    // The strip is centered inside the (40, w-40) box.  A 2-slot
    // strip on a 1024px screen: box_w = 944, needed_w = 90, start_x
    // = 40 + (944 - 90)/2 = 467.
    assert_eq!(requirements_bar_start_x(1024, 2), Some(467));
    // 0 slots = no strip to center.
    assert_eq!(requirements_bar_start_x(1024, 0), None);
    // Narrow screen with negative box_w falls back to None.
    assert_eq!(requirements_bar_start_x(40, 2), None);
}

#[test]
fn requirements_slot_tooltip_mt_id_matches_table() {
    use crate::ingame_menu::resources::{
        MT_INFOBULLE_QG_NEEDED_ACTION, MT_INFOBULLE_QG_NEEDED_PC, MT_INFOBULLE_QG_OTHER_PC,
    };
    use crate::widget::requirements::{RequirementSlot, RequirementStatus};
    use robin_engine::profiles::Action;
    assert_eq!(
        requirements_slot_tooltip_mt_id(&RequirementSlot::RequiredCharacter {
            character_profile_idx: engine_profiles::CharacterProfileIdx(1),
            status: RequirementStatus::Fulfilled,
            selected: false,
        }),
        MT_INFOBULLE_QG_NEEDED_PC
    );
    assert_eq!(
        requirements_slot_tooltip_mt_id(&RequirementSlot::RequiredAction {
            action: Action::Bow,
            status: RequirementStatus::Fulfilled,
            selected: false,
        }),
        MT_INFOBULLE_QG_NEEDED_ACTION
    );
    assert_eq!(
        requirements_slot_tooltip_mt_id(&RequirementSlot::OptionalCharacter {
            character_profile_idx: Some(engine_profiles::CharacterProfileIdx(2)),
        }),
        MT_INFOBULLE_QG_OTHER_PC
    );
    assert_eq!(
        requirements_slot_tooltip_mt_id(&RequirementSlot::OptionalCharacter {
            character_profile_idx: None,
        }),
        MT_INFOBULLE_QG_OTHER_PC
    );
}

#[test]
fn typed_hover_tracker_preserves_delay_reset_and_saturation() {
    use crate::corner_hud::CornerButton;
    let mut tracker = HoverTooltipTracker::<CornerButton>::new();
    assert_eq!(tracker.ready_slot(), None);
    tracker.update(Some(CornerButton::Sight));
    for _ in 0..REQUIREMENTS_TOOLTIP_DELAY_TICKS {
        tracker.update(Some(CornerButton::Sight));
    }
    assert_eq!(tracker.ready_slot(), None);
    tracker.update(Some(CornerButton::Sight));
    assert_eq!(tracker.ready_slot(), Some(CornerButton::Sight));
    tracker.hover_ticks = u32::MAX;
    tracker.update(Some(CornerButton::Sight));
    assert_eq!(tracker.hover_ticks, u32::MAX);
    assert_eq!(tracker.ready_slot(), Some(CornerButton::Sight));
    tracker.update(Some(CornerButton::Clock));
    assert_eq!(tracker.ready_slot(), None);
    assert_eq!(tracker.hover_ticks, 0);
    tracker.update(None);
    assert_eq!(tracker.ready_slot(), None);
}

#[test]
fn requirements_tooltip_tracker_counts_ticks() {
    let mut t = RequirementsTooltipTracker::new();
    assert!(t.ready_slot().is_none());

    // First update arms the counter at 0 (timer resets when the
    // focus changes).  Not yet ready.
    t.update(Some(0));
    assert!(t.ready_slot().is_none());

    // Bump up to (but not past) the threshold — the comparison is
    // strictly greater-than, so 75 ticks still means "not ready".
    for _ in 0..REQUIREMENTS_TOOLTIP_DELAY_TICKS {
        t.update(Some(0));
    }
    assert_eq!(t.ready_slot(), None);

    // One more tick crosses the threshold.
    t.update(Some(0));
    assert_eq!(t.ready_slot(), Some(0));

    // Switching slots resets the counter.
    t.update(Some(1));
    assert!(t.ready_slot().is_none());

    // Leaving the bar clears the tracker entirely.
    t.update(None);
    assert!(t.ready_slot().is_none());
}

#[test]
fn portrait_cache_empty() {
    let cache = PortraitCache::new();
    assert!(!cache.is_loaded());
    let robin = CharacterKind::RobinHood { is_town: false };
    assert_eq!(cache.get_surface(robin), None);
    assert!(
        cache
            .action_icons(robin, ActionButtonVisual::Normal)
            .is_none()
    );
    assert!(cache.get_localized_name(robin).is_none());
    assert_eq!(
        cache.get_sub_picture(resource_ids::RHID_REQUIRED_PC, 1),
        None,
    );
}

#[test]
fn required_action_sub_ids_match_table() {
    use robin_engine::profiles::Action;
    // Sub-id table:
    // UnknownAction=0, Bow=1, Carry=2, Climb=3, Jump=4, Lever=5,
    // Lockpick=6, Stun=7, Tie=8, Eat=9, Search=10.
    assert_eq!(required_action_sub_id(Action::Bow), 1);
    assert_eq!(required_action_sub_id(Action::LittleJohnCarry), 2);
    assert_eq!(required_action_sub_id(Action::FarmerCarry), 2);
    assert_eq!(required_action_sub_id(Action::Climb), 3);
    assert_eq!(required_action_sub_id(Action::Jump), 4);
    assert_eq!(required_action_sub_id(Action::Lever), 5);
    assert_eq!(required_action_sub_id(Action::Lockpick), 6);
    assert_eq!(required_action_sub_id(Action::Hit), 7);
    assert_eq!(required_action_sub_id(Action::HitHard), 7);
    assert_eq!(required_action_sub_id(Action::Tie), 8);
    assert_eq!(required_action_sub_id(Action::Eat), 9);
    assert_eq!(required_action_sub_id(Action::Guzzle), 9);
    assert_eq!(required_action_sub_id(Action::Search), 10);
}
/// Runs inside the named headless GPU gate, using two live renderer identities.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn verify_portrait_gpu_ownership(renderer: &mut Renderer, other: &mut Renderer) {
    let picture = Picture {
        width: 1,
        height: 1,
        pitch: 2,
        pixel_format: robin_assets::picture::PixelFormat::Rgb16,
        data: vec![255, 255],
        palette: None,
    };
    let kind = CharacterKind::VARIANTS[0];
    let mut cache = PortraitCache::new();
    cache.localized_names[kind.as_index()] = Some("Retained name".into());
    let install = |cache: &mut PortraitCache, renderer: &mut Renderer| -> anyhow::Result<()> {
        let handle = owned_picture_surface(renderer, &mut cache.owned_surfaces, &picture)?;
        cache.surfaces[kind.as_index()] = Some(handle);
        // Authored sparse subframe indices must not collapse on replacement.
        cache.sub_pictures.insert((42, 3), handle);
        Ok(())
    };
    cache.replace_with(renderer, install).unwrap();
    let first = cache.get_surface(kind).unwrap();
    let mut peer = PortraitCache::new();
    peer.replace_with(other, install).unwrap();
    let foreign = peer.get_surface(kind).unwrap();
    assert_eq!(first.legacy_id(), foreign.legacy_id());
    assert_ne!(first, foreign);
    assert_eq!(cache.get_sub_picture(42, 3), Some(first));
    assert!(cache.get_sub_picture(42, 2).is_none());
    renderer.assert_legacy_adoption_rejected(first);
    assert!(
        renderer
            .try_delete_legacy_surface(first.legacy_id())
            .is_err()
    );
    assert!(other.draw_surface(first, None, None, 0).is_err());
    assert!(cache.retire(other).is_err());
    assert!(
        cache
            .replace_with(other, |_, _| panic!("must reject before loading"))
            .is_err()
    );
    assert_eq!(cache.get_surface(kind), Some(first));
    assert!(other.surface_dimensions(foreign).is_ok());
    peer.retire(other).unwrap();

    // A failed candidate has already uploaded a picture: it must be retired,
    // while the previous bank and metadata remain available.
    let mut failed = None;
    assert!(
        cache
            .replace_with(renderer, |candidate, renderer| {
                install(candidate, renderer)?;
                failed = candidate.get_surface(kind);
                let mut invalid = picture.clone();
                invalid.data.pop();
                owned_picture_surface(renderer, &mut candidate.owned_surfaces, &invalid)?;
                Ok(())
            })
            .is_err()
    );
    assert!(renderer.surface_dimensions(failed.unwrap()).is_err());
    assert_eq!(cache.get_surface(kind), Some(first));
    assert_eq!(cache.get_localized_name(kind), Some("Retained name"));

    // Exercise the public load failure as well: missing required PNG is an
    // error, unlike absent optional portraits in an empty resource manager.
    let files = robin_engine::sbfile::SbFileSystem::new(std::sync::Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    ));
    assert!(
        cache
            .load(&mut ResourceManager::new(), renderer, &files)
            .is_err()
    );
    assert_eq!(cache.get_surface(kind), Some(first));

    renderer
        .draw_surface(first, None, None, BLIT_SOURCE_TRANSPARENT)
        .unwrap();
    for _ in 0..3 {
        let previous = cache.get_surface(kind).unwrap();
        cache.replace_with(renderer, install).unwrap();
        assert!(renderer.surface_dimensions(previous).is_err());
        assert_eq!(cache.get_sub_picture(42, 3), cache.get_surface(kind));
    }
    let last = cache.get_surface(kind).unwrap();
    let assets = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
    let artwork = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/core-datadir/Data/Interface/UI");
    for entry in std::fs::read_dir(artwork).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "png") {
            assets
                .install_preloaded_asset(
                    format!(
                        "Data/Interface/UI/{}",
                        path.file_name().unwrap().to_str().unwrap()
                    ),
                    std::fs::read(&path).unwrap(),
                )
                .unwrap();
        }
    }
    let complete_files = robin_engine::sbfile::SbFileSystem::new(assets);
    for _ in 0..2 {
        cache
            .load(&mut ResourceManager::new(), renderer, &complete_files)
            .unwrap();
        assert!(
            !cache.is_loaded(),
            "absent optional portraits must clear stale slots"
        );
        assert!(cache.allied_portrait_background.is_some());
        assert!(cache.allied_visages.iter().all(Option::is_some));
        assert!(cache.allied_pin_icons.iter().all(Option::is_some));
        assert!(cache.allied_action_surfaces.iter().all(Option::is_some));
        // RGBA-only caches still require their originating renderer.
        assert!(cache.retire(other).is_err());
    }
    assert!(
        cache
            .load(&mut ResourceManager::new(), renderer, &files)
            .is_err()
    );
    assert!(cache.allied_portrait_background.is_some());
    cache.retire(renderer).unwrap();
    cache.retire(renderer).unwrap();
    assert!(renderer.surface_dimensions(last).is_err());
    assert!(!cache.is_loaded());
    assert!(cache.get_sub_picture(42, 3).is_none());
    assert_eq!(cache.get_localized_name(kind), Some("Retained name"));
    // A retired, empty cache can be loaded by a different renderer.
    cache.replace_with(other, install).unwrap();
    cache.retire(other).unwrap();
    // The queued portrait remains renderable after its owner was retired.
    assert_eq!(
        &renderer.try_capture_frame_rgba().unwrap().2[..4],
        &[248, 252, 248, 255] // Native RGB565 channel expansion uses left shifts.
    );
}
