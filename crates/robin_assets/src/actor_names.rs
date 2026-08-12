//! Best-effort script-position → human-readable name mapping.
//!
//! `GetActorScript(N)` addresses entities by index in the global
//! "script element" array. That array is the flat concatenation
//! (per [`engine/level_loading.rs`](../../robin_engine/src/engine/level_loading.rs)):
//! proto-level patch FX (only those with a non-empty frame profile),
//! proto-level animation FX, mission-level patch FX, civilians,
//! PCs-to-rescue, soldiers, targets, bonuses, scrolls.
//! `GetPatchScript(N)` indexes the concatenated proto + mission patches.
//!
//! We load a datadir via the engine's normal loader path (`load_level`
//! + `ProfileManager`) and walk the resulting structs. Names come from:
//! 1. `script_class` on the mission record (hash suffix stripped).
//! 2. Profile `filename` (English identifier like `WillScarlet`,
//!    `Guard A01`) keyed by the mission record's `profile_number` /
//!    `profile_index`.
//! 3. Patches use their sprite's `profile_name`.
//!
//! Buildings / locations / doors / hiking paths carry no names in the
//! data, so `GetBuildingScript(N)` etc. stay as raw indices in the
//! script output.

use std::collections::HashMap;
use std::path::Path;

use robin_engine::element_kinds::BonusItemType;
use robin_engine::level_data::{
    LoadedLevel, LoadedMission, LoadedProtoLevel, RawCivilian, RawPcRescue, RawSoldier,
    WaypointCommand, load_level,
};
use robin_engine::profiles::{CivilianType, ProfileManager};
use robin_engine::sbfile::{SB_FILE_READ, SbFile};

/// Which engine slot binds this script class. Derived from the mission
/// data: a class name appearing in `mission.soldiers[].script_class` is
/// an actor-kind script; in `mission.scrolls[]` → Scroll; etc.
///
/// The decompiler uses this to pick the abstract base class.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ScriptKind {
    /// `StartUp` — the whole mission's lifecycle hook.
    Mission,
    /// Soldier / civilian / PC-to-rescue `script_class`.
    Actor,
    /// `scrolls[].script_class` — Parchment / Tutorial.
    Scroll,
    /// `script_objects.sectors[].script_class` — EnterZone / ExitZone.
    Zone,
    /// `targets[].script_class` — ActivatedByX mechanisms.
    Target,
    /// `hiking_paths[].waypoints[].command = Script` — ReachPoint callbacks.
    Waypoint,
}

/// Which entity kind owns a script-element slot. Used by the decompiler
/// to group `GetActorScript(N)` references under the matching global
/// (e.g. an animation index lives under `Anim.xxx`, a soldier under
/// `Actors.xxx`). Tracked per slot because the global flat `actors`
/// array concatenates every entity kind.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ActorSlotKind {
    PatchFx,
    Anim,
    Actor,
    Target,
    Bonus,
    Scroll,
}

impl ActorSlotKind {
    /// TypeScript global namespace name for this slot kind.
    pub fn global_name(self) -> &'static str {
        match self {
            ActorSlotKind::PatchFx => "PatchFx",
            ActorSlotKind::Anim => "Anim",
            ActorSlotKind::Actor => "Actors",
            ActorSlotKind::Target => "Targets",
            ActorSlotKind::Bonus => "Bonuses",
            ActorSlotKind::Scroll => "Scrolls",
        }
    }
}

/// Name tables keyed by script position, one per entity kind.
#[derive(Debug, Default)]
pub struct ActorNames {
    /// `actors[i]` is the identifier for `GetActorScript(i)`, or `None`.
    pub actors: Vec<Option<String>>,
    /// `actor_kinds[i]` is which entity kind occupies slot `i`, or
    /// `None` if nothing lives there. Parallel to `actors`.
    pub actor_kinds: Vec<Option<ActorSlotKind>>,
    /// `patches[i]` is the identifier for `GetPatchScript(i)`, or `None`.
    pub patches: Vec<Option<String>>,
    /// SCB class name → which engine slot binds it.
    pub class_kinds: HashMap<String, ScriptKind>,
    /// Popup-scroll text strings indexed by `iPopupTextID`. Loaded from
    /// the mission's `.red` file + `Data/Text/Level.res` text table.
    /// Empty if loading failed (the decompiler still works, it just
    /// doesn't annotate the IDs with content).
    pub popup_texts: Vec<String>,
    /// Short-briefing text strings indexed by `iID` in
    /// `DoneShortBriefing(iID)`.
    pub short_briefing_texts: Vec<String>,
}

impl ActorNames {
    /// Return `(global_name, identifier)` for `GetActorScript(position)` —
    /// e.g. `("Anim", "Nottingham_eau01")` or `("Actors", "PrinceJohn")`.
    /// `None` if the slot is unnamed.
    pub fn actor_qualified(&self, position: i32) -> Option<(&'static str, &str)> {
        let i = usize::try_from(position).ok()?;
        let name = self.actors.get(i).and_then(Option::as_deref)?;
        let kind = (*self.actor_kinds.get(i)?)?;
        Some((kind.global_name(), name))
    }

    pub fn patch(&self, position: i32) -> Option<&str> {
        lookup(&self.patches, position)
    }

    pub fn kind_of(&self, class_name: &str) -> Option<ScriptKind> {
        if class_name == "StartUp" {
            return Some(ScriptKind::Mission);
        }
        self.class_kinds.get(class_name).copied()
    }

    pub fn popup_text(&self, id: i32) -> Option<&str> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.popup_texts.get(i))
            .map(String::as_str)
    }

    pub fn short_briefing_text(&self, id: i32) -> Option<&str> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.short_briefing_texts.get(i))
            .map(String::as_str)
    }
}

fn lookup(v: &[Option<String>], position: i32) -> Option<&str> {
    usize::try_from(position)
        .ok()
        .and_then(|i| v.get(i))
        .and_then(|o| o.as_deref())
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Profiles(String),
    MissionNotFound(String),
    Level(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "{e}"),
            LoadError::Profiles(s) => write!(f, "profiles: {s}"),
            LoadError::MissionNotFound(m) => write!(f, "mission '{m}' not in profile.cpf"),
            LoadError::Level(s) => write!(f, "level: {s}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

/// Load a mission from a datadir and build the name tables.
///
/// `datadir` points at a game-root directory (the one that contains
/// `Data/Configuration/` and `Data/Levels/`). `mission_filename` is the
/// bare name ("Dem_Lei_MP") — the corresponding proto-level is looked up
/// in the campaign's mission profiles.
pub fn load_from_datadir(datadir: &Path, mission_filename: &str) -> Result<ActorNames, LoadError> {
    let profiles = load_profile_manager(datadir)?;

    let mission_profile = profiles
        .missions
        .iter()
        .find(|m| m.mission_filename == mission_filename)
        .ok_or_else(|| LoadError::MissionNotFound(mission_filename.to_owned()))?;
    let proto_filename = mission_profile.proto_level_filename.clone();

    // Build `is_beggar` from civilian profile types — required by the
    // binary .rhm parser to decide whether to expect beggar scroll sets.
    let civ_is_beggar: Vec<bool> = profiles
        .civilians
        .iter()
        .map(|c| c.civilian_type == CivilianType::Beggar)
        .collect();
    let is_beggar = |idx: u32| civ_is_beggar.get(idx as usize).copied().unwrap_or(false);

    let level_dir = datadir.join("Data").join("Levels");
    let level_dir_str = level_dir.to_string_lossy().into_owned();
    let loaded = load_level(
        mission_filename,
        &proto_filename,
        &level_dir_str,
        &is_beggar,
        &mut |_| (),
    )
    .map_err(|e| LoadError::Level(format!("{e:?}")))?;

    let mut names = build_names(&loaded, &profiles);
    // Best-effort text loading — failures are logged and leave the
    // name table with empty `popup_texts`/`short_briefing_texts`.
    load_mission_texts(datadir, mission_profile.id, &mut names);
    Ok(names)
}

/// Populate popup-scroll and short-briefing text arrays from the mission's
/// `.red` file + a level `.res` file found under the datadir. Failures are
/// logged and leave the arrays empty.
fn load_mission_texts(datadir: &Path, mission_id: u32, names: &mut ActorNames) {
    let data_dir = datadir.join("Data");
    let shipping = crate::shipping_datadir::try_load(&data_dir).ok().flatten();

    // Resolve the `.red` level-descriptor file.
    let red_name = crate::res_descr::red_filename(mission_id);
    let descriptors = if let Some(dd) = shipping.as_ref()
        && let Some(d) = dd.red_files.get(&red_name)
    {
        d.clone()
    } else {
        let red_path = data_dir.join("Text").join(&red_name);
        match crate::res_descr::load(&red_path.to_string_lossy()) {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("{}: {e}", red_path.display());
                return;
            }
        }
    };

    // Resolve `Data/Text/Level.res`. Locale varies (`2047` = neutral, `1031`
    // = German, …) so search a few locale dirs when the neutral path is
    // missing.
    let mut text_res = crate::resource_manager::ResourceManager::new();
    if let Some(dd) = shipping.as_ref() {
        if let Err(e) = text_res.attach_or_from_shipping("Data/Text/Level.res", Some(dd)) {
            tracing::debug!("attach shipping Data/Text/Level.res: {e}");
            return;
        }
    } else {
        let candidates = ["", "2047", "1031", "1036", "1033"];
        let mut loaded = false;
        for locale in candidates {
            let path = datadir
                .join(locale)
                .join("Data")
                .join("Text")
                .join("Level.res");
            if !path.is_file() {
                continue;
            }
            match text_res.attach_resource_file(&path.to_string_lossy()) {
                Ok(()) => {
                    loaded = true;
                    break;
                }
                Err(e) => tracing::debug!("attach {}: {e}", path.display()),
            }
        }
        if !loaded {
            tracing::debug!("no Data/Text/Level.res found under {}", datadir.display());
            return;
        }
    }

    // Popup scroll texts.
    let popup_table = descriptors.popup_text.text_table_id;
    if popup_table != 0
        && let Ok(count) = text_res.get_string_count(popup_table)
    {
        names.popup_texts.reserve(count);
        for i in 0..count {
            match text_res.get_string(popup_table, i) {
                Ok(s) => names.popup_texts.push(s.to_owned()),
                Err(e) => {
                    tracing::debug!("popup text {popup_table}/{i}: {e}");
                    names.popup_texts.push(String::new());
                }
            }
        }
    }

    // Short-briefing texts.
    let sb_table = descriptors.short_briefing.text_table_id;
    if sb_table != 0
        && let Ok(count) = text_res.get_string_count(sb_table)
    {
        names.short_briefing_texts.reserve(count);
        for i in 0..count {
            match text_res.get_string(sb_table, i) {
                Ok(s) => names.short_briefing_texts.push(s.to_owned()),
                Err(e) => {
                    tracing::debug!("short briefing text {sb_table}/{i}: {e}");
                    names.short_briefing_texts.push(String::new());
                }
            }
        }
    }
}

/// Three-step cascade mirrors `robin_rs::main_entry::load_profiles`:
///   1. Pre-parsed profiles from a shipping datadir manifest (if any).
///   2. JSON dump at `Data/Configuration/profile.cpf.json`.
///   3. Binary `Data/Configuration/profile.cpf` via the legacy loader.
fn load_profile_manager(datadir: &Path) -> Result<ProfileManager, LoadError> {
    let data_dir = datadir.join("Data");
    if let Ok(Some(shipping)) = crate::shipping_datadir::try_load(&data_dir)
        && let Some(p) = shipping.profiles.as_ref()
    {
        return Ok(p.clone());
    }
    let json_path = data_dir.join("Configuration").join("profile.cpf.json");
    if json_path.exists() {
        return ProfileManager::load_json(&json_path.to_string_lossy())
            .map_err(LoadError::Profiles);
    }
    let cpf_path = data_dir.join("Configuration").join("profile.cpf");
    let mut file = SbFile::open(&cpf_path.to_string_lossy(), SB_FILE_READ)
        .map_err(|e| LoadError::Profiles(format!("open {}: error {e}", cpf_path.display())))?;
    let mut mgr = ProfileManager::new();
    mgr.load_all_legacy_cpf(&mut file)
        .map_err(|e| LoadError::Profiles(format!("parse {}: error {e}", cpf_path.display())))?;
    Ok(mgr)
}

fn build_names(loaded: &LoadedLevel, profiles: &ProfileManager) -> ActorNames {
    let mut slots = Slots::default();

    push_patch_fx_slots(&loaded.proto.patches, &mut slots);
    push_animation_slots(&loaded.proto, &mut slots);
    push_patch_fx_slots(&loaded.mission.mission_patches, &mut slots);
    push_civilians(&loaded.mission, profiles, &mut slots);
    push_pcs_to_rescue(&loaded.mission, profiles, &mut slots);
    push_soldiers(&loaded.mission, profiles, &mut slots);
    push_targets(&loaded.mission, &mut slots);
    push_bonuses(&loaded.mission, &mut slots);
    push_scrolls(&loaded.mission, &mut slots);

    let patches = collect_patch_names(
        loaded
            .proto
            .patches
            .iter()
            .chain(&loaded.mission.mission_patches),
    );
    let class_kinds = collect_class_kinds(&loaded.mission);

    ActorNames {
        actors: slots.actors,
        actor_kinds: slots.kinds,
        patches,
        class_kinds,
        popup_texts: Vec::new(),
        short_briefing_texts: Vec::new(),
    }
}

/// Accumulates the three parallel arrays (`actors`, `kinds`, and a shared
/// dedup counter) that the `push_*` helpers build up.
#[derive(Default)]
struct Slots {
    actors: Vec<Option<String>>,
    kinds: Vec<Option<ActorSlotKind>>,
    /// Dedup counter per kind. Shared so `Actors.Guard_A04` and
    /// `Actors.Guard_A04_2` stay stable, but also scoped per kind so
    /// `Anim.Fumee` and `PatchFx.Fumee` don't collide with each other.
    used: HashMap<(ActorSlotKind, String), usize>,
}

impl Slots {
    fn push_with_name(&mut self, kind: ActorSlotKind, base: Option<String>) {
        let named = base.and_then(|b| {
            if b.is_empty() {
                return None;
            }
            let counter = self.used.entry((kind, b.clone())).or_insert(0);
            *counter += 1;
            Some(if *counter == 1 {
                b
            } else {
                format!("{b}_{counter}")
            })
        });
        self.actors.push(named);
        self.kinds.push(Some(kind));
    }
}

fn collect_class_kinds(mission: &LoadedMission) -> HashMap<String, ScriptKind> {
    let mut out: HashMap<String, ScriptKind> = HashMap::new();
    let mut mark = |cls: &Option<String>, kind: ScriptKind| {
        if let Some(name) = cls.as_ref().filter(|n| !n.is_empty()) {
            // First-wins — a class shouldn't appear bound to two slots,
            // but if it does we keep the first mapping rather than churn.
            out.entry(name.clone()).or_insert(kind);
        }
    };
    for s in &mission.soldiers {
        mark(&s.script_class, ScriptKind::Actor);
    }
    for c in &mission.civilians {
        mark(&c.script_class, ScriptKind::Actor);
    }
    for p in &mission.pcs_to_rescue {
        mark(&p.script_class, ScriptKind::Actor);
    }
    for s in &mission.scrolls {
        mark(&s.script_class, ScriptKind::Scroll);
    }
    for t in &mission.targets {
        mark(&t.script_class, ScriptKind::Target);
    }
    if let Some(so) = mission.script_objects.as_ref() {
        for sect in &so.sectors {
            mark(&sect.script_class, ScriptKind::Zone);
        }
    }
    for path in &mission.hiking_paths {
        for wp in &path.waypoints {
            if let WaypointCommand::Script(s) = &wp.command {
                out.entry(s.clone()).or_insert(ScriptKind::Waypoint);
            }
        }
    }
    out
}

fn push_patch_fx_slots(patches: &[robin_engine::level_data::RawPatch], slots: &mut Slots) {
    // Only patches with a non-empty frame profile spawn a script-element.
    for patch in patches {
        if patch.element_fx.sprite.frame_profile_name.is_empty() {
            continue;
        }
        slots.push_with_name(
            ActorSlotKind::PatchFx,
            Some(sanitize_identifier(&patch.element_fx.sprite.profile_name)),
        );
    }
}

fn push_animation_slots(proto: &LoadedProtoLevel, slots: &mut Slots) {
    for anim in &proto.animations {
        slots.push_with_name(
            ActorSlotKind::Anim,
            Some(sanitize_identifier(&anim.sprite.profile_name)),
        );
    }
}

fn push_civilians(mission: &LoadedMission, profiles: &ProfileManager, slots: &mut Slots) {
    for civ in &mission.civilians {
        slots.push_with_name(
            ActorSlotKind::Actor,
            pick_base_name(
                civ.script_class.as_deref(),
                profile_filename_civilian(civ, profiles),
            ),
        );
    }
}

fn push_pcs_to_rescue(mission: &LoadedMission, profiles: &ProfileManager, slots: &mut Slots) {
    for pc in &mission.pcs_to_rescue {
        slots.push_with_name(
            ActorSlotKind::Actor,
            pick_base_name(
                pc.script_class.as_deref(),
                profile_filename_character(pc, profiles),
            ),
        );
    }
}

fn push_soldiers(mission: &LoadedMission, profiles: &ProfileManager, slots: &mut Slots) {
    for s in &mission.soldiers {
        slots.push_with_name(
            ActorSlotKind::Actor,
            pick_base_name(
                s.script_class.as_deref(),
                profile_filename_soldier(s, profiles),
            ),
        );
    }
}

fn push_targets(mission: &LoadedMission, slots: &mut Slots) {
    for t in &mission.targets {
        slots.push_with_name(
            ActorSlotKind::Target,
            pick_base_name(t.script_class.as_deref(), None),
        );
    }
}

fn push_bonuses(mission: &LoadedMission, slots: &mut Slots) {
    // Bonuses have no script_class, but their `bonus_type` ordinal
    // maps to a stable enum (Arrow, Apple, Purse, …) — good enough
    // for a readable `Bonuses.Apple_3` identifier.
    for b in &mission.bonuses {
        let name = BonusItemType::from_u16(b.bonus_type).map(|t| format!("{t:?}"));
        slots.push_with_name(ActorSlotKind::Bonus, name);
    }
}

fn push_scrolls(mission: &LoadedMission, slots: &mut Slots) {
    for s in &mission.scrolls {
        slots.push_with_name(
            ActorSlotKind::Scroll,
            pick_base_name(s.script_class.as_deref(), None),
        );
    }
}

fn profile_filename_soldier(s: &RawSoldier, profiles: &ProfileManager) -> Option<String> {
    profiles
        .get_soldier(s.profile_number)
        .map(|p| p.filename.clone())
}

fn profile_filename_civilian(c: &RawCivilian, profiles: &ProfileManager) -> Option<String> {
    profiles
        .get_civilian(c.profile_number)
        .map(|p| p.filename.clone())
}

fn profile_filename_character(pc: &RawPcRescue, profiles: &ProfileManager) -> Option<String> {
    profiles
        .get_character(pc.profile_index)
        .map(|p| p.filename.clone())
}

/// Pick the "base" name for a slot (pre-dedup): the script-class name
/// if present, otherwise the profile filename. `Slots::push_with_name`
/// handles suffixing (`_2`, `_3`, …) within each kind.
fn pick_base_name(script_class: Option<&str>, fallback: Option<String>) -> Option<String> {
    script_class
        .filter(|s| !s.is_empty())
        .map(sanitize_class_name)
        .or_else(|| fallback.as_deref().map(sanitize_identifier))
        .filter(|s| !s.is_empty())
}

fn collect_patch_names<'a>(
    patches: impl Iterator<Item = &'a robin_engine::level_data::RawPatch>,
) -> Vec<Option<String>> {
    let mut out = Vec::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    for patch in patches {
        let profile = &patch.element_fx.sprite.profile_name;
        let base = sanitize_identifier(profile);
        if base.is_empty() {
            out.push(None);
            continue;
        }
        let count = used.entry(base.clone()).or_insert(0);
        *count += 1;
        out.push(Some(if *count == 1 {
            base
        } else {
            format!("{base}_{count}")
        }));
    }
    out
}

/// Sanitize to a TypeScript-friendly identifier: keep alphanumerics,
/// replace runs of other characters with a single `_`, trim outer `_`.
fn sanitize_identifier(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_underscore = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_was_underscore = false;
        } else if !last_was_underscore {
            out.push('_');
            last_was_underscore = true;
        }
    }
    out.trim_matches('_').to_owned()
}

/// `"Femme_officier_8000023e"` → `"Femme_officier"`.
fn sanitize_class_name(class: &str) -> String {
    let trimmed = match class.rsplit_once('_') {
        Some((head, tail)) if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_hexdigit()) => {
            head
        }
        _ => class,
    };
    sanitize_identifier(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_8hex_suffix() {
        assert_eq!(
            sanitize_class_name("Femme_officier_8000023e"),
            "Femme_officier"
        );
        assert_eq!(sanitize_class_name("Parchment_80000239"), "Parchment");
        assert_eq!(sanitize_class_name("PlainName"), "PlainName");
        assert_eq!(sanitize_class_name("Name_ghijklmn"), "Name_ghijklmn");
    }

    #[test]
    fn sanitize_collapses_runs_and_trims() {
        assert_eq!(
            sanitize_identifier("Leicester - Patch03"),
            "Leicester_Patch03"
        );
        assert_eq!(sanitize_identifier("Guard A01"), "Guard_A01");
        assert_eq!(sanitize_identifier("  foo--bar  "), "foo_bar");
    }
}
