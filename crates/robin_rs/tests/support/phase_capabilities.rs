//! The private production views must actually carry the shared Engine borrow
//! exercised by the public compiler-negative tests. This structural assertion
//! avoids pretending that an inaccessible private type is a capability proof.

use super::source_syntax::{item_fn, item_struct, parsed};
use syn::visit::{self, Visit};

// Transport authority moved into these modules; ownership guards must inspect
// implementations as well as the root's type declarations and re-exports.
const HTTP_AUTHORITY_SOURCES: &[&str] = &[
    include_str!("../../src/http_server.rs"),
    include_str!("../../src/http_server/dispatch.rs"),
    include_str!("../../src/http_server/ingress.rs"),
    include_str!("../../src/http_server/native_routes.rs"),
    include_str!("../../src/http_server/native_transport.rs"),
    include_str!("../../src/http_server/request_decode.rs"),
    include_str!("../../src/http_server/request_lifetime.rs"),
];

#[test]
fn rendering_entrypoint_requires_the_explicit_presentation_view() {
    let syntax = parsed(include_str!("../../src/game_session/render.rs"));
    let render = item_fn(&syntax, "render_frame").expect("production render entrypoint");
    struct ViewArgument {
        found: bool,
    }
    impl<'ast> Visit<'ast> for ViewArgument {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            for segment in &path.path.segments {
                assert!(
                    segment.ident != "Engine" && segment.ident != "EngineInner",
                    "render pass must not receive the general simulation read API"
                );
                self.found |= segment.ident == "PresentationView";
            }
            visit::visit_type_path(self, path);
        }
    }
    let mut argument = ViewArgument { found: false };
    argument.visit_signature(&render.sig);
    assert!(
        argument.found,
        "render pass must consume the explicit read view"
    );
}

#[test]
fn interpolation_storage_cannot_reintroduce_authoritative_engine_ownership() {
    let syntax = parsed(include_str!("../../src/game_session/interactive.rs"));
    let owner =
        item_struct(&syntax, "NativeRefreshInterpolation").expect("native interpolation owner");
    struct Storage {
        presentation_count: usize,
    }
    impl<'ast> Visit<'ast> for Storage {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            for segment in &path.path.segments {
                assert_ne!(
                    segment.ident, "Engine",
                    "interpolation must not own a runnable authoritative engine"
                );
                self.presentation_count += usize::from(segment.ident == "PresentationEngine");
            }
            visit::visit_type_path(self, path);
        }
    }
    let mut storage = Storage {
        presentation_count: 0,
    };
    storage.visit_item_struct(owner);
    assert_eq!(
        storage.presentation_count, 1,
        "retain one presentation owner"
    );
}

#[test]
fn live_frame_authority_cannot_be_cloned_or_derived_from_diagnostics() {
    let syntax = parsed(include_str!("../../src/game_session/runtime/frame.rs"));
    let frame = item_struct(&syntax, "MissionFrame").expect("live frame transaction");
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
            && let Some((trait_path, _)) = &item.trait_
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
fn ready_context_is_not_a_deserializable_data_wrapper() {
    let syntax = parsed(include_str!("../../src/application.rs"));
    for name in [
        "ApplicationContext",
        "ReadyApplicationContext",
        "ApplicationServices",
    ] {
        let owner =
            item_struct(&syntax, name).unwrap_or_else(|| panic!("missing live owner {name}"));
        for attribute in &owner.attrs {
            if attribute.path().is_ident("derive") {
                let derives = attribute
                    .parse_args_with(
                        syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                    )
                    .unwrap();
                assert!(
                    !derives.iter().any(|path| path
                        .segments
                        .iter()
                        .any(|segment| segment.ident == "Deserialize")),
                    "{name} must be composed from real services; deserialize its diagnostic DTO instead"
                );
            }
        }
    }
}

#[test]
fn application_composition_is_separate_from_mission_host() {
    let host = parsed(include_str!("../../src/host.rs"));
    assert!(!host.items.iter().any(|item| matches!(
        item,
        syn::Item::Struct(item)
            if ["ApplicationContext", "ApplicationServices", "ReadyApplicationContext"]
                .iter().any(|name| item.ident == *name)
    )));

    struct ServiceAssemblies(usize);
    impl<'ast> Visit<'ast> for ServiceAssemblies {
        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            if matches!(item.self_ty.as_ref(), syn::Type::Path(path)
                if path.path.is_ident("ApplicationServices"))
            {
                self.0 += item
                    .items
                    .iter()
                    .filter(|item| {
                        matches!(
                            item, syn::ImplItem::Fn(method) if method.sig.ident == "compose"
                        )
                    })
                    .count();
            }
            syn::visit::visit_item_impl(self, item);
        }
    }
    let application = parsed(include_str!("../../src/application.rs"));
    let mut assemblies = ServiceAssemblies(0);
    assemblies.visit_file(&application);
    assert_eq!(
        assemblies.0, 1,
        "service ownership has one composition entry point"
    );
}

#[test]
fn mission_journals_and_sprite_publication_are_private() {
    for (source, owner_name, fields) in [
        (
            include_str!("../../src/game_session/runtime/frame.rs"),
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
            include_str!("../../src/host/frontend.rs"),
            "FrontendResources",
            &["frame_holder", "frame_holder_opacity"][..],
        ),
    ] {
        let syntax = parsed(source);
        let owner = item_struct(&syntax, owner_name).expect("capability owner");
        for name in fields {
            let field = owner
                .fields
                .iter()
                .find(|field| field.ident.as_ref().is_some_and(|ident| ident == name))
                .unwrap_or_else(|| panic!("missing {owner_name}.{name}"));
            // Frame journals moved into a child module, but their visibility
            // must still stop at runtime, excluding sibling mission drivers.
            let runtime_only = owner_name == "MissionFrame"
                && matches!(&field.vis, syn::Visibility::Restricted(visibility)
                    if visibility.path.is_ident("super"));
            assert!(
                matches!(field.vis, syn::Visibility::Inherited) || runtime_only,
                "{owner_name}.{name} must remain private to its runtime owner"
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
        signatures.visit_file(&parsed(source));
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
    ReplayStatics.visit_file(&parsed(include_str!("../../src/replay_service.rs")));
    ReplayStatics.visit_file(&parsed(include_str!("../../src/mission_replays.rs")));
    for source in [
        include_str!("../../src/game_session/replay_init.rs"),
        include_str!("../../src/game_session/replay_launch.rs"),
        include_str!("../../src/game_session/mission_launch.rs"),
    ] {
        ReplayStatics.visit_file(&parsed(source));
    }
}

#[test]
fn timeline_reconciliation_and_history_are_private_owners() {
    let runtime = parsed(include_str!("../../src/game_session/runtime.rs"));
    let owner = item_struct(&runtime, "TimelineRuntime").expect("timeline runtime");
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
    for &source in HTTP_AUTHORITY_SOURCES {
        StorageDeclarations.visit_file(&parsed(source));
    }
}

#[test]
fn diagnostic_and_image_builders_do_not_own_rpc_lifetimes() {
    struct Boundaries;
    impl<'ast> Visit<'ast> for Boundaries {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            for segment in &path.segments {
                assert!(
                    ![
                        "HttpTransport",
                        "SessionIngress",
                        "Responder",
                        "RequestRouter",
                        "HttpRequest"
                    ]
                    .iter()
                    .any(|name| segment.ident == name),
                    "{} does not belong in a diagnostic/image builder",
                    segment.ident
                );
            }
            visit::visit_path(self, path);
        }
    }
    for source in [
        include_str!("../../src/http_server/diagnostics.rs"),
        include_str!("../../src/http_server/screenshot.rs"),
    ] {
        Boundaries.visit_file(&parsed(source));
    }
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
        signatures.visit_file(&parsed(source));
    }
    assert!(signatures.found_advance && signatures.found_rpc);
}

#[test]
fn scripted_modal_helpers_borrow_engine_without_timeline_authority() {
    let source = parsed(include_str!("../../src/game_session/frame_simulate.rs"));
    for name in ["drive_scripted_modal_lanes", "drive_leave_mission_prompt"] {
        let function =
            item_fn(&source, name).unwrap_or_else(|| panic!("missing modal helper {name}"));
        struct Arguments;
        impl<'ast> Visit<'ast> for Arguments {
            fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
                for segment in &ty.path.segments {
                    assert!(
                        segment.ident != "TimelineRuntime" && segment.ident != "EngineManager",
                        "scripted modal helpers must not own timeline or snapshot replacement authority"
                    );
                }
                visit::visit_type_path(self, ty);
            }
        }
        let mut engine_found = false;
        for input in &function.sig.inputs {
            Arguments.visit_fn_arg(input);
            let syn::FnArg::Typed(argument) = input else {
                continue;
            };
            let syn::Type::Reference(reference) = argument.ty.as_ref() else {
                continue;
            };
            let syn::Type::Path(path) = reference.elem.as_ref() else {
                continue;
            };
            if path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "Engine")
            {
                assert!(
                    reference.mutability.is_none(),
                    "modal engine access must be read-only"
                );
                engine_found = true;
            }
        }
        assert!(engine_found, "{name} must borrow its read-only engine");
    }
}

#[test]
fn deferred_http_work_is_owned_by_the_mission_not_process_statics() {
    let runtime = parsed(include_str!("../../src/game_session/runtime.rs"));
    let owner = item_struct(&runtime, "MissionRuntime").expect("mission runtime");
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
    for &source in HTTP_AUTHORITY_SOURCES {
        let syntax = parsed(source);
        StaticTypes.visit_file(&syntax);
        for item in &syntax.items {
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
    let host = parsed(include_str!("../../src/host/frontend.rs"));
    let structure =
        |name: &str| item_struct(&host, name).unwrap_or_else(|| panic!("missing owner {name}"));
    let frontend = structure("HostFrontend");
    for (owner, fields) in [
        (
            "FrontendResources",
            &[
                "mission_surfaces",
                "frame_holder",
                "frame_holder_opacity",
                "shipping",
                "background_decals",
            ][..],
        ),
        (
            "FrontendPresentation",
            &[
                "engine_display",
                "draw_order",
                "selection_mark",
                "draw_manager",
                "pc_info_overlay",
                "fade_to_black",
                "skip_render",
            ][..],
        ),
    ] {
        let owner = structure(owner);
        for name in fields {
            assert!(
                owner
                    .fields
                    .iter()
                    .any(|field| field.ident.as_ref().is_some_and(|ident| ident == name)),
                "missing lifecycle-owned {name}"
            );
            assert!(
                !frontend
                    .fields
                    .iter()
                    .any(|field| field.ident.as_ref().is_some_and(|ident| ident == name)),
                "{name} must not duplicate the lifecycle owner in HostFrontend"
            );
        }
    }
    let input = parsed(include_str!("../../src/frontend_input.rs"));
    let pointer_sequence = item_struct(&input, "FrontendPointerSequence")
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
        "pending_print_screen",
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
    let diagnostics = parsed(include_str!("../../src/frontend_diagnostics.rs"));
    let owner = item_struct(&diagnostics, "FrontendDiagnostics").expect("diagnostics owner");
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
    let runtime = parsed(include_str!("../../src/game_session/runtime.rs"));
    let host = parsed(include_str!("../../src/host.rs"));
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
        let view = item_struct(&syntax, name).expect("production capability exists");
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
        let syntax = parsed(source);
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
    let syntax = parsed(include_str!("../../src/host.rs"));
    let view =
        item_struct(&syntax, "HostDraw").expect("production immutable draw capability exists");
    let fields: Vec<_> = view
        .fields
        .iter()
        .map(|field| field.ident.as_ref().unwrap().to_string())
        .collect();
    assert_eq!(
        fields,
        [
            "viewport",
            "draw_manager",
            "frontend",
            "sound",
            "options",
            "local_seat",
            "graphic_config"
        ]
    );
    for (field, expected) in view.fields.iter().zip([
        "ViewportState",
        "DrawManager",
        "HostFrontend",
        "SoundManager",
        "GlobalOptions",
        "PlayerId",
        "GraphicConfig",
    ]) {
        if matches!(expected, "DrawManager" | "PlayerId" | "GraphicConfig") {
            assert!(matches!(&field.ty, syn::Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == expected)));
        } else {
            assert!(
                readonly_reference_to(&field.ty, expected),
                "draw {expected} must be shared"
            );
        }
        if matches!(expected, "ViewportState" | "DrawManager" | "GraphicConfig") {
            assert!(matches!(field.vis, syn::Visibility::Inherited));
        }
    }
    // Capture-specific camera data is private and readable only through shared
    // accessors; it must not introduce mutable frontend authority into drawing.
    for (accessor, expected) in [
        ("viewport", "ViewportState"),
        ("draw_manager", "DrawManager"),
    ] {
        let method = syntax.items.iter().filter_map(|item| match item {
            syn::Item::Impl(implementation)
                if matches!(&*implementation.self_ty, syn::Type::Path(path)
                    if path.path.segments.last().is_some_and(|segment| segment.ident == "HostDraw")) => Some(implementation),
            _ => None,
        }).flat_map(|implementation| &implementation.items).find_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == accessor => Some(method),
            _ => None,
        }).expect("explicit draw camera accessor exists");
        assert!(matches!(&method.sig.output, syn::ReturnType::Type(_, ty)
            if readonly_reference_to(ty, expected)));
    }
    let render = parsed(include_str!("../../src/game_session/render.rs"));
    let draw = item_fn(&render, "render_frame").expect("production render entry point exists");
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
    let syntax = parsed(include_str!("../../src/game_session/render.rs"));
    let context = item_struct(&syntax, "RenderContext").expect("production draw context exists");
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
        let view =
            item_struct(&syntax, name).unwrap_or_else(|| panic!("missing production phase {name}"));
        assert!(
            violations(view, presentation).is_empty(),
            "{name}: {:?}",
            violations(view, presentation)
        );
    }
}
