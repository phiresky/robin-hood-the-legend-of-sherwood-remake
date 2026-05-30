//! Console command dispatch — the engine-side glue that turns a parsed
//! `ConsoleCommand` into actual mutations of engine, AI, and campaign
//! state.
//!
//! The parser lives in `crate::console`; this module is the single
//! consumer of `ConsoleCommand`.

use super::{DevState, EngineInner, LevelAssets};
use crate::ai::AiLockFlags;
use crate::campaign::CampaignValue;
use crate::console::{ConsoleCommand, parse_with_final};
use crate::element::{Camp, Command, Entity, EntityId, ObjectType, Posture};
use crate::natives::EngineCommand;
use crate::sequence::SequenceElement;

/// Outcome of running a single console command.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleResponse {
    /// The input parsed and the command ran.  The string is a
    /// human-readable reply to echo back to the console history/UI.
    /// Empty string is allowed for silent commands.
    Ok(String),
    /// The input did not parse to any known command.
    Unknown,
    /// The command parsed but is not yet implemented on the Rust side.
    /// The string names the variant so the operator knows which stub
    /// they hit.  Distinct from `Ok` so callers (and tests) can tell
    /// when a command was a no-op.
    NotImplemented(&'static str),
    /// The `CAMPAIGN <file>` console cheat requested a save load.
    /// EngineInner can't reach into the save-file parser (host-owned format),
    /// so the host drains this variant and performs the load itself.
    /// Decision 6B.
    LoadCampaignRequested(std::path::PathBuf),
    /// The `_FINAL`-build deity easter egg fired: the input's MD5
    /// matched, `use_final` has been cleared, and the "Praised be His
    /// Name." message has already been pushed onto
    /// `Console::pending_output`.  The host drains this variant and
    /// applies the input-translator rebind table (engine has no access
    /// to the host-owned `InputTranslator`).
    DeityInvoked,
}

impl EngineInner {
    /// Parse `input` using the console's current dev/final mode,
    /// dispatch the command, and record the raw input in history.
    ///
    /// Returns `ConsoleResponse::Unknown` if the input doesn't parse.
    pub(crate) fn run_console_command(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        selected_view_element: &mut Option<EntityId>,
        input: &str,
    ) -> ConsoleResponse {
        let cmd_opt = parse_with_final(input, dev.console.use_final);

        // Deity easter egg: when `use_final` is set, match the line
        // buffer against the hidden token, clear `use_final`, print
        // "Praised be His Name.", and have the input translator
        // rebind the SLOW_MOTION/TELEPORT/RECORD_MOVIE/REQUEST_INFO keys.
        // The command-line parser destructively uppercases + NUL-splits
        // the buffer before checking the token, so the comparison sees
        // only the first (uppercased) token. The original binary compared
        // MD5(token) to 8f986776f01f52c1225231ae93ab634f; brute-forcing
        // likely all-caps cheat words recovered GLOIRE, confirmed with
        // `printf GLOIRE | md5sum`. The rebind itself lives in the
        // host-owned `InputTranslator`, so emit `DeityInvoked` for the
        // host to drain.
        if dev.console.use_final
            && let Some(token) = input
                .split_whitespace()
                .next()
                .map(|t| t.to_ascii_uppercase())
            && token == "GLOIRE"
        {
            dev.console.use_final = false;
            dev.console.push_history(input);
            dev.console.push_output("Praised be His Name.");
            return ConsoleResponse::DeityInvoked;
        }

        let Some(cmd) = cmd_opt else {
            return ConsoleResponse::Unknown;
        };
        dev.console.push_history(input);
        self.dispatch_console_command(assets, dev, selected_view_element, &cmd)
    }

    /// Dev-forced entry for out-of-band cheat sources (HTTP RPC, debug
    /// overlays) that need access to the full dev cheat set even in a
    /// `_FINAL`-style build.  Saves `use_final`, forces it to `false`
    /// around the dispatch, and restores it afterwards — gives the
    /// WASM GUI access to the full cheat list even in `_FINAL` builds
    /// (the help array otherwise hides the developer cheats).
    pub(crate) fn run_cheat_string(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        selected_view_element: &mut Option<EntityId>,
        input: &str,
    ) -> ConsoleResponse {
        let saved = dev.console.use_final;
        dev.console.use_final = false;
        let resp = self.run_console_command(assets, dev, selected_view_element, input);
        dev.console.use_final = saved;
        resp
    }

    /// Dispatch an already-parsed console command.  Exposed for tests
    /// that want to bypass the parser.
    ///
    /// `selected_view_element` is the host-side UI selection (the NPC
    /// whose vision cone is being displayed).  Four cheats — Honolulu,
    /// Morpheus, Hades, LastManStanding — act on "the NPC you're
    /// currently looking at" and some also clear the selection on
    /// success; the caller hands in a mutable reference so those cheats
    /// can write back.
    pub fn dispatch_console_command(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        selected_view_element: &mut Option<EntityId>,
        cmd: &ConsoleCommand,
    ) -> ConsoleResponse {
        use ConsoleCommand::*;
        match cmd {
            // ── Campaign value mutations ─────────────────────────
            GiveMoney { amount, show_help } => {
                // Panic on missing campaign — matches `campaign_mut_or_panic`'s
                // contract for cheats issued outside a mission.
                assert!(
                    self.campaign.is_some(),
                    "console: no active campaign to mutate"
                );
                self.add_campaign_value(CampaignValue::Ransom, *amount as i32);
                // Always prints "Money !" first, then emits a four-line
                // help listing (`Try also the following:`, the three
                // CASH suggestions) when called without args, then
                // applies the default.  We join everything into one
                // newline-delimited response — the overlay splits on
                // `\n` so each line renders separately.
                let mut out = String::from("Money !");
                if *show_help {
                    out.push_str(
                        "\nTry also the following :\n\
                         CASH CENT\n\
                         CASH THOUSAND\n\
                         CASH TENTHOUSAND\n\
                         CASH HUNDREDTHOUSAND",
                    );
                }
                out.push_str(&format!("\n{amount} gold added."));
                ConsoleResponse::Ok(out)
            }
            GiveBlazon { amount } => {
                self.campaign_mut_or_panic()
                    .add_value(CampaignValue::Blazon, *amount as i32);
                // Rust HUD is immediate-mode but we still push the
                // information-bars command so script-side consumers and
                // the blazon-bar state recomputation see the hook —
                // same pattern as the `WinMission` branch below.
                if let Some(game_host) =
                    self.mission_script.as_mut().and_then(|s| s.game_host_mut())
                {
                    game_host
                        .commands
                        .push(EngineCommand::UpdateInformationBars);
                }
                ConsoleResponse::Ok(format!("{amount} blazons added."))
            }
            GiveAmulets { amount } => {
                self.campaign_mut_or_panic()
                    .set_value(CampaignValue::Amulets, *amount as i32);
                ConsoleResponse::Ok(format!("{amount} amulets set."))
            }
            AddPeasant => {
                self.campaign_mut_or_panic()
                    .add_new_peasant_to_gang(None, &assets.profile_manager);
                ConsoleResponse::Ok("New member!".to_string())
            }
            CampaignReport => {
                self.campaign_mut_or_panic()
                    .log_report(&assets.profile_manager);
                ConsoleResponse::Ok("Reporting...".to_string())
            }

            // ── Mission flow ─────────────────────────────────────
            LoseMission => {
                self.mission.quit_lost = true;
                ConsoleResponse::Ok("Mission lost !".to_string())
            }
            WinMission => {
                // No-op in Sherwood; otherwise adds mission-stat money
                // (soldier + bonus − collected) + rescue PCs + pending
                // bonus-blazon pickups to the campaign totals before
                // calling `engine.win(true)`.
                let in_sherwood = self
                    .campaign
                    .as_ref()
                    .and_then(|c| {
                        let idx = c.current_mission_idx?;
                        Some(
                            c.missions[idx].profile(&assets.profile_manager).location
                                == crate::profiles::MissionLocation::Sherwood,
                        )
                    })
                    .unwrap_or(false);
                if in_sherwood {
                    // Whole cheat is gated on `location != SHERWOOD`.
                    return ConsoleResponse::Ok(String::new());
                }

                let money_delta = self.mission_stat.soldier_money as i32
                    + self.mission_stat.bonus_money as i32
                    - self.mission_stat.collected_money as i32;

                // Sum quantities of still-active BONUS_BLAZON pickups
                // left on the map.
                let mut pending_blazons: i32 = 0;
                for entity in self.entities.iter().flatten() {
                    if !entity.is_bonus() || !entity.element_data().active {
                        continue;
                    }
                    if let Some(obj) = entity.object_data()
                        && obj.object_type == ObjectType::BonusBlazon
                    {
                        pending_blazons += obj.quantity as i32;
                    }
                }

                self.add_campaign_value(CampaignValue::Ransom, money_delta);
                if let Some(campaign) = self.campaign.as_mut() {
                    // Per-mission rescue-PC table — adds recruits
                    // matching the current mission filename (e.g.
                    // S01_Not_VL → Stutely + Paysan A/B/C).
                    let added =
                        campaign.rescue_pcs_for_current_mission_win(&assets.profile_manager);
                    if added > 0 {
                        tracing::info!("WIN cheat: rescued {added} PC(s)");
                    }
                    campaign.add_value(CampaignValue::Blazon, pending_blazons);
                }
                // Rust's HUD is immediate-mode so no widget rebuild is
                // needed, but we still push the information-bars
                // command so script-side consumers see the hook.
                if let Some(game_host) =
                    self.mission_script.as_mut().and_then(|s| s.game_host_mut())
                {
                    game_host
                        .commands
                        .push(EngineCommand::UpdateInformationBars);
                }
                self.win(true);
                self.mission.quit_won = true;
                ConsoleResponse::Ok("Mission won !".to_string())
            }
            WinCampaign => {
                // Sets `ARESStateSucceeded = 9` on the shared mission
                // profile.  Rust profiles are `Arc`-shared, so we stash
                // the override on `Mission::ares_state_override` — read
                // by `Campaign::set_mission_done` when the win lands.
                if let Some(campaign) = self.campaign.as_mut()
                    && let Some(idx) = campaign.current_mission_idx
                {
                    campaign.missions[idx].ares_state_override = Some(9);
                }
                self.win(true);
                self.mission.quit_won = true;
                ConsoleResponse::Ok("Campaign won !".to_string())
            }
            LoadCampaign { filename } => {
                // The save-file format lives in the host (robin_rs::save_file).
                // EngineInner returns the request; host dispatches the actual load.
                ConsoleResponse::LoadCampaignRequested(std::path::PathBuf::from(filename))
            }

            // ── Blip / stealth cheats ────────────────────────────
            Ubiquity => {
                self.reveal_all_blips();
                ConsoleResponse::Ok("Unblip !".to_string())
            }

            // ── Simple AI-global toggles ─────────────────────────
            Freeze => {
                // Prints a leading "freeze" banner line, then the
                // frozen/defrosted status line.
                self.ai_global.freeze = !self.ai_global.freeze;
                let status = if self.ai_global.freeze {
                    "Enemies frozen."
                } else {
                    "Enemies defrosted."
                };
                ConsoleResponse::Ok(format!("freeze\n{status}"))
            }
            StupidSoldiers => {
                // Prints a leading "Pamela Anderson" banner line before
                // the stupid/smart status line.
                self.ai_global.stupid_soldiers_cheat = !self.ai_global.stupid_soldiers_cheat;
                let status = if self.ai_global.stupid_soldiers_cheat {
                    "Soldiers are stupid !"
                } else {
                    "Soldiers are smart !"
                };
                ConsoleResponse::Ok(format!("Pamela Anderson\n{status}"))
            }
            Goldeneye => {
                self.ai_global.golden_eye_mode = !self.ai_global.golden_eye_mode;
                ConsoleResponse::Ok(
                    if self.ai_global.golden_eye_mode {
                        "Invisibility On."
                    } else {
                        "Invisibility Off."
                    }
                    .to_string(),
                )
            }
            Babylon => {
                self.ai_global.speech_display = !self.ai_global.speech_display;
                ConsoleResponse::Ok(
                    if self.ai_global.speech_display {
                        "Patati Patata Bla Bla Laber Rhabarber Patatitata..."
                    } else {
                        "Shht !"
                    }
                    .to_string(),
                )
            }
            Ai => {
                self.ai_global.attribute_display = !self.ai_global.attribute_display;
                ConsoleResponse::Ok(
                    if self.ai_global.attribute_display {
                        "Attributes displayed"
                    } else {
                        "Attributes hidden"
                    }
                    .to_string(),
                )
            }

            // ── Debug flag toggles ───────────────────────────────
            Elevation => toggle_debug(
                &mut dev.debug.elevation_display,
                "Elevation display enabled.",
                "Elevation display disabled.",
            ),
            Railroad => toggle_debug(
                &mut dev.debug.railroad_display,
                "Railroads displayed.",
                "Railroads hidden.",
            ),
            Einstein => toggle_debug(
                &mut dev.debug.all_obstacles_display,
                "3D-obstacles displayed.",
                "3D-obstacles hidden.",
            ),
            Projection => toggle_debug(
                &mut dev.debug.projection_areas_display,
                "Projection areas displayed.",
                "Projection areas hidden.",
            ),
            Euler => {
                // Toggles motion-graph display and resets the index to 0.
                dev.debug.motion_graph_display = !dev.debug.motion_graph_display;
                dev.debug.motion_graph_display_index = 0;
                ConsoleResponse::Ok(
                    if dev.debug.motion_graph_display {
                        "The seven bridges of Koenigsberg."
                    } else {
                        "Graph hidden."
                    }
                    .to_string(),
                )
            }
            Motion => {
                dev.debug.motion_obstacles_display = !dev.debug.motion_obstacles_display;
                dev.debug.door_display = !dev.debug.door_display;
                ConsoleResponse::Ok(
                    if dev.debug.motion_obstacles_display {
                        "Motion obstacles displayed."
                    } else {
                        "Motion obstacles hidden."
                    }
                    .to_string(),
                )
            }
            Noise => {
                // "noise" banner, then on enable four lines (status +
                // three legend lines), on disable just the status line.
                dev.debug.noise_display = !dev.debug.noise_display;
                let body = if dev.debug.noise_display {
                    "Noise display enabled.\n\
                     \x20 White circles: Noises\n\
                     \x20 Black circles: Deafness because of covering noises, explosions etc...\n\
                     \x20 A NPC can hear a nois when he resp. his black circle is entirely within a white circle."
                } else {
                    "Noise display disabled."
                };
                ConsoleResponse::Ok(format!("noise\n{body}"))
            }
            SeekAndDestroy => toggle_debug(
                &mut dev.debug.display_seek_points,
                "Seek points displayed",
                "Seek points hidden",
            ),
            Light => toggle_debug(
                &mut dev.debug.display_light_zones,
                "Light zones enabled.",
                "Light zones disabled.",
            ),
            PcSight => {
                // Prints the *old* state then toggles, so the displayed
                // text is inverted vs. the new value.
                let was = dev.debug.pc_sight;
                dev.debug.pc_sight = !was;
                ConsoleResponse::Ok(if was { "PCs can't see" } else { "PCs can see" }.to_string())
            }
            Shadow => toggle_debug(
                &mut dev.debug.free_shadow_polygon,
                "Free shadow polygon enabled",
                "Free shadow polygon disabled",
            ),
            Sphere => {
                dev.debug.shadow_polygon_sphere = !dev.debug.shadow_polygon_sphere;
                ConsoleResponse::Ok(String::new())
            }
            SpriteMasks => toggle_debug(
                &mut dev.debug.sprite_masks_display,
                "Sprite masks displayed.",
                "Sprite masks hidden.",
            ),
            Surface => toggle_debug(
                &mut dev.debug.surface_display,
                "Surface overlay displayed.",
                "Surface overlay hidden.",
            ),
            EnergyDisplay => toggle_debug(
                &mut dev.debug.combat_energy_display,
                "Combat energy display enabled !",
                "Combat energy display disabled !",
            ),
            Anim => toggle_debug(
                &mut dev.debug.display_animation_lines,
                "Animation lines displayed.",
                "Animation lines hidden.",
            ),
            Companies => {
                dev.debug.company_number_display = !dev.debug.company_number_display;
                let on = dev.debug.company_number_display;
                ConsoleResponse::Ok(
                    if on {
                        "Company number displayed"
                    } else {
                        "Company number hidden"
                    }
                    .to_string(),
                )
            }
            CestLaZone => {
                // "Zone" banner then the enabled/disabled status line.
                dev.debug.script_zone_display = !dev.debug.script_zone_display;
                let status = if dev.debug.script_zone_display {
                    "Script zone display enabled."
                } else {
                    "Script zone display disabled."
                };
                ConsoleResponse::Ok(format!("Zone\n{status}"))
            }
            BigBrother => {
                dev.debug.actor_info_display = !dev.debug.actor_info_display;
                // TODO: Port the original actor-info text overlay. Until then,
                // make BIG BROTHER drive the live numeric ID overlay that is
                // already rendered by the host.
                dev.debug.entity_ids = dev.debug.actor_info_display;
                ConsoleResponse::Ok(
                    if dev.debug.actor_info_display {
                        "Actor infos displayed !"
                    } else {
                        "Actors infos hidden !"
                    }
                    .to_string(),
                )
            }
            LevelText { option } => match option.as_deref() {
                Some("DG") => {
                    dev.debug.all_dialogues = true;
                    ConsoleResponse::Ok("Displaying all dialogues...".to_string())
                }
                Some("DB") => {
                    dev.debug.all_debriefings = true;
                    ConsoleResponse::Ok("Displaying all debriefings...".to_string())
                }
                Some("PT") => {
                    dev.debug.all_popup_texts = true;
                    ConsoleResponse::Ok("Displaying all popup texts...".to_string())
                }
                Some("SB") => ConsoleResponse::Ok(
                    "Short-briefing DisplayAll cheat is no longer available.".to_string(),
                ),
                _ => ConsoleResponse::Ok("Displayes all texts. Options: DG DB PT".to_string()),
            },

            // ── PC invulnerability ───────────────────────────────
            // `Highlander` makes every Royalist fighter invulnerable;
            // `Highlander2` does the same for Lacklandists.  Both walk
            // `fighter_ids[camp]` (populated in `add_entity`).
            Highlander => {
                let ids =
                    self.fighter_ids[crate::element::Camp::Royalists.index().unwrap()].clone();
                for id in ids {
                    if let Some(entity) = self.get_entity_mut(id)
                        && let Some(h) = entity.human_data_mut()
                    {
                        h.invulnerable = true;
                    }
                }
                ConsoleResponse::Ok("Friends invulnerable".to_string())
            }
            Highlander2 => {
                let ids =
                    self.fighter_ids[crate::element::Camp::Lacklandists.index().unwrap()].clone();
                for id in ids {
                    if let Some(entity) = self.get_entity_mut(id)
                        && let Some(h) = entity.human_data_mut()
                    {
                        h.invulnerable = true;
                    }
                }
                ConsoleResponse::Ok("Foes invulnerable".to_string())
            }

            // ── Commands needing features not yet implemented ────
            Nuke => {
                // Prints "Nuking ..." before walking every soldier,
                // launches a damage(1000, 1000) sequence per victim,
                // then prints "Nuked N soldiers".  Uses the precomputed
                // `soldier_ids[camp]`.
                let victims: Vec<_> = self
                    .soldier_ids
                    .iter()
                    .flat_map(|v| v.iter().copied())
                    .collect();
                let count = victims.len();
                for id in victims {
                    self.launch_damage(id, 1000, 1000);
                }
                ConsoleResponse::Ok(format!("Nuking ...\nNuked {count} soldiers"))
            }
            Wakeup => {
                // Walk every NPC and, if unconscious, force concussion
                // to `31` — one above `CONCUSSION_WAKEUP_THRESHOLD` —
                // which drops them back to conscious via the normal
                // threshold transition in `set_concussion`.
                let ids: Vec<EntityId> = self
                    .entities
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, slot)| {
                        slot.as_ref().and_then(|e| {
                            if e.is_npc() && e.human_data().map(|h| h.unconscious).unwrap_or(false)
                            {
                                Some(EntityId::from_raw(idx as u32))
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                for id in ids {
                    // Route through `apply_concussion` so the guards
                    // (invulnerable / tied / carried / script-locked)
                    // fire AND the WentUnconscious / WokeUp outcome
                    // side-effects get dispatched to
                    // `pending_concussion_side_effects` for
                    // `perform_hourglass` to drain.
                    self.apply_concussion(assets, id, 31, false);
                }
                ConsoleResponse::Ok("Wake up !".to_string())
            }
            BudSpencer => {
                // Knock out every Lacklandist soldier via
                // concussion(100) + posture LYING + a trivial Wait
                // sequence element.  Concussion application reads the
                // target's own invulnerable / tied / carried state via
                // `concussion_ctx_for`.
                let ids = self.soldier_ids[Camp::Lacklandists.index().unwrap()].clone();
                for id in ids {
                    // Route through `apply_concussion` so a swordfighting
                    // victim is dropped from opponents' lists and gets
                    // the unconscious-star titbit + lose-consciousness
                    // stimulus.
                    self.apply_concussion(assets, id, 100, false);
                    if let Some(entity) = self.get_entity_mut(id) {
                        entity.set_posture(Posture::Lying);
                    }
                    self.launch_element(SequenceElement::new(1, Command::Wait, Some(id)));
                }
                ConsoleResponse::Ok("NPCs knocked out !".to_string())
            }
            Honolulu => {
                // Body only runs when there is a selected view element
                // and it is an NPC; otherwise it falls through silently.
                // The success path always prints "Honolulu" first, then
                // one of three branches keyed on the `last_actor_in_honolulu`
                // tracker:
                //  1. selection is active → deactivate + lock AI +
                //     stash in tracker + clear selection + "Bye…"
                //  2. tracker is populated and inactive → reactivate
                //     stored NPC + unlock AI + "I'm back!"
                //  3. otherwise → three-line usage help
                //
                // The tracker lives on `DevState::last_actor_in_honolulu`
                // (not simulation state; dev-only).
                let Some(id) = *selected_view_element else {
                    // No selection / not an NPC falls through with zero
                    // output.
                    return ConsoleResponse::Ok(String::new());
                };
                let is_npc = self.get_entity(id).map(|e| e.is_npc()).unwrap_or(false);
                if !is_npc {
                    return ConsoleResponse::Ok(String::new());
                }
                let active_now = self
                    .get_entity(id)
                    .map(|e| e.element_data().active)
                    .unwrap_or(false);

                if active_now {
                    // Send this NPC on vacation.
                    if let Some(entity) = self.get_entity_mut(id) {
                        entity.element_data_mut().active = false;
                        if let Some(npc) = entity.npc_data_mut()
                            && let Some(base) = npc.ai_brain.base_mut()
                        {
                            base.non_script_lock(AiLockFlags::FREEZE);
                        }
                    }
                    dev.last_actor_in_honolulu = Some(id);
                    *selected_view_element = None;
                    return ConsoleResponse::Ok("Honolulu\nBye, I'm on holiday.".to_string());
                }

                // Reactivate stored vacation NPC — only if it's still
                // the one we previously stashed and still inactive.
                if let Some(last_id) = dev.last_actor_in_honolulu {
                    let still_inactive = self
                        .get_entity(last_id)
                        .map(|e| !e.element_data().active)
                        .unwrap_or(false);
                    if still_inactive {
                        if let Some(entity) = self.get_entity_mut(last_id) {
                            entity.element_data_mut().active = true;
                            if let Some(npc) = entity.npc_data_mut()
                                && let Some(base) = npc.ai_brain.base_mut()
                            {
                                base.non_script_unlock(AiLockFlags::FREEZE);
                            }
                        }
                        return ConsoleResponse::Ok("Honolulu\nI'm back!".to_string());
                    }
                }

                // Fallback: three-line usage help.
                ConsoleResponse::Ok(
                    "Honolulu\n\
                     Cheat couldn't be performed. There are two possibilities to do this cheat:\n\
                     (1) Enable a view cone, then use this cheat to send this guy to Honolulu\n\
                     (2) If (1) already done: Disable view cone, use this cheat to get last guy back from Honolulu."
                        .to_string(),
                )
            }
            Morpheus => {
                // Always prints "MORPHEUS" first, then gates on
                // selection being an NPC.  On success: concussion 100
                // + posture LYING + Wait element + clear selection +
                // "Sleep well...".  On failure (no selection / not NPC):
                // the "please enable view cone" message.  Concussion
                // application reads the target's own invulnerable /
                // tied / carried state via `concussion_ctx_for`.
                let is_npc = selected_view_element
                    .and_then(|id| self.get_entity(id).map(|e| e.is_npc()))
                    .unwrap_or(false);
                if !is_npc {
                    return ConsoleResponse::Ok(
                        "MORPHEUS\n\
                         Please enable view cone of a NPC before using this command."
                            .to_string(),
                    );
                }
                let id = selected_view_element.expect("NPC-selected implies id present");
                // Route through `apply_concussion` so the KO side-effects
                // (drop from sword-fight opponents' lists,
                // unconscious-star titbit, lose-consciousness stimulus)
                // fire — a direct `set_concussion` call would skip them.
                self.apply_concussion(assets, id, 100, false);
                if let Some(entity) = self.get_entity_mut(id) {
                    entity.set_posture(Posture::Lying);
                }
                self.launch_element(SequenceElement::new(1, Command::Wait, Some(id)));
                *selected_view_element = None;
                ConsoleResponse::Ok("MORPHEUS\nSleep well...".to_string())
            }
            Hades => {
                // Always prints "HADES" first, gates on selected NPC.
                // On success: zero life points (alert green, sleeping
                // state with the "forever" substate, close eyes,
                // detectable cleanup, dying animation) + clear selection
                // + "Sleep well... forever!".  The full cascade lives
                // in `EngineInner::handle_death`, which needs
                // `&LevelAssets`, so we queue the victim on
                // `pending_hades_kills` and `perform_hourglass` drains.
                let is_npc = selected_view_element
                    .and_then(|id| self.get_entity(id).map(|e| e.is_npc()))
                    .unwrap_or(false);
                if !is_npc {
                    return ConsoleResponse::Ok(
                        "HADES\n\
                         Please enable view cone of a NPC before using this command."
                            .to_string(),
                    );
                }
                let id = selected_view_element.expect("NPC-selected implies id present");
                self.pending_hades_kills.push(id);
                *selected_view_element = None;
                ConsoleResponse::Ok("HADES\nSleep well... forever!".to_string())
            }
            LastManStanding => {
                // Prints "Last man standing" unconditionally, then
                // either deactivates every NPC other than the selected
                // and prints "Lonely hero...", or prints the
                // no-selection error.
                let Some(keep) = *selected_view_element else {
                    return ConsoleResponse::Ok(
                        "Last man standing\n\
                         Please enable view cone of a NPC before using this command."
                            .to_string(),
                    );
                };
                let ids: Vec<EntityId> = self.npc_ids.clone();
                for id in ids {
                    if id == keep {
                        continue;
                    }
                    if let Some(entity) = self.get_entity_mut(id) {
                        entity.element_data_mut().active = false;
                        if let Some(npc) = entity.npc_data_mut()
                            && let Some(base) = npc.ai_brain.base_mut()
                        {
                            base.non_script_lock(AiLockFlags::FREEZE);
                        }
                    }
                }
                ConsoleResponse::Ok("Last man standing\nLonely hero...".to_string())
            }
            DiesIrae => {
                // Prints "Dies irae" banner, toggles
                // `ai_global.ezekiel_2517`, then prints either "Mine is
                // the vengeance, says the Lord !" or "The Lord pardons...".
                // The flag is consumed by view-cone selection and script
                // `EnableViewCone`, matching the original cheat hook.
                self.ai_global.ezekiel_2517 = !self.ai_global.ezekiel_2517;
                let status = if self.ai_global.ezekiel_2517 {
                    "Mine is the vengeance, says the Lord !"
                } else {
                    "The Lord pardons..."
                };
                ConsoleResponse::Ok(format!("Dies irae\n{status}"))
            }
            RoterAlarm => {
                // Sets attentive mode on every soldier — silent cheat,
                // emits no console output.
                let soldier_ids: Vec<EntityId> = self
                    .entities
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, slot)| {
                        slot.as_ref()
                            .filter(|e| e.is_soldier())
                            .map(|_| EntityId::from_raw(idx as u32))
                    })
                    .collect();
                for id in soldier_ids {
                    self.set_soldier_attentive_mode(id, true, false);
                }
                ConsoleResponse::Ok(String::new())
            }
            MisterSandman => {
                // For every PC, launch a damage(100, 0) sequence —
                // hp=100, concussion=0, *not* the reverse.  Swapping
                // the two would change a death roll into a concussion
                // roll.
                let pcs = self.pc_ids.clone();
                for id in pcs {
                    self.launch_damage(id, 100, 0);
                }
                ConsoleResponse::Ok("Sweet dreams !".to_string())
            }
            Coma => {
                // Needs a selected PC and at least one amulet, then
                // launches hp=10000 / concussion=0 damage on the first
                // selected PC.
                let selected = self.seats[0].selection.first().copied();
                let amulets = self
                    .campaign
                    .as_ref()
                    .map(|c| c.get_value(CampaignValue::Amulets))
                    .unwrap_or(0);
                match (selected, amulets) {
                    (None, _) => {
                        ConsoleResponse::Ok("Please, select the PC to make sleep.".to_string())
                    }
                    (Some(_), n) if n < 1 => ConsoleResponse::Ok(
                        "There not enough amulets left to put the selected PC in the coma."
                            .to_string(),
                    ),
                    (Some(id), _) => {
                        self.launch_damage(id, 10000, 0);
                        ConsoleResponse::Ok("Coma !".to_string())
                    }
                }
            }
            Reinforcement => {
                // Silent cheat: queue a reinforcement request here,
                // and let `drain_pending_reinforcements` perform the
                // actual PC spawn during `perform_hourglass`.
                self.pending_reinforcements.push(None);
                ConsoleResponse::Ok(String::new())
            }
            SanPetrus => {
                // Unconditionally prints "San Petrus", then either the
                // no-selection error or — per selected PC — launches a
                // hp=10000 / concussion=0 damage sequence and prints
                // `"<profile name> has been recalled by San Petrus."`.
                let selected = self.seats[0].selection.clone();
                if selected.is_empty() {
                    return ConsoleResponse::Ok(
                        "San Petrus\nYou must select at least one PC.".to_string(),
                    );
                }
                let mut out = String::from("San Petrus");
                // Resolve profile names before mutating via
                // `launch_damage` so the campaign borrow stays clean.
                let names: Vec<String> = selected
                    .iter()
                    .map(|&id| {
                        let profile_idx = self
                            .get_entity(id)
                            .and_then(|e| e.pc_data())
                            .map(|pc| pc.profile_index);
                        match (profile_idx, self.campaign.as_ref()) {
                            (Some(idx), Some(_)) => assets
                                .profile_manager
                                .get_character(idx)
                                .map(|p| p.profile_name.to_string())
                                .unwrap_or_else(|| format!("PC {id:?}")),
                            _ => format!("PC {id:?}"),
                        }
                    })
                    .collect();
                for (id, name) in selected.iter().zip(names.iter()) {
                    self.launch_damage(*id, 10000, 0);
                    out.push_str(&format!("\n{name} has been recalled by San Petrus."));
                }
                ConsoleResponse::Ok(out)
            }
            WaspMaster => {
                // Always prints "Wasps", then either the typo-preserved
                // error or force-sets every selected PC's wasp ammo to
                // `0xFFFF`.  Re-enables the action slot via
                // `enable_pc_action` when amount > 0.
                self.force_ammo_with_banner(
                    assets,
                    crate::profiles::Action::WaspNest,
                    0xFFFF,
                    "Wasps",
                    "You must selected at meast one PC which must go to paradise.",
                )
            }
            GiveArrows => {
                // Always prints "Arrows", then either the typo-preserved
                // error or force-sets every selected PC's bow ammo to
                // `0xFFFF` (and re-enables the action slot).
                self.force_ammo_with_banner(
                    assets,
                    crate::profiles::Action::Bow,
                    0xFFFF,
                    "Arrows",
                    "You must selected at meast one PC.",
                )
            }
            GiveAmmo => {
                // For every PC, force all 3 action slots to 999.
                // Forcing ammo also re-enables the slot when the amount
                // is non-zero — that ripple fires here via
                // `enable_pc_action` so a PC who had run out of a given
                // action can fire again immediately.
                let pcs: Vec<_> = self
                    .pc_ids
                    .iter()
                    .filter_map(|&id| {
                        self.get_entity(id).and_then(|e| match e {
                            Entity::Pc(pc) => Some((
                                id,
                                pc.pc.profile_index,
                                self.pc_description_index_for_pc_data(&pc.pc)?,
                            )),
                            _ => None,
                        })
                    })
                    .collect();
                for (id, profile_idx, status_idx) in pcs {
                    let Some(campaign) = self.campaign.as_mut() else {
                        continue;
                    };
                    let actions = match assets.profile_manager.get_character(profile_idx) {
                        Some(p) => p.actions,
                        None => continue,
                    };
                    if let Some(desc) = campaign.characters.get_mut(status_idx) {
                        for action in actions {
                            desc.status.force_set_ammo(action, 999);
                        }
                    }
                    // Re-enable every slot now that it has ammo again.
                    for action in actions {
                        if action != crate::profiles::Action::NoAction {
                            self.enable_pc_action(assets, id, action);
                        }
                    }
                }
                ConsoleResponse::Ok("Ammunition !".to_string())
            }
            Lukas { pcs } => {
                // Resolve each single-letter initial (R/J/T/S/W/M/A/B/C)
                // to a PC via the character profile index, then inflict
                // pain — funnels to an hp=100 / concussion=100 damage
                // sequence (same sequence used elsewhere).
                if let Some(pcs) = pcs {
                    let ids = self.resolve_pcs_by_initials(assets, pcs);
                    for id in ids {
                        self.launch_damage(id, 100, 100);
                    }
                }
                ConsoleResponse::Ok("PCs knocked out !".to_string())
            }
            Call { actor, method } => {
                // Dispatch a named method on a named actor; the only
                // methods actually reachable from the shipping console
                // are `HideInterface` / `DisplayInterface` on a PC.
                // The original console identifies the actor by hex
                // pointer; the Rust port uses a single-letter initial
                // instead, since `EntityId` is a stable index rather
                // than a raw memory address.
                //
                // We flip the per-PC `interface_hidden` flag and emit
                // the "Hiding interface for ActorPC(...)" /
                // "Displaying interface for ActorPC(...)" response.
                // The HUD portrait row is derived from live PC entities
                // and filters on `pc_data().interface_hidden`.
                let mut ch = actor.chars();
                let (Some(c), None) = (ch.next(), ch.next()) else {
                    return ConsoleResponse::Ok("CALL: expected single PC initial.".to_string());
                };
                let ids = self.resolve_pcs_by_initials(assets, &c.to_string());
                let Some(&id) = ids.first() else {
                    return ConsoleResponse::Ok("CALL: no such PC.".to_string());
                };
                let hide = match method.to_ascii_uppercase().as_str() {
                    "HIDEINTERFACE" => true,
                    "DISPLAYINTERFACE" => false,
                    _ => {
                        return ConsoleResponse::Ok(format!("CALL: unknown method {method}."));
                    }
                };
                if let Some(pc) = self.get_entity_mut(id).and_then(|e| e.pc_data_mut()) {
                    pc.interface_hidden = hide;
                }
                let verb = if hide { "Hiding" } else { "Displaying" };
                ConsoleResponse::Ok(format!(
                    "{verb} interface for ActorPC({}:{})",
                    c.to_ascii_uppercase(),
                    id.index()
                ))
            }
            Fps => {
                // Idempotent set (not a toggle): unconditionally
                // enables FPS display and prints "FPS displayed."
                // every time.
                dev.debug.fps_display = true;
                ConsoleResponse::Ok("FPS displayed.".to_string())
            }
            StatusFramecache | StatusShadow => {
                // The Rust port has no frame cache and no legacy shadow
                // buffer — the diagnostics these variants used to
                // print are meaningless here.
                ConsoleResponse::Ok(
                    "STATUS: no frame cache / shadow buffer in Rust port.".to_string(),
                )
            }
            StatusHardware => {
                // Print a multi-section hardware report.  We push every
                // line through `Console::pending_output` so the overlay
                // scrollback shows them interleaved with the user's
                // input line.  Rust port uses the system allocator and
                // portable feature detection, so we surface the subset
                // we can actually query (arch, SIMD features, CPU
                // count); the rest (cache sizes, physical memory) were
                // platform-detection stubs even in the shipping
                // original.
                dev.console.push_output("=> CPU Information");
                dev.console.push_output("");
                dev.console.push_output(format!(
                    "Vendor String................. {}",
                    std::env::consts::ARCH
                ));
                #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
                let (has_mmx, has_sse, has_sse2, has_avx) = (
                    std::is_x86_feature_detected!("mmx"),
                    std::is_x86_feature_detected!("sse"),
                    std::is_x86_feature_detected!("sse2"),
                    std::is_x86_feature_detected!("avx"),
                );
                #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
                let (has_mmx, has_sse, has_sse2, has_avx) = (false, false, false, false);
                dev.console
                    .push_output("FPU........................... Detected");
                dev.console.push_output(format!(
                    "Multi Media eXtension......... {}",
                    if has_mmx { "Detected" } else { "Not Detected" }
                ));
                dev.console.push_output(format!(
                    "Streaming SIMD Extension...... {}",
                    if has_sse { "Detected" } else { "Not Detected" }
                ));
                dev.console.push_output(format!(
                    "SSE2.......................... {}",
                    if has_sse2 { "Detected" } else { "Not Detected" }
                ));
                dev.console.push_output(format!(
                    "AVX........................... {}",
                    if has_avx { "Detected" } else { "Not Detected" }
                ));
                let procs = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1);
                dev.console.push_output(format!(
                    "Architecture.................. {}",
                    if procs > 1 {
                        "Multiprocessor"
                    } else {
                        "Monoprocessor"
                    }
                ));
                dev.console
                    .push_output(format!("Logical Processors............ {procs}"));
                ConsoleResponse::Ok(String::new())
            }
            StatusPc => {
                // Per-PC dump of the actor identity + its
                // interface-displayed state.  Hex pointers are
                // meaningless in the Rust runtime, so we print the
                // stable `EntityId` instead.
                if self.pc_ids.is_empty() {
                    return ConsoleResponse::Ok("No PCs in the mission.".to_string());
                }
                for &id in &self.pc_ids.clone() {
                    let displayed = self
                        .get_entity(id)
                        .and_then(|e| e.pc_data())
                        .map(|pc| !pc.interface_hidden)
                        .unwrap_or(true);
                    dev.console
                        .push_output(format!("Actor id ........................ {}", id.index()));
                    dev.console.push_output(format!(
                        "Interface Displayed ............. {}",
                        if displayed { "YES" } else { "NO" }
                    ));
                }
                ConsoleResponse::Ok(String::new())
            }
            Optimize => {
                // No frame cache to defragment — the Rust port uses
                // the system allocator.
                ConsoleResponse::Ok(
                    "No frame cache to optimize; Rust port uses the system allocator.".to_string(),
                )
            }
            Forget => {
                // No-op in the shipping binary (memory-check command
                // was compiled out).
                ConsoleResponse::Ok(String::new())
            }
            Sarkozy => {
                // No-op in the shipping binary (alloc-check command was
                // compiled out).
                ConsoleResponse::Ok(String::new())
            }

            // ── Misc dev-mode ────────────────────────────────────
            Help => ConsoleResponse::Ok(cheat_help_text(dev.console.use_final).to_string()),
            AssertFalse => {
                // The int 3 trap is commented out, so just log.
                tracing::warn!("console: assert(false) cheat invoked");
                ConsoleResponse::Ok("assert( false );".to_string())
            }
            UsageError(msg) => ConsoleResponse::Ok((*msg).to_string()),
        }
    }

    /// Force the ammo counter for `action` to `amount` on every
    /// currently-selected PC, printing a leading banner line plus
    /// either the no-selection error or the banner alone.  The
    /// "header print then branch" shape is shared by the `Arrows` and
    /// `WaspMaster` cheats: `"<banner>"` is always emitted, then either
    /// `"<err>"` or the ammo-forcing loop silently proceeds.  Also
    /// calls [`EngineInner::enable_pc_action`] per PC so the slot is
    /// re-enabled now that it has ammo again.
    fn force_ammo_with_banner(
        &mut self,
        assets: &LevelAssets,
        action: crate::profiles::Action,
        amount: u16,
        banner: &str,
        err_if_empty: &str,
    ) -> ConsoleResponse {
        if self.seats[0].selection.is_empty() {
            return ConsoleResponse::Ok(format!("{banner}\n{err_if_empty}"));
        }
        let selected: Vec<EntityId> = self.seats[0].selection.clone();
        let profile_indices: Vec<(EntityId, crate::profiles::CharacterProfileIdx)> = selected
            .iter()
            .filter_map(|&id| {
                self.get_entity(id)
                    .and_then(|e| e.pc_data())
                    .map(|pc| (id, pc.profile_index))
            })
            .collect();
        if let Some(campaign) = self.campaign.as_mut() {
            for (_id, idx) in &profile_indices {
                if let Some(desc) = campaign.characters.get_mut(usize::from(*idx)) {
                    desc.status.force_set_ammo(action, amount);
                }
            }
        }
        if amount > 0 {
            for (id, _idx) in profile_indices {
                self.enable_pc_action(assets, id, action);
            }
        } else {
            // Forcing the ammo counter to 0 should disable the action
            // slot.  Every in-tree caller passes 0xFFFF, 999, or 1 —
            // never 0 — so this arm is unreachable today.  If a future
            // caller passes 0, route through `disable_pc_action` (needs
            // a `&LevelAssets` reference to honour the
            // first-available-action deselect fallback) instead of
            // silently leaving the slot enabled-but-empty.
            debug_assert!(
                amount > 0,
                "force_ammo_with_banner with amount=0 would need disable_pc_action; see comment"
            );
        }
        ConsoleResponse::Ok(banner.to_string())
    }

    /// Resolve a `LUKAS`-style PC initial string (e.g. `"RJS"`) to the
    /// entity IDs of the matching PCs on the map.
    ///
    /// For the `'R'` initial there are two profile entries named
    /// "Robin des bois" (town / forest variants).  We walk *every*
    /// profile that matches the name and return the first one that
    /// resolves to a live PC.
    ///
    /// Unknown initials emit a warning and are otherwise skipped — the
    /// hint string is "Unknown character (use one or more of these:
    /// RJTSWMABC) !".
    fn resolve_pcs_by_initials(&self, assets: &LevelAssets, initials: &str) -> Vec<EntityId> {
        let mut out = Vec::new();
        if self.campaign.is_none() {
            return out;
        }
        for ch in initials.chars() {
            let Some(name) = pc_initial_to_profile_name(ch) else {
                tracing::warn!(
                    "console: unknown PC initial {ch:?} — use one or more of these: RJTSWMABC"
                );
                continue;
            };
            // Walk *all* profiles named `name` to handle the 'R'
            // fallback (Robin has two profile entries — town and
            // forest).  For other initials there is only one match so
            // the loop body runs once.  Profile-name lookup is
            // case-sensitive.
            let matching_profiles: Vec<crate::profiles::CharacterProfileIdx> = assets
                .profile_manager
                .characters
                .iter()
                .enumerate()
                .filter(|(_, cp)| cp.profile_name == name)
                .map(|(i, _)| crate::profiles::CharacterProfileIdx(i as u32))
                .collect();
            if matching_profiles.is_empty() {
                tracing::warn!("console: no character profile named {name:?} (initial {ch:?})");
                continue;
            }
            let before = out.len();
            'matched: for profile_idx in matching_profiles {
                for &pc_id in &self.pc_ids {
                    if let Some(pc) = self.get_entity(pc_id).and_then(|e| e.pc_data())
                        && pc.profile_index == profile_idx
                    {
                        out.push(pc_id);
                        break 'matched;
                    }
                }
            }
            // Warn on a miss (rare in practice — the console cheat
            // only reaches here with a live gang — but a silent no-op
            // would mask bad cheat input).
            if out.len() == before {
                tracing::warn!("console: no PC found for profile {name:?} (initial {ch:?})");
            }
        }
        out
    }

    /// Helper that panics if a console command tries to touch campaign
    /// state outside of a mission.  Matches the "don't fabricate data"
    /// project rule — failing loudly is better than silently no-opping.
    fn campaign_mut_or_panic(&mut self) -> &mut crate::campaign::Campaign {
        self.campaign
            .as_mut()
            .expect("console: no active campaign to mutate")
    }
}

/// Toggle a bool debug flag and produce an on/off reply in one expression.
fn toggle_debug(flag: &mut bool, on_msg: &'static str, off_msg: &'static str) -> ConsoleResponse {
    *flag = !*flag;
    ConsoleResponse::Ok(if *flag { on_msg } else { off_msg }.to_string())
}

/// Map a single PC initial to its character-profile name.  Used by
/// `LUKAS` and shared with the campaign-side `create_gang_from_pcs`
/// fallback.
fn pc_initial_to_profile_name(c: char) -> Option<&'static str> {
    match c.to_ascii_uppercase() {
        'R' => Some("Robin des bois"),
        'J' => Some("Petit Jean"),
        'T' => Some("Frere Tuck"),
        'S' => Some("Stutely"),
        'W' => Some("Will Ecarlate"),
        'M' => Some("Lady Marianne"),
        'A' => Some("Paysan A"),
        'B' => Some("Paysan B"),
        'C' => Some("Paysan C"),
        'F' => Some("Ferris"),
        _ => None,
    }
}

/// Formatted cheat listing for the `HELP` command.
///
/// We keep the list in a hand-curated form grouped by category — a
/// categorised listing is more useful than a flat list, and the set
/// of commands is small enough that an annotation table per
/// `ConsoleCommand` variant would be over-engineered.  The
/// `use_final` gate picks between the 9-entry final (release) cheat
/// set and the full dev set.
fn cheat_help_text(use_final: bool) -> &'static str {
    if use_final {
        "Robin Hood Console Help File.\n\
         Available commands in this release:\n\
         \n\
         CASH <amount>        Add gold to the campaign.\n\
         GOODLUCK <amount>    Set the amulet count.\n\
         EINSTEIN             Toggle 3D-obstacle display.\n\
         IMMUNITY             Make friendly PCs invulnerable.\n\
         MERRYMAN             Add a new peasant to the gang.\n\
         PAM                  Toggle stupid-soldiers mode.\n\
         UNBLIP               Reveal every blipped NPC.\n\
         WINNER               Complete the current mission.\n\
         BINGO                Refill every PC's ammunition.\n"
    } else {
        "Robin Hood Console Help File.\n\
         Available commands in this release:\n\
         \n\
         Campaign / mission:  EZB <amount>, WAPPEN <amount>, AMULETS <amount>,\n\
                              KOLKOZ, REPORT, WIN, LOOSE, I AM THE WINNER,\n\
                              CAMPAIGN <file>\n\
         AI toggles:          AI, BABYLON, FREEZE, GOLDENEYE,\n\
                              PAMELA ANDERSON (aka STUPID SOLDIERS),\n\
                              ROTER ALARM, DIES IRAE\n\
         PC helpers:          HIGHLANDER, HIGHLANDER2, AMOR, FULLHOUSE,\n\
                              WASP MASTER, MISTER SANDMAN, COMA, SAN PETRUS,\n\
                              LUKAS <initials>\n\
         NPC mutators:        NUKE, WAKEUP, BUD SPENCER, LAST MAN STANDING,\n\
                              HONOLULU, MORPHEUS, HADES, ALARM (REINFORCEMENT)\n\
         Stealth / vision:    UBIQUITY, PCSIGHT, BIG BROTHER\n\
         Display toggles:     ANIM, COMPANIES, EINSTEIN, ELEVATION, EULER,\n\
                              ENERGYDISPLAY, LIGHT, MOTION, NOISE, PROJECTION,\n\
                              RAILROAD, SEEKANDDESTROY, SHADOW, SPHERE,\n\
                              CESTLAZONE, LEVEL TEXT [DG|DB|PT|SB]\n\
         Rendering / debug:   FPS, STATUS FRAMECACHE|HARDWARE|SHADOW|PC,\n\
                              OPTIMIZE, CALL <initial> HIDEINTERFACE|DISPLAYINTERFACE,\n\
                              FORGET, SARKOZY, ASSERTFALSE\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign::{Campaign, CampaignValue};
    use crate::element::{
        ActorData, ActorSoldier, ElementData, ElementKind, Entity, HumanData, NpcData, SoldierData,
    };

    fn soldier(blipped: bool) -> Entity {
        Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                blipped,
                // Soldiers loaded from a level always carry a concrete
                // posture (the deserialiser remaps `Undefined` to the
                // kind-specific default).  Test helpers don't go
                // through that path, so seed `Upright` here — without
                // it the `posture_after_transition` stamp picks up
                // `Undefined` and the `MakePostureTransition` panic
                // arm fires when the test launches a sequence.
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 50,
                ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                ..NpcData::default()
            },
            soldier: SoldierData {
                // Real enemy soldiers always have a defined camp;
                // explicitly setting it here means `add_entity` files
                // them under `fighter_ids[Lacklandists]` so cheats
                // like HIGHLANDER2 can iterate them.
                cached_camp: crate::element::Camp::Lacklandists,
                ..SoldierData::default()
            },
        })
    }

    fn engine_with_campaign() -> (EngineInner, DevState) {
        let dev = DevState::default();
        let mut engine = EngineInner::new();
        engine.campaign = Some(Campaign::new());
        (engine, dev)
    }

    fn assets() -> crate::engine::LevelAssets {
        crate::engine::LevelAssets::new()
    }

    #[test]
    fn unknown_input_returns_unknown() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        assert_eq!(
            engine.run_console_command(&assets(), &mut dev, &mut None, "XYZZY"),
            ConsoleResponse::Unknown
        );
    }

    #[test]
    fn parsed_input_pushes_history() {
        let (mut engine, mut dev) = engine_with_campaign();
        let _ = engine.run_console_command(&assets(), &mut dev, &mut None, "NUKE");
        assert_eq!(dev.console.history.last().map(String::as_str), Some("NUKE"));
    }

    #[test]
    fn big_brother_toggles_rendered_entity_ids() {
        let (mut engine, mut dev) = engine_with_campaign();

        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "BIG BROTHER");
        assert_eq!(
            resp,
            ConsoleResponse::Ok("Actor infos displayed !".to_string())
        );
        assert!(dev.debug.actor_info_display);
        assert!(dev.debug.entity_ids);

        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "BIG BROTHER");
        assert_eq!(
            resp,
            ConsoleResponse::Ok("Actors infos hidden !".to_string())
        );
        assert!(!dev.debug.actor_info_display);
        assert!(!dev.debug.entity_ids);
    }

    #[test]
    fn sprite_masks_toggles_overlay() {
        let (mut engine, mut dev) = engine_with_campaign();

        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "SPRITEMASKS");
        assert_eq!(
            resp,
            ConsoleResponse::Ok("Sprite masks displayed.".to_string())
        );
        assert!(dev.debug.sprite_masks_display);

        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "SPRITE MASKS");
        assert_eq!(
            resp,
            ConsoleResponse::Ok("Sprite masks hidden.".to_string())
        );
        assert!(!dev.debug.sprite_masks_display);
    }

    #[test]
    fn give_money_mutates_campaign() {
        let (mut engine, mut dev) = engine_with_campaign();
        let before = engine
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Ransom);
        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "EZB 500");
        assert_eq!(
            resp,
            ConsoleResponse::Ok("Money !\n500 gold added.".to_string())
        );
        let after = engine
            .campaign
            .as_ref()
            .unwrap()
            .get_value(CampaignValue::Ransom);
        assert_eq!(after, before + 500);
    }

    #[test]
    fn lose_mission_sets_quit_lost() {
        let (mut engine, mut dev) = engine_with_campaign();
        assert!(!engine.mission.quit_lost);
        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "LOOSE");
        assert!(matches!(resp, ConsoleResponse::Ok(_)));
        assert!(engine.mission.quit_lost);
    }

    #[test]
    fn freeze_toggles_ai_global_flag() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        assert!(!engine.ai_global.freeze);
        engine.run_console_command(&assets(), &mut dev, &mut None, "FREEZE");
        assert!(engine.ai_global.freeze);
        engine.run_console_command(&assets(), &mut dev, &mut None, "FREEZE");
        assert!(!engine.ai_global.freeze);
    }

    #[test]
    fn ubiquity_reveals_all_blipped_npcs() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        let id_blipped = engine.add_entity(soldier(true));
        let id_plain = engine.add_entity(soldier(false));

        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "UBIQUITY");
        assert!(matches!(resp, ConsoleResponse::Ok(_)));

        assert!(
            !engine
                .get_entity(id_blipped)
                .unwrap()
                .element_data()
                .blipped
        );
        assert!(!engine.get_entity(id_plain).unwrap().element_data().blipped);
    }

    #[test]
    fn highlander2_marks_enemy_npcs_invulnerable() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        let id = engine.add_entity(soldier(false));
        // Sanity: starts vulnerable.
        assert!(
            !engine
                .get_entity(id)
                .unwrap()
                .human_data()
                .unwrap()
                .invulnerable
        );

        engine.run_console_command(&assets(), &mut dev, &mut None, "HIGHLANDER2");

        assert!(
            engine
                .get_entity(id)
                .unwrap()
                .human_data()
                .unwrap()
                .invulnerable
        );
    }

    #[test]
    fn elevation_is_debug_toggle() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        assert!(!dev.debug.elevation_display);
        engine.run_console_command(&assets(), &mut dev, &mut None, "ELEVATION");
        assert!(dev.debug.elevation_display);
    }

    #[test]
    fn level_text_routes_by_option() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        engine.run_console_command(&assets(), &mut dev, &mut None, "LEVEL TEXT DB");
        assert!(dev.debug.all_debriefings);
        assert!(!dev.debug.all_dialogues);
    }

    #[test]
    fn roter_alarm_launches_enter_attentive_sequence_on_every_soldier() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        let id = engine.add_entity(soldier(false));
        {
            let e = engine.get_entity(id).unwrap().enemy_ai().unwrap();
            assert!(!e.attentive);
            assert!(!e.will_be_attentive);
        }
        assert_eq!(engine.sequence_manager.sequence_count(), 0);
        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "ROTER ALARM");
        // ROTER ALARM is a silent cheat — emits no console text.
        assert_eq!(resp, ConsoleResponse::Ok(String::new()));
        // The sequence element launch flips `will_be_attentive` immediately;
        // `attentive` only flips once the transition animation completes
        // (see `engine::animation` for the anim-done handler).
        let e = engine.get_entity(id).unwrap().enemy_ai().unwrap();
        assert!(e.will_be_attentive);
        assert_eq!(engine.sequence_manager.sequence_count(), 1);
    }

    #[test]
    fn nuke_launches_damage_on_every_soldier() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        engine.add_entity(soldier(false));
        engine.add_entity(soldier(false));
        assert_eq!(engine.sequence_manager.sequence_count(), 0);
        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "NUKE");
        assert_eq!(
            resp,
            ConsoleResponse::Ok("Nuking ...\nNuked 2 soldiers".to_string())
        );
        assert_eq!(engine.sequence_manager.sequence_count(), 2);
    }

    #[test]
    fn final_mode_accepts_unblip_alias() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        dev.console.use_final = true;
        engine.add_entity(soldier(true));
        let resp = engine.run_console_command(&assets(), &mut dev, &mut None, "UNBLIP");
        assert!(matches!(resp, ConsoleResponse::Ok(_)));
    }

    #[test]
    fn final_mode_rejects_dev_only_commands() {
        let mut dev = DevState::default();
        let mut engine = EngineInner::new();
        dev.console.use_final = true;
        // NUKE is a dev-only cheat — must not resolve in final mode.
        assert_eq!(
            engine.run_console_command(&assets(), &mut dev, &mut None, "NUKE"),
            ConsoleResponse::Unknown
        );
    }
}
