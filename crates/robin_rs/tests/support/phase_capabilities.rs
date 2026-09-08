//! The private production views must actually carry the shared Engine borrow
//! exercised by the public compiler-negative tests. This structural assertion
//! avoids pretending that an inaccessible private type is a capability proof.

use syn::visit::{self, Visit};

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
