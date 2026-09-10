//! These screens must synchronize renderer dimensions before interpreting
//! pointer events. This is a source-boundary guard, not a GPU resize test.

use syn::visit::Visit;

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct DirectPolls {
    count: usize,
}

impl<'ast> Visit<'ast> for DirectPolls {
    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        if expression.method == "poll_events" {
            self.count += 1;
        }
        syn::visit::visit_expr_method_call(self, expression);
    }
}

#[test]
fn modal_screens_use_resize_aware_event_polling() {
    for (name, source) in [
        (
            "player_select",
            include_str!("../src/main_menu/player_select.rs"),
        ),
        (
            "ui_task_state",
            include_str!("../src/game_session/ui_task_state.rs"),
        ),
        ("language", include_str!("../src/ingame_menu/language.rs")),
        (
            "buy_blazons",
            include_str!("../src/ingame_menu/buy_blazons.rs"),
        ),
        (
            "mission_description",
            include_str!("../src/ingame_menu/mission_description.rs"),
        ),
        ("save_load", include_str!("../src/ingame_menu/save_load.rs")),
        ("dialogue", include_str!("../src/ingame_menu/dialogue.rs")),
    ] {
        let file = syn::parse_file(source).unwrap();
        let mut polls = DirectPolls::default();
        polls.visit_file(&file);
        assert_eq!(
            polls.count, 0,
            "{name}: use layout::poll_events_with_transform before interpreting input"
        );
    }
}
