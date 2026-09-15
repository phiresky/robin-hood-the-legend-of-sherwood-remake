//! Mission VM ownership, persistence, bindings, and callback state.
use super::*;

// ─── Mission script ─────────────────────────────────────────────────

/// Runtime-only stack of active script callback receivers.
///
/// The Original brackets its process-global script receiver around every
/// dispatch. Keeping the equivalent values in a structural stack makes nested
/// inheritance explicit and prevents callback state from leaking into saves.
#[derive(Clone, Debug, Default)]
struct ScriptCallStack {
    frames: Vec<(crate::natives::ScriptCallFrame, bool)>,
}

impl ScriptCallStack {
    fn push(&mut self, frame: crate::natives::ScriptCallFrame, vm_activation: bool) {
        self.frames.push((frame, vm_activation));
    }

    fn pop(&mut self) -> crate::natives::ScriptCallFrame {
        self.frames
            .pop()
            .expect("script call-frame stack underflow")
            .0
    }

    fn vm_depth(&self) -> usize {
        self.frames
            .iter()
            .filter(|(_, vm_activation)| *vm_activation)
            .count()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.frames.len()
    }

    fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// Wraps the script VM for a single mission level.
///
/// Holds the `ScriptManager` (loaded `.scb` bytecode), the global
/// `ScriptInstance` (bound to `StartUp`), and per-actor script instances
/// that persist across event callbacks (`Initialize`, `ActionChange`,
/// `HandleEvent`, `FilterAIEvent`, `ProcessMessage`).
///
/// One global engine-script instance plus one per-actor instance, each
/// with its own persistent heap.
///
/// Persistence is derived directly on this type. `remote = "Self"` turns the
/// derived serde code into inherent functions so the trait impls below can
/// reject active callback stacks before delegating. The bitcode wire of
/// `call_stack` is empty and asserts the same invariant.
#[derive(
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(remote = "Self")]
pub struct MissionScript {
    /// Mission base filename used to reattach immutable bytecode from
    /// [`LevelAssets`] after snapshot deserialization.
    pub script_name: String,
    pub(in crate::engine) manager: ScriptManager,
    /// Persistent state belonging to the script subsystem.
    pub state: ScriptState,
    /// Immutable level-native capabilities. Snapshot decode intentionally
    /// leaves this detached; the engine's snapshot adoption/restore boundary
    /// restores it from [`LevelAssets`] before the VM can resume.
    #[state_hash(skip)]
    #[serde(skip)]
    #[bitcode(skip)]
    pub(crate) bindings: crate::natives::AttachedScriptBindings,
    /// Active callback receivers. This is runtime control state, excluded from
    /// serialization and hashing; snapshots are rejected while it is nonempty
    /// (serde: `MissionScript::check_snapshot_safe`; bitcode: the empty
    /// `NativeBitcode` wire of `ScriptCallStack`).
    #[state_hash(skip)]
    #[serde(skip)]
    call_stack: ScriptCallStack,
    pub(in crate::engine) instance: ScriptInstance,
    /// Per-actor script instances, keyed by actor script handle.
    ///
    /// Each actor with a `script_class` gets a persistent `ScriptInstance`
    /// whose heap survives across calls.
    pub(in crate::engine) actor_instances: BTreeMap<i32, ScriptInstance>,
    /// Per-zone script instances, keyed by zone index (0-based index into
    /// `EngineInner::script_zone_grid_indices`).
    ///
    /// Zones with a `script_class` get a persistent `ScriptInstance` for
    /// `Initialize`, `EnterZone(actor)`, and `ExitZone(actor)` callbacks.
    pub(in crate::engine) zone_instances: BTreeMap<usize, ScriptInstance>,
    /// Per-target script instances, keyed by target actor script handle.
    ///
    /// FX targets with a non-empty `script_class` get a persistent
    /// `ScriptInstance` whose heap survives across calls. Each target
    /// is its own VM, with named functions like `ActivatedByListenable`,
    /// `ActivatedByApple`, and `ActivatedByArrow`, dispatched by the sole
    /// `EngineInner` script driver.
    pub(in crate::engine) target_instances: BTreeMap<i32, ScriptInstance>,
    /// Per-scroll script instances, keyed by scroll actor script handle.
    ///
    /// Scrolls with a non-empty `script_class` bind their class during
    /// scroll mission-stream init and then run their script's
    /// `Initialize()` in `initialize_all_scrolls`. `IsTaken(pc)` is
    /// dispatched later when a PC picks up the scroll.
    pub(in crate::engine) scroll_instances: BTreeMap<i32, ScriptInstance>,
    /// Per-waypoint script instances, keyed by `(hiking_path_index,
    /// waypoint_index)`.
    ///
    /// Waypoints whose command is `WaypointCommand::Script(class)` bind
    /// the class at level load, run their `Initialize()` once, and then
    /// receive `ReachPoint(actor)` every time an NPC arrives at that
    /// waypoint (dispatched from `execute_waypoint_script`). Each
    /// waypoint is its own VM instance so the heap persists across
    /// traversals.
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    pub(in crate::engine) waypoint_instances: BTreeMap<(crate::ai::PathId, u8), ScriptInstance>,
    /// Entity callbacks implemented only by the attached Spellforge package.
    /// These identities are serialized so event routing survives save/load
    /// even when the companion class intentionally has no SCB equivalent.
    #[serde(default)]
    spellforge_virtual_instances: BTreeSet<ScriptVmKey>,
    #[serde(default)]
    spellforge_virtual_bindings_enabled: bool,
}

const ACTIVE_CALLBACK_SNAPSHOT_ERROR: &str =
    "cannot snapshot MissionScript during an active script callback";

/// Native snapshots carry no call-stack bytes (the same empty encoding as a
/// skipped field) but must never capture a synchronous callback in flight.
impl crate::bitcode_adapters::NativeBitcode for ScriptCallStack {
    type Wire = std::marker::PhantomData<()>;

    fn to_wire(&self) -> Self::Wire {
        assert!(self.is_empty(), "{ACTIVE_CALLBACK_SNAPSHOT_ERROR}");
        std::marker::PhantomData
    }

    fn from_wire(_wire: Self::Wire) -> Self {
        Self::default()
    }
}

crate::bitcode_adapters::impl_native_bitcode!(ScriptCallStack);

impl Serialize for MissionScript {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.check_snapshot_safe()
            .map_err(serde::ser::Error::custom)?;
        // Inherent function generated by `#[serde(remote = "Self")]`.
        MissionScript::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for MissionScript {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Inherent function generated by `#[serde(remote = "Self")]`.
        MissionScript::deserialize(deserializer)
    }
}

impl MissionScript {
    /// Saves and snapshots are rejected while a synchronous callback is active:
    /// skipping `call_stack` would otherwise silently persist a half-run VM.
    pub(crate) fn check_snapshot_safe(&self) -> Result<(), String> {
        if self.call_stack.is_empty() {
            Ok(())
        } else {
            Err(ACTIVE_CALLBACK_SNAPSHOT_ERROR.into())
        }
    }

    /// In-memory equivalent of a save round trip, without running a codec.
    ///
    /// Copy persistent owners directly; process attachments are reconstructed
    /// on restore and never copied into the snapshot.
    pub(crate) fn persisted_clone(&self) -> Result<Self, String> {
        self.check_snapshot_safe()?;
        Ok(Self {
            script_name: self.script_name.clone(),
            // Only the static area is saved; bytecode reattaches from LevelAssets.
            manager: self.manager.persisted_clone(),
            state: self.state.clone(),
            bindings: crate::natives::AttachedScriptBindings::default(),
            call_stack: ScriptCallStack::default(),
            instance: self.instance.clone(),
            actor_instances: self.actor_instances.clone(),
            zone_instances: self.zone_instances.clone(),
            target_instances: self.target_instances.clone(),
            scroll_instances: self.scroll_instances.clone(),
            waypoint_instances: self.waypoint_instances.clone(),
            spellforge_virtual_instances: self.spellforge_virtual_instances.clone(),
            spellforge_virtual_bindings_enabled: self.spellforge_virtual_bindings_enabled,
        })
    }
}

/// Which instance map a bound script class belongs to.
///
/// Used by [`MissionScript::bind_actor`] / [`MissionScript::bind_target`]
/// / [`MissionScript::bind_scroll`] to select the instance map for all three entity flavours.
/// Initialization runs separately through the engine callback driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptBindKind {
    Actor,
    Target,
    Scroll,
}

/// Identity of one persistent mission-script VM.
///
/// The engine's synchronous callback driver uses this enum for every script
/// flavour, so a VM yield cannot accidentally be supported only for actor
/// callbacks.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
pub(crate) enum ScriptVmKey {
    Global,
    Actor(i32),
    Zone(usize),
    Target(i32),
    Scroll(i32),
    Waypoint(crate::ai::PathId, u8),
}

impl std::fmt::Debug for MissionScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissionScript")
            .field("class_count", &self.manager.class_count())
            .field("actor_instances", &self.actor_instances.len())
            .field("zone_instances", &self.zone_instances.len())
            .finish()
    }
}

impl MissionScript {
    /// Immutable bytecode metadata for diagnostics. Execution remains owned
    /// by the engine driver; callers cannot reach a `ScriptInstance` here.
    pub fn scb(&self) -> &crate::scb::ScbFile {
        self.manager.scb()
    }

    /// Counts exposed to the HTTP diagnostics without publishing live VMs.
    pub fn instance_counts(&self) -> ScriptInstanceCounts {
        ScriptInstanceCounts {
            actors: self.actor_instances.len(),
            zones: self.zone_instances.len(),
            targets: self.target_instances.len(),
            scrolls: self.scroll_instances.len(),
            waypoints: self.waypoint_instances.len(),
        }
    }

    fn script_instance(&self, key: ScriptVmKey) -> Option<&ScriptInstance> {
        match key {
            ScriptVmKey::Global => Some(&self.instance),
            ScriptVmKey::Actor(handle) => self.actor_instances.get(&handle),
            ScriptVmKey::Zone(index) => self.zone_instances.get(&index),
            ScriptVmKey::Target(handle) => self.target_instances.get(&handle),
            ScriptVmKey::Scroll(handle) => self.scroll_instances.get(&handle),
            ScriptVmKey::Waypoint(path, waypoint) => self.waypoint_instances.get(&(path, waypoint)),
        }
    }

    pub(crate) fn script_vm_has_function(&self, key: ScriptVmKey, fn_name: &str) -> bool {
        self.script_instance(key)
            .is_some_and(|instance| instance.has_function(&self.manager, fn_name))
    }

    pub(crate) fn has_script_vm(&self, key: ScriptVmKey) -> bool {
        self.script_instance(key).is_some() || self.spellforge_virtual_instances.contains(&key)
    }

    pub(crate) fn has_scb_script_vm(&self, key: ScriptVmKey) -> bool {
        self.script_instance(key).is_some()
    }

    pub(crate) fn enable_spellforge_virtual_bindings(&mut self) {
        self.spellforge_virtual_bindings_enabled = true;
    }

    pub(crate) fn bind_spellforge_virtual_zone(&mut self, zone: usize) {
        assert!(
            self.spellforge_virtual_bindings_enabled,
            "virtual Spellforge zone binding used without a runtime"
        );
        self.spellforge_virtual_instances
            .insert(ScriptVmKey::Zone(zone));
    }

    /// Start a callback without executing its first opcode. The matching
    /// `resume_script_vm` calls are owned by `EngineInner`, which can service
    /// typed synchronous yields before allowing the VM to continue.
    pub(crate) fn begin_script_vm(
        &mut self,
        key: ScriptVmKey,
        fn_name: &str,
        params: &[i32],
    ) -> Result<crate::interp::VmActivationState, String> {
        if !self.script_vm_has_function(key, fn_name) {
            return Err(format!(
                "cannot begin required {key:?}.{fn_name}: method is not bound"
            ));
        }
        let MissionScript {
            manager,
            instance,
            actor_instances,
            zone_instances,
            target_instances,
            scroll_instances,
            waypoint_instances,
            ..
        } = self;
        let instance = match key {
            ScriptVmKey::Global => instance,
            ScriptVmKey::Actor(handle) => actor_instances
                .get_mut(&handle)
                .expect("validated actor script VM vanished"),
            ScriptVmKey::Zone(index) => zone_instances
                .get_mut(&index)
                .expect("validated zone script VM vanished"),
            ScriptVmKey::Target(handle) => target_instances
                .get_mut(&handle)
                .expect("validated target script VM vanished"),
            ScriptVmKey::Scroll(handle) => scroll_instances
                .get_mut(&handle)
                .expect("validated scroll script VM vanished"),
            ScriptVmKey::Waypoint(path, waypoint) => waypoint_instances
                .get_mut(&(path, waypoint))
                .expect("validated waypoint script VM vanished"),
        };
        instance
            .begin_activation(manager, fn_name, params)
            .map_err(|error| format!("{key:?} script {fn_name} failed to start: {error}"))
    }

    pub(crate) fn resume_script_vm(
        &mut self,
        key: ScriptVmKey,
        fn_name: &str,
        frame: crate::natives::ScriptCallFrame,
        activation: &mut crate::interp::VmActivationState,
        script_domains: &mut crate::engine::ScriptDomains,
        capabilities: &mut crate::natives::NativeSessionCapabilities<'_>,
    ) -> crate::interp::StopReason {
        let class_idx = self
            .script_instance(key)
            .expect("script VM vanished while preparing native diagnostics")
            .class_idx();
        let diagnostic = crate::sim_rng::ScriptVmDiagnosticContext {
            vm_key: format!("{key:?}"),
            class_name: self.manager.scb().classes[class_idx].class_name.clone(),
            method_name: fn_name.to_owned(),
            native_max: None,
        };
        let MissionScript {
            manager,
            state,
            bindings,
            instance,
            actor_instances,
            zone_instances,
            target_instances,
            scroll_instances,
            waypoint_instances,
            ..
        } = self;
        let instance = match key {
            ScriptVmKey::Global => instance,
            ScriptVmKey::Actor(handle) => actor_instances
                .get_mut(&handle)
                .expect("actor script VM vanished while suspended"),
            ScriptVmKey::Zone(index) => zone_instances
                .get_mut(&index)
                .expect("zone script VM vanished while suspended"),
            ScriptVmKey::Target(handle) => target_instances
                .get_mut(&handle)
                .expect("target script VM vanished while suspended"),
            ScriptVmKey::Scroll(handle) => scroll_instances
                .get_mut(&handle)
                .expect("scroll script VM vanished while suspended"),
            ScriptVmKey::Waypoint(path, waypoint) => waypoint_instances
                .get_mut(&(path, waypoint))
                .expect("waypoint script VM vanished while suspended"),
        };
        let mut context =
            NativeContext::with_call_frame(state, script_domains, bindings, capabilities, frame);
        context.script_vm_diagnostic = Some(diagnostic);
        instance.poll_activation_with_host(manager, activation, 10_000_000, fn_name, &mut context)
    }

    /// Build a [`MissionScript`] from an already-parsed `.scb` payload.
    pub fn from_scb(scb: crate::scb::ScbFile) -> Result<Self, String> {
        let program = crate::script_manager::ScriptProgram::from_scb(scb)
            .map_err(|error| error.to_string())?;
        Self::from_program(String::new(), std::sync::Arc::new(program))
    }

    /// Build a [`MissionScript`] from host-owned immutable bytecode.
    pub fn from_program(
        script_name: String,
        program: std::sync::Arc<crate::script_manager::ScriptProgram>,
    ) -> Result<Self, String> {
        Self::from_manager(script_name, ScriptManager::from_program(program))
    }

    fn from_manager(script_name: String, manager: ScriptManager) -> Result<Self, String> {
        let instance = manager
            .create_instance("StartUp")
            .map_err(|e| format!("No StartUp class in mission script: {e}"))?;

        Ok(Self {
            script_name,
            manager,
            state: ScriptState::default(),
            bindings: crate::natives::AttachedScriptBindings::default(),
            call_stack: ScriptCallStack::default(),
            instance,
            actor_instances: BTreeMap::new(),
            zone_instances: BTreeMap::new(),
            target_instances: BTreeMap::new(),
            scroll_instances: BTreeMap::new(),
            waypoint_instances: BTreeMap::new(),
            spellforge_virtual_instances: BTreeSet::new(),
            spellforge_virtual_bindings_enabled: false,
        })
    }

    pub(crate) fn attach_program(
        &mut self,
        program: std::sync::Arc<crate::script_manager::ScriptProgram>,
    ) {
        self.manager.attach_program(program);
    }

    pub(crate) fn attach_bindings(&mut self, bindings: crate::natives::AttachedScriptBindings) {
        self.bindings = bindings;
    }

    /// Identity and mutable heap of the engine-global Original VM object.
    ///
    /// Linux-v48 adoption validates the decoded member schema against this
    /// class before replacing the heap. Keeping the access here avoids
    /// exposing the global `ScriptInstance` as a second public state owner.
    pub(crate) fn global_vm_class_and_heap(&self) -> (&crate::scb::ClassEntry, &[u8]) {
        let class = &self.manager.program.scb().classes[self.instance.class_idx()];
        (class, &self.instance.vm.heap)
    }

    pub(crate) fn replace_global_vm_heap(&mut self, heap: Vec<u8>) {
        self.instance.vm.heap = heap;
    }

    /// Identity and heap of one mission-authored Actor VM.
    pub(crate) fn actor_vm_class_and_heap(
        &self,
        handle: i32,
    ) -> Option<(&crate::scb::ClassEntry, &[u8])> {
        let instance = self.actor_instances.get(&handle)?;
        let class = &self.manager.program.scb().classes[instance.class_idx()];
        Some((class, &instance.vm.heap))
    }

    pub(crate) fn replace_actor_vm_heap(&mut self, handle: i32, heap: Vec<u8>) {
        self.actor_instances
            .get_mut(&handle)
            .expect("preflighted actor VM disappeared")
            .vm
            .heap = heap;
    }

    /// Identity and heap of one mission-authored Target VM.
    pub(crate) fn target_vm_class_and_heap(
        &self,
        handle: i32,
    ) -> Option<(&crate::scb::ClassEntry, &[u8])> {
        let instance = self.target_instances.get(&handle)?;
        let class = &self.manager.program.scb().classes[instance.class_idx()];
        Some((class, &instance.vm.heap))
    }

    pub(crate) fn replace_target_vm_heap(&mut self, handle: i32, heap: Vec<u8>) {
        self.target_instances
            .get_mut(&handle)
            .expect("preflighted target VM disappeared")
            .vm
            .heap = heap;
    }

    /// Identity and heap of one mission-authored Scroll VM.
    pub(crate) fn scroll_vm_class_and_heap(
        &self,
        handle: i32,
    ) -> Option<(&crate::scb::ClassEntry, &[u8])> {
        let instance = self.scroll_instances.get(&handle)?;
        let class = &self.manager.program.scb().classes[instance.class_idx()];
        Some((class, &instance.vm.heap))
    }

    pub(crate) fn replace_scroll_vm_heap(&mut self, handle: i32, heap: Vec<u8>) {
        self.scroll_instances
            .get_mut(&handle)
            .expect("preflighted scroll VM disappeared")
            .vm
            .heap = heap;
    }

    /// Identity and heap of one mission-authored script-zone VM.
    pub(crate) fn zone_vm_class_and_heap(
        &self,
        zone: usize,
    ) -> Option<(&crate::scb::ClassEntry, &[u8])> {
        let instance = self.zone_instances.get(&zone)?;
        let class = &self.manager.program.scb().classes[instance.class_idx()];
        Some((class, &instance.vm.heap))
    }

    pub(crate) fn replace_zone_vm_heap(&mut self, zone: usize, heap: Vec<u8>) {
        self.zone_instances
            .get_mut(&zone)
            .expect("preflighted script-zone VM disappeared")
            .vm
            .heap = heap;
    }

    /// Identity and mutable heap of one mission-authored hiking waypoint VM.
    ///
    /// Original v48 saves serialize these instances in hiking-path order,
    /// before the engine-global VM. Legacy adoption maps that ordinal topology
    /// back to the stable `(PathId, waypoint)` owner and validates the class
    /// before replacing any heap bytes.
    pub(crate) fn waypoint_vm_class_and_heap(
        &self,
        path: crate::ai::PathId,
        waypoint: u8,
    ) -> Option<(&crate::scb::ClassEntry, &[u8])> {
        let instance = self.waypoint_instances.get(&(path, waypoint))?;
        let class = &self.manager.program.scb().classes[instance.class_idx()];
        Some((class, &instance.vm.heap))
    }

    pub(crate) fn replace_waypoint_vm_heap(
        &mut self,
        path: crate::ai::PathId,
        waypoint: u8,
        heap: Vec<u8>,
    ) -> bool {
        let Some(instance) = self.waypoint_instances.get_mut(&(path, waypoint)) else {
            return false;
        };
        instance.vm.heap = heap;
        true
    }

    pub(in crate::engine) fn push_active_driver_frame(
        &mut self,
        frame: crate::natives::ScriptCallFrame,
        vm_activation: bool,
    ) {
        self.call_stack.push(frame, vm_activation);
    }

    /// Count VM activations across every synchronous callback entry, including
    /// callbacks whose local native-driver stack starts empty. External-native
    /// receiver guards protect context but do not consume a recursion slot.
    pub(in crate::engine) fn active_vm_depth(&self) -> usize {
        self.call_stack.vm_depth()
    }

    pub(in crate::engine) fn active_script_frame(&self) -> Option<crate::natives::ScriptCallFrame> {
        self.call_stack.frames.last().map(|(frame, _)| *frame)
    }

    pub(in crate::engine) fn pop_active_driver_frame(
        &mut self,
        expected: crate::natives::ScriptCallFrame,
    ) {
        let popped = self.call_stack.pop();
        assert_eq!(popped, expected, "script call-frame stack order changed");
    }

    #[cfg(test)]
    pub(crate) fn active_call_frame_count(&self) -> usize {
        self.call_stack.len()
    }

    pub(crate) fn assert_no_active_call_frames(&self) {
        assert!(
            self.call_stack.is_empty(),
            "mission script crossed a session boundary with active call frames"
        );
    }

    /// Bind a script class to an entity handle, creating a persistent
    /// `ScriptInstance`. `EngineInner` invokes `Initialize()` through the
    /// sole shared callback driver after all instances are inserted.
    ///
    /// The resulting instance is inserted into `actor_instances` keyed by
    /// `handle`, so the Engine-owned [`ScriptVmKey::Actor`] driver finds it.
    ///
    /// A referenced missing class is structural level corruption unless
    /// virtual Spellforge bindings are enabled.
    pub(crate) fn bind_actor(&mut self, handle: i32, class_name: &str) {
        self.bind_instance(handle, class_name, ScriptBindKind::Actor)
    }

    /// Target analogue of [`bind_actor`]. Stores the created instance in
    /// `target_instances`; initialization is driven by `EngineInner`.
    pub(crate) fn bind_target(&mut self, handle: i32, class_name: &str) {
        self.bind_instance(handle, class_name, ScriptBindKind::Target)
    }

    /// Scroll analogue of [`bind_actor`]. Stores the created instance in
    /// `scroll_instances`; initialization is driven by `EngineInner`.
    pub(crate) fn bind_scroll(&mut self, handle: i32, class_name: &str) {
        self.bind_instance(handle, class_name, ScriptBindKind::Scroll)
    }

    /// Shared implementation for [`bind_actor`], [`bind_target`], and
    /// [`bind_scroll`]: look up the class, create an instance, and insert it
    /// into the appropriate map.
    fn bind_instance(&mut self, handle: i32, class_name: &str, kind: ScriptBindKind) {
        let class_idx = match self.manager.find_class(class_name) {
            Some(idx) => idx,
            None if self.spellforge_virtual_bindings_enabled => {
                let key = match kind {
                    ScriptBindKind::Actor => ScriptVmKey::Actor(handle),
                    ScriptBindKind::Target => ScriptVmKey::Target(handle),
                    ScriptBindKind::Scroll => ScriptVmKey::Scroll(handle),
                };
                self.spellforge_virtual_instances.insert(key);
                return;
            }
            None => {
                panic!("{kind:?} script class '{class_name}' not found in SCB (handle {handle})")
            }
        };
        let inst = self.manager.create_instance_idx(class_idx);

        match kind {
            ScriptBindKind::Actor => {
                self.actor_instances.insert(handle, inst);
            }
            ScriptBindKind::Target => {
                self.target_instances.insert(handle, inst);
            }
            ScriptBindKind::Scroll => {
                self.scroll_instances.insert(handle, inst);
            }
        }
    }

    /// True if `handle` has a bound actor script that defines `fn_name`.
    ///
    /// Used by `filter_stimulus` to distinguish a missing `FilterAIEvent`
    /// override from a script-authored zero return. The shared engine driver
    /// applies the original-game default of one only to missing behavior;
    /// a missing required actor VM remains an error.
    pub fn actor_has_function(&self, handle: i32, fn_name: &str) -> bool {
        self.actor_instances
            .get(&handle)
            .map(|inst| inst.has_function(&self.manager, fn_name))
            .unwrap_or(false)
    }

    /// Bind a waypoint-script class to a given `(path_idx, wp_idx)`
    /// pair, creating a persistent `ScriptInstance`. `EngineInner` invokes
    /// `Initialize()` through the shared driver.
    ///
    /// Missing referenced classes are structural level errors unless
    /// virtual Spellforge bindings are enabled.
    pub(crate) fn bind_waypoint(
        &mut self,
        path_idx: crate::ai::PathId,
        wp_idx: u8,
        class_name: &str,
    ) {
        let class_idx = match self.manager.find_class(class_name) {
            Some(idx) => idx,
            None if self.spellforge_virtual_bindings_enabled => {
                self.spellforge_virtual_instances
                    .insert(ScriptVmKey::Waypoint(path_idx, wp_idx));
                return;
            }
            None => panic!(
                "Waypoint script class '{class_name}' (path {path_idx}, wp {wp_idx}) not found in SCB"
            ),
        };
        let inst = self.manager.create_instance_idx(class_idx);
        self.waypoint_instances.insert((path_idx, wp_idx), inst);
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct ScriptInstanceCounts {
    pub actors: usize,
    pub zones: usize,
    pub targets: usize,
    pub scrolls: usize,
    pub waypoints: usize,
}
