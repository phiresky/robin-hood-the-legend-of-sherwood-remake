//! The private production views must actually carry the shared Engine borrow
//! exercised by the public compiler-negative tests. This structural assertion
//! avoids pretending that an inaccessible private type is a capability proof.

use syn::visit::{self, Visit};

#[test]
fn live_frame_authority_cannot_be_cloned_or_derived_from_diagnostics() {
    let syntax = syn::parse_file(include_str!("../../src/game_session/runtime.rs")).unwrap();
    let frame = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "MissionFrame" => Some(item),
            _ => None,
        })
        .expect("live frame transaction");
    for attribute in &frame.attrs {
        if attribute.path().is_ident("derive") {
            let derives = attribute
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .unwrap();
            for derive in derives {
                assert!(
                    !derive
                        .segments
                        .iter()
                        .any(|segment| segment.ident == "Clone" || segment.ident == "Deserialize"),
                    "derive data traits on the frame snapshot, not its live transaction authority"
                );
            }
        }
    }
    for item in &syntax.items {
        if let syn::Item::Impl(item) = item
            && let syn::Type::Path(ty) = item.self_ty.as_ref()
            && ty.path.is_ident("MissionFrame")
            && let Some((_, trait_path, _)) = &item.trait_
        {
            assert!(
                !trait_path
                    .segments
                    .iter()
                    .any(|segment| segment.ident == "Clone"),
                "applied cursors and recorder tokens must not be duplicated"
            );
        }
    }
}

#[test]
fn mission_journals_and_sprite_publication_are_private() {
    for (source, owner_name, fields) in [
        (
            include_str!("../../src/game_session/runtime.rs"),
            "MissionFrame",
            &[
                "commands",
                "post_commands",
                "external_actions",
                "post_external_actions",
                "external_facts",
            ][..],
        ),
        (
            include_str!("../../src/host.rs"),
            "HostFrontend",
            &["frame_holder"][..],
        ),
    ] {
        let syntax = syn::parse_file(source).unwrap();
        let owner = syntax
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == owner_name => Some(item),
                _ => None,
            })
            .expect("capability owner");
        for name in fields {
            let field = owner
                .fields
                .iter()
                .find(|field| field.ident.as_ref().is_some_and(|ident| ident == name))
                .unwrap_or_else(|| panic!("missing {owner_name}.{name}"));
            assert!(
                matches!(field.vis, syn::Visibility::Inherited),
                "{owner_name}.{name} must remain private"
            );
        }
    }
}

#[test]
fn ordinary_tick_effects_do_not_receive_aggregate_host_authority() {
    struct TickSignatures(usize);
    impl<'ast> Visit<'ast> for TickSignatures {
        fn visit_signature(&mut self, signature: &'ast syn::Signature) {
            if [
                "run_engine_frame_core",
                "run_engine_tick_core",
                "run_engine_tick",
                "run_post_initialize_stage",
                "run_post_initialize_stage_with_actions",
            ]
            .iter()
            .any(|name| signature.ident == name)
            {
                self.0 += 1;
                struct Arguments;
                impl<'ast> Visit<'ast> for Arguments {
                    fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
                        for segment in &ty.path.segments {
                            assert!(
                                !["Host", "HostTransport", "EngineManager"]
                                    .iter()
                                    .any(|name| segment.ident == name),
                                "ordinary effects must receive disjoint presentation authority"
                            );
                        }
                        visit::visit_type_path(self, ty);
                    }
                }
                for input in &signature.inputs {
                    Arguments.visit_fn_arg(input);
                }
            }
            visit::visit_signature(self, signature);
        }
    }
    let mut signatures = TickSignatures(0);
    for source in [
        include_str!("../../src/sim_timeline.rs"),
        include_str!("../../src/game.rs"),
    ] {
        signatures.visit_file(&syn::parse_file(source).unwrap());
    }
    assert_eq!(signatures.0, 5);
}

#[test]
fn replay_authority_has_no_process_singleton() {
    struct ReplayStatics;
    impl<'ast> Visit<'ast> for ReplayStatics {
        fn visit_item_static(&mut self, _: &'ast syn::ItemStatic) {
            panic!("replay authority must be application-owned, not static");
        }
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            assert_ne!(
                item.sig.ident, "process",
                "do not restore singleton replay authority"
            );
            visit::visit_item_fn(self, item);
        }
    }
    ReplayStatics
        .visit_file(&syn::parse_file(include_str!("../../src/replay_service.rs")).unwrap());
}

#[test]
fn timeline_reconciliation_and_history_are_private_owners() {
    let runtime = syn::parse_file(include_str!("../../src/game_session/runtime.rs")).unwrap();
    let owner = runtime
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "TimelineRuntime" => Some(item),
            _ => None,
        })
        .expect("timeline runtime");
    for name in [
        "network",
        "history",
        "mp_admission",
        "replay",
        "multiplayer_timing",
    ] {
        let field = owner
            .fields
            .iter()
            .find(|field| field.ident.as_ref().is_some_and(|ident| ident == name))
            .unwrap_or_else(|| panic!("missing timeline owner {name}"));
        assert!(
            matches!(field.vis, syn::Visibility::Inherited),
            "{name} must not be mutated directly by sibling frame drivers"
        );
    }
    for field in &owner.fields {
        assert!(
            ![
                "pending_inputs",
                "peer_hashes",
                "local_mp_hashes",
                "rewind_buffer",
                "rollback_checker",
                "replay_player",
                "replay_recorder",
                "recording_validity",
                "sealed_replay_header",
                "mp_host_frame_schedule",
                "pending_mp_state_hash",
                "last_mp_state_hash_frame"
            ]
            .iter()
            .any(|name| field.ident.as_ref().is_some_and(|ident| ident == name)),
            "timeline collections belong inside their lifecycle owners"
        );
    }
}

#[test]
fn http_transport_does_not_own_replay_storage() {
    struct StorageDeclarations;
    impl<'ast> Visit<'ast> for StorageDeclarations {
        fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
            assert!(
                ![
                    "ReplaySpool",
                    "ReplaySpoolState",
                    "ReplaySpoolWriter",
                    "ReplaySnapshot",
                    "PendingReplay",
                ]
                .iter()
                .any(|name| item.ident == name),
                "{} belongs to the replay service, not HTTP transport",
                item.ident
            );
            visit::visit_item_struct(self, item);
        }
    }
    StorageDeclarations
        .visit_file(&syn::parse_file(include_str!("../../src/http_server.rs")).unwrap());
}

#[test]
fn timeline_execution_uses_modes_without_snapshot_replacement_authority() {
    struct ExecutionSignatures {
        found_advance: bool,
        found_rpc: bool,
    }
    impl<'ast> Visit<'ast> for ExecutionSignatures {
        fn visit_signature(&mut self, signature: &'ast syn::Signature) {
            let advance = signature.ident == "advance_timeline";
            let rpc = signature.ident == "drain_post_tick_rpc";
            if advance || rpc {
                self.found_advance |= advance;
                self.found_rpc |= rpc;
                struct Arguments {
                    advance: bool,
                    mode: bool,
                }
                impl<'ast> Visit<'ast> for Arguments {
                    fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
                        for segment in &ty.path.segments {
                            assert!(
                                segment.ident != "EngineManager",
                                "frame execution must not replace snapshots"
                            );
                            if self.advance {
                                assert!(
                                    segment.ident != "bool",
                                    "use the admitted execution mode, not independent flags"
                                );
                                self.mode |= segment.ident == "FrameExecutionMode";
                            } else {
                                assert!(
                                    segment.ident != "Game" && segment.ident != "Host",
                                    "RPC execution must receive disjoint phase authority, not the whole game or host"
                                );
                            }
                        }
                        visit::visit_type_path(self, ty);
                    }
                }
                let mut arguments = Arguments {
                    advance,
                    mode: false,
                };
                for input in &signature.inputs {
                    arguments.visit_fn_arg(input);
                }
                assert!(
                    !advance || arguments.mode,
                    "timeline needs an explicit execution mode"
                );
            }
            visit::visit_signature(self, signature);
        }
    }
    let mut signatures = ExecutionSignatures {
        found_advance: false,
        found_rpc: false,
    };
    for source in [
        include_str!("../../src/game_session/frame_simulate.rs"),
        include_str!("../../src/game_session/runtime.rs"),
    ] {
        signatures.visit_file(&syn::parse_file(source).unwrap());
    }
    assert!(signatures.found_advance && signatures.found_rpc);
}

#[test]
fn deferred_http_work_is_owned_by_the_mission_not_process_statics() {
    let runtime = syn::parse_file(include_str!("../../src/game_session/runtime.rs")).unwrap();
    let owner = runtime
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "MissionRuntime" => Some(item),
            _ => None,
        })
        .expect("mission runtime");
    let ingress = owner
        .fields
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|ident| ident == "http"))
        .expect("mission owns HTTP ingress");
    assert!(
        matches!(&ingress.ty, syn::Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "SessionIngress"))
    );

    struct StaticTypes;
    impl<'ast> Visit<'ast> for StaticTypes {
        fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
            struct DeferredTypes;
            impl<'ast> Visit<'ast> for DeferredTypes {
                fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
                    for segment in &path.path.segments {
                        assert!(
                            ![
                                "PendingStep",
                                "PendingScreenshot",
                                "InputTaintKind",
                                "ReplayStatus",
                                "SessionIngress"
                            ]
                            .iter()
                            .any(|name| segment.ident == name),
                            "deferred mission work must not be stored in a static"
                        );
                    }
                    visit::visit_type_path(self, path);
                }
            }
            DeferredTypes.visit_type(&item.ty);
            visit::visit_item_static(self, item);
        }
    }
    for source in [
        include_str!("../../src/http_server.rs"),
        include_str!("../../src/http_server/ingress.rs"),
    ] {
        let syntax = syn::parse_file(source).unwrap();
        StaticTypes.visit_file(&syntax);
        for item in syntax.items {
            if let syn::Item::Static(item) = item {
                let name = item.ident.to_string();
                assert!(
                    ![
                        "PENDING_STEPS",
                        "PENDING_SCREENSHOTS",
                        "PENDING_REPLAY_TAINTS",
                        "REPLAY_STATUS"
                    ]
                    .contains(&name.as_str()),
                    "{name} must not outlive the mission"
                );
            }
        }
    }
}

#[test]
fn frontend_policy_and_observation_owners_remain_private() {
    let host = syn::parse_file(include_str!("../../src/host.rs")).unwrap();
    let structure = |name: &str| {
        host.items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == name => Some(item),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing owner {name}"))
    };
    let frontend = structure("HostFrontend");
    let input = syn::parse_file(include_str!("../../src/frontend_input.rs")).unwrap();
    let pointer_sequence = input
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "FrontendPointerSequence" => Some(item),
            _ => None,
        })
        .expect("pointer sequence owns capture and gesture together");
    assert!(
        pointer_sequence
            .fields
            .iter()
            .all(|field| matches!(field.vis, syn::Visibility::Inherited))
    );
    for name in [
        "preferences",
        "diagnostics",
        "planning",
        "pointer_sequence",
        "queue_strip_animations",
        "interaction",
    ] {
        let field = frontend
            .fields
            .iter()
            .find(|field| field.ident.as_ref().is_some_and(|ident| ident == name))
            .unwrap_or_else(|| panic!("missing frontend owner {name}"));
        assert!(
            matches!(field.vis, syn::Visibility::Inherited),
            "{name} must be private"
        );
    }
    for name in [
        "FrontendPreferences",
        "QueueStripAnimations",
        "FrontendInteraction",
    ] {
        assert!(
            structure(name)
                .fields
                .iter()
                .all(|field| matches!(field.vis, syn::Visibility::Inherited)),
            "{name} must expose operations, not writable fields"
        );
    }
    let diagnostics = syn::parse_file(include_str!("../../src/frontend_diagnostics.rs")).unwrap();
    let owner = diagnostics
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "FrontendDiagnostics" => Some(item),
            _ => None,
        })
        .expect("diagnostics owner");
    assert!(
        owner
            .fields
            .iter()
            .all(|field| matches!(field.vis, syn::Visibility::Inherited))
    );
}

fn readonly_reference_to(ty: &syn::Type, expected: &str) -> bool {
    matches!(ty, syn::Type::Reference(reference)
        if reference.mutability.is_none()
        && matches!(reference.elem.as_ref(), syn::Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == expected)))
}

#[derive(Default)]
struct BroadAuthority {
    found: bool,
    forbid_host: bool,
}

impl<'ast> Visit<'ast> for BroadAuthority {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if self.forbid_host {
            self.found |= path.path.segments.iter().any(|segment| {
                [
                    "Host",
                    "HostTransport",
                    "HostScripting",
                    "HostEffectBatches",
                    "HostAudio",
                ]
                .iter()
                .any(|name| segment.ident == name)
            });
        }
        self.found |= path.path.segments.iter().any(|segment| {
            [
                "Engine",
                "EngineManager",
                "MissionRuntime",
                "MissionWorld",
                "MissionFrame",
                "MissionMutation",
                "MissionIngress",
                "MissionPreTickPhase",
                "MissionAudioPhase",
            ]
            .iter()
            .any(|name| segment.ident == name)
        });
        visit::visit_type_path(self, path);
    }
}

fn violations(view: &syn::ItemStruct, presentation: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let mut engine_count = 0;
    let mut dev_count = 0;
    for field in &view.fields {
        let name = field.ident.as_ref().expect("phase views have named fields");
        if name == "engine" {
            engine_count += 1;
            if !readonly_reference_to(&field.ty, "Engine") {
                problems.push("engine must be &Engine".into());
            }
        } else {
            let mut broad = BroadAuthority {
                forbid_host: presentation,
                ..Default::default()
            };
            broad.visit_type(&field.ty);
            if broad.found {
                problems.push(format!("{name} exposes a broad simulation owner"));
            }
        }
        if presentation && name == "dev" {
            dev_count += 1;
            if !readonly_reference_to(&field.ty, "DevState") {
                problems.push("presentation dev must be &DevState".into());
            }
        }
    }
    if engine_count != 1 {
        problems.push("view needs exactly one engine query borrow".into());
    }
    if presentation && dev_count != 1 {
        problems.push("presentation needs exactly one developer query borrow".into());
    }
    problems
}

#[test]
fn phase_guard_distinguishes_shared_queries_from_mutation_escapes() {
    let good = syn::parse_str::<syn::ItemStruct>(
        "struct View<'a> { engine: &'a Engine, host: HostPresentation<'a>, dev: &'a DevState }",
    )
    .unwrap();
    assert!(violations(&good, true).is_empty());
    let mutable = syn::parse_str::<syn::ItemStruct>(
        "struct View<'a> { engine: &'a mut Engine, dev: &'a mut DevState }",
    )
    .unwrap();
    assert_eq!(violations(&mutable, true).len(), 2);
    let wrapped = syn::parse_str::<syn::ItemStruct>(
        "struct View<'a> { engine: &'a Engine, owner: Option<&'a mut EngineManager>, frame: &'a mut MissionFrame }",
    ).unwrap();
    assert_eq!(violations(&wrapped, false).len(), 2);
    let host_escape = syn::parse_str::<syn::ItemStruct>(
        "struct View<'a> { engine: &'a Engine, host: Option<&'a mut Host>, dev: &'a DevState }",
    )
    .unwrap();
    assert_eq!(violations(&host_escape, true).len(), 1);
}

#[test]
fn production_render_and_audio_capabilities_exclude_broad_authority() {
    let runtime = syn::parse_file(include_str!("../../src/game_session/runtime.rs")).unwrap();
    let host = syn::parse_file(include_str!("../../src/host.rs")).unwrap();
    for (syntax, name, expected) in [
        (
            &runtime,
            "MissionAudioPhase",
            vec!["audio", "viewport", "engine", "assets"],
        ),
        (
            &host,
            "HostPresentation",
            vec!["frontend", "sound", "options", "local_seat", "application"],
        ),
    ] {
        let view = syntax
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(view) if view.ident == name => Some(view),
                _ => None,
            })
            .expect("production capability exists");
        let fields: Vec<_> = view
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap().to_string())
            .collect();
        assert_eq!(
            fields, expected,
            "{name} authority changed; review its consumers"
        );
        for field in &view.fields {
            let field_name = field.ident.as_ref().unwrap().to_string();
            if let Some(expected_owner) = match field_name.as_str() {
                "audio" => Some("HostAudio"),
                "frontend" => Some("HostFrontend"),
                _ => None,
            } {
                assert!(
                    matches!(&field.ty, syn::Type::Reference(reference)
                    if reference.mutability.is_some()
                    && matches!(reference.elem.as_ref(), syn::Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment| segment.ident == expected_owner))),
                    "{name}.{field_name} must borrow only {expected_owner}"
                );
            }
            let expected_type = match field_name.as_str() {
                "engine" => Some("Engine"),
                "viewport" => Some("ViewportState"),
                "sound" => Some("SoundManager"),
                "options" => Some("GlobalOptions"),
                "application" => Some("ApplicationContext"),
                _ => None,
            };
            if let Some(expected_type) = expected_type {
                assert!(
                    readonly_reference_to(&field.ty, expected_type),
                    "{name}.{field_name} must remain read-only"
                );
            }
            if field_name == "application" {
                assert!(
                    matches!(field.vis, syn::Visibility::Inherited),
                    "application access must remain private behind the graphics query"
                );
            }
        }
    }
}

#[test]
fn render_consumers_cannot_request_a_whole_host() {
    for source in [
        include_str!("../../src/game_render.rs"),
        include_str!("../../src/game_render/debug.rs"),
        include_str!("../../src/game_render/hud.rs"),
        include_str!("../../src/game_render/minimap.rs"),
        include_str!("../../src/game_session/render.rs"),
    ] {
        let syntax = syn::parse_file(source).unwrap();
        for item in &syntax.items {
            let syn::Item::Fn(function) = item else {
                continue;
            };
            // This is an explicit command producer called before granting the
            // render capability, not a draw consumer.
            if function.sig.ident == "update_mouse_and_cursor" {
                continue;
            }
            for input in &function.sig.inputs {
                let syn::FnArg::Typed(argument) = input else {
                    continue;
                };
                if readonly_reference_to(&argument.ty, "Engine") {
                    continue;
                }
                let mut authority = BroadAuthority {
                    forbid_host: true,
                    ..Default::default()
                };
                authority.visit_type(&argument.ty);
                assert!(
                    !authority.found,
                    "{} exposes unrestricted simulation/host authority",
                    function.sig.ident
                );
            }
        }
    }
}

#[test]
fn draw_capability_has_no_mutable_frontend_or_application_authority() {
    let syntax = syn::parse_file(include_str!("../../src/host.rs")).unwrap();
    let view = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(view) if view.ident == "HostDraw" => Some(view),
            _ => None,
        })
        .expect("production immutable draw capability exists");
    let fields: Vec<_> = view
        .fields
        .iter()
        .map(|field| field.ident.as_ref().unwrap().to_string())
        .collect();
    assert_eq!(
        fields,
        [
            "frontend",
            "sound",
            "options",
            "local_seat",
            "graphic_config"
        ]
    );
    for (field, expected) in view.fields.iter().zip([
        "HostFrontend",
        "SoundManager",
        "GlobalOptions",
        "PlayerId",
        "GraphicConfig",
    ]) {
        if matches!(expected, "PlayerId" | "GraphicConfig") {
            assert!(matches!(&field.ty, syn::Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == expected)));
        } else {
            assert!(
                readonly_reference_to(&field.ty, expected),
                "draw {expected} must be shared"
            );
        }
        if expected == "GraphicConfig" {
            assert!(matches!(field.vis, syn::Visibility::Inherited));
        }
    }
    let render = syn::parse_file(include_str!("../../src/game_session/render.rs")).unwrap();
    let draw = render
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "render_frame" => Some(function),
            _ => None,
        })
        .expect("production render entry point exists");
    assert!(draw.sig.inputs.iter().any(|input| matches!(input,
        syn::FnArg::Typed(argument) if readonly_reference_to(&argument.ty, "HostDraw"))));
    for input in &draw.sig.inputs {
        if let syn::FnArg::Typed(argument) = input {
            struct MutableHost(bool);
            impl<'ast> Visit<'ast> for MutableHost {
                fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
                    self.0 |= path.path.segments.iter().any(|segment| {
                        ["Host", "HostPresentation", "HostFrontend"]
                            .iter()
                            .any(|name| segment.ident == name)
                    });
                    visit::visit_type_path(self, path);
                }
            }
            let mut escape = MutableHost(false);
            escape.visit_type(&argument.ty);
            assert!(
                !escape.0,
                "render_frame must receive only immutable host draw authority"
            );
        }
    }
}

#[test]
fn hud_draw_context_receives_decisions_not_mutable_hover_clocks() {
    let syntax = syn::parse_file(include_str!("../../src/game_session/render.rs")).unwrap();
    let context = syntax
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(view) if view.ident == "RenderContext" => Some(view),
            _ => None,
        })
        .expect("production draw context exists");
    let field = |name: &str| {
        context
            .fields
            .iter()
            .find(|field| field.ident.as_ref().is_some_and(|ident| ident == name))
            .unwrap_or_else(|| panic!("missing draw context field {name}"))
    };
    assert!(readonly_reference_to(
        &field("console_overlay").ty,
        "ConsoleOverlay"
    ));
    assert!(matches!(&field("hud_tooltips").ty, syn::Type::Path(path)
        if path.path.is_ident("HudTooltipPresentation")));
    // All hover clocks stay behind explicit preparation boundaries, never
    // inside the draw bundle (including the frame-addressed zoom clock).
    struct HoverClock(bool);
    impl<'ast> Visit<'ast> for HoverClock {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            self.0 |= path.path.segments.iter().any(|segment| {
                [
                    "CornerTooltipTracker",
                    "RequirementsTooltipTracker",
                    "BlazonTooltipTracker",
                    "StatureTooltipTracker",
                    "SherwoodTooltipTracker",
                    "PcActionTooltipTracker",
                    "ZoomTooltipTracker",
                ]
                .iter()
                .any(|name| segment.ident == name)
            });
            visit::visit_type_path(self, path);
        }
    }
    let mut clocks = HoverClock(false);
    clocks.visit_item_struct(context);
    assert!(!clocks.0, "draw context must not own live hover clocks");
}

pub(super) fn assert_production_views_are_readonly() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/game_session");
    for (file, name, presentation) in [
        ("runtime.rs", "MissionInputPhase", false),
        ("runtime.rs", "MissionPresentationPhase", true),
        ("live_gameplay.rs", "LiveGameplayContext", false),
    ] {
        let source = std::fs::read_to_string(root.join(file)).expect("read phase definition");
        let syntax = syn::parse_file(&source).expect("parse phase definition");
        let view = syntax
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(view) if view.ident == name => Some(view),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing production phase {name}"));
        assert!(
            violations(view, presentation).is_empty(),
            "{name}: {:?}",
            violations(view, presentation)
        );
    }
}
