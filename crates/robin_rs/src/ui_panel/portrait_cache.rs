use super::*;
use enum_map::EnumMap;

/// Per-character action icon bank: interaction state → the three action slots.
pub(super) type ActionIconBank = EnumMap<ActionButtonVisual, [Option<SurfaceHandle>; 3]>;

/// Pre-loaded portrait renderer surfaces and action button icons, keyed by [`CharacterKind`].
///
/// Loaded once at mission start from `Data/Interface/DEFAULT.RES`, then
/// passed to [`HudDrawCtx::draw_panel`] each frame.  The per-character arrays are
/// indexed via `CharacterKind::as_index()` (`CharacterKind::COUNT`
/// slots).
///
/// The remaining singleton `Option<SurfaceHandle>` fields are kept named on
/// purpose: each one is read by exactly one draw site with its own placement
/// rule, and the loader is already table-driven, so an enum-indexed map would
/// only rename `portraits.guard_surface` to `portraits.indicators[Guard]`.
pub struct PortraitCache {
    /// Unique retirement authority for every managed upload in this cache.
    pub(super) owned_surfaces: Vec<OwnedSurface>,
    /// Also binds the directly owned RGBA artwork to its originating renderer.
    pub(super) renderer_identity: Option<u64>,
    /// Borrowed renderer surface for each character's face portrait.
    pub(super) surfaces: [Option<SurfaceHandle>; CharacterKind::COUNT],
    /// Per-character action icons, indexed by interaction state then action.
    pub(super) action_surfaces: [Option<ActionIconBank>; CharacterKind::COUNT],
    /// Localized display name per character.
    pub(super) localized_names: [Option<String>; CharacterKind::COUNT],
    /// Generic scroll decoration surfaces (shared by all portraits).
    pub(super) top_scroll_surface: Option<SurfaceHandle>,
    pub(super) top_scroll_alt_surface: Option<SurfaceHandle>,
    pub(super) bottom_scroll_surface: Option<SurfaceHandle>,
    /// Authored 112x134 transparent scroll background for allied groups.
    pub(super) allied_portrait_background: Option<GpuImage>,
    /// Complete 112x50 visage strips for generic and named allied soldiers.
    pub(super) allied_visages: [Option<GpuImage>; ALLIED_VISAGE_COUNT],
    /// Transparent medieval brooch artwork for the transient and pinned states.
    pub(super) allied_pin_icons: [Option<GpuImage>; 2],
    /// State-specific artwork: three stances, two patrol states, and four
    /// formations, in that order.
    pub(super) allied_action_surfaces: [Option<GpuImage>; 9],
    /// Panel border frame pieces.
    pub(super) border_top_left: Option<SurfaceHandle>,
    pub(super) border_top_right: Option<SurfaceHandle>,
    pub(super) border_bottom_left: Option<SurfaceHandle>,
    pub(super) border_bottom_right: Option<SurfaceHandle>,
    pub(super) border_middle: Option<SurfaceHandle>,
    pub(super) portrait_page_left: Option<SurfaceHandle>,
    pub(super) portrait_page_right: Option<SurfaceHandle>,
    /// Fighting sword overlay surface per character.
    pub(super) fighting_surfaces: [Option<SurfaceHandle>; CharacterKind::COUNT],
    /// Guard indicator surface (RHID_GUARD=209).
    pub(super) guard_surface: Option<SurfaceHandle>,
    /// Trumpet/reinforcement indicator surface (RHID_TRUMPET=224).
    pub(super) trumpet_surface: Option<SurfaceHandle>,
    /// Amulet/clover indicator surface (RHID_CLOVER=165).
    /// Shown in burned state when PC is NOT guarded (player can click to revive).
    pub(super) amulet_surface: Option<SurfaceHandle>,
    /// Pixel-level hit mask for the top scroll surface.
    /// Used to reject clicks on transparent curved parchment edges.
    pub(super) top_scroll_hit_mask: Option<robin_engine::minimap::HitMask>,
    /// Quick-action slot icon (RHID_QUICKACTION, shared by all QA slots).
    pub(super) qa_icon_surface: Option<SurfaceHandle>,
    /// Quick-action slot icon while recording (RHID_QUICKACTION_IN_PROGRESS).
    pub(super) qa_icon_recording_surface: Option<SurfaceHandle>,
    /// PC-info popup backgrounds (RHID_INFO_POPUP_BKGND_{TINY,HUGE}).
    pub(super) info_popup_bg_tiny: Option<SurfaceHandle>,
    pub(super) info_popup_bg_huge: Option<SurfaceHandle>,
    /// PC-info popup pip sprites (RHID_INFO_POPUP_SWORD / BOW).  We blit
    /// the "on" pip for the first `n` slots and skip the rest.
    pub(super) info_popup_sword: Option<SurfaceHandle>,
    pub(super) info_popup_bow: Option<SurfaceHandle>,
    /// Blazon bar icon strip — sub_ids: 0 = empty, 1 = normal (won),
    /// 2 = castle (to-collect).  We load the tiny set (used when the bar
    /// is the thin top strip).
    pub(super) blazon_tiny_empty: Option<SurfaceHandle>,
    pub(super) blazon_tiny_normal: Option<SurfaceHandle>,
    pub(super) blazon_tiny_castle: Option<SurfaceHandle>,
    /// Per-(resource, sub_id) surface cache for resources that carry a
    /// table of sub-pictures indexed by character profile / action
    /// (requirements bar per-slot icons).  Pre-loaded at level load so the
    /// HUD can blit any `(res_id, sub_id)` without holding a
    /// `ResourceManager` borrow across the draw path.
    pub(super) sub_pictures: HashMap<(ResourceId, usize), SurfaceHandle>,
    /// `RHID_YES_NO` status overlay — sub_id 0 = yes (green tick),
    /// sub_id 1 = no (red cross).
    pub(super) req_yes: Option<SurfaceHandle>,
    pub(super) req_no: Option<SurfaceHandle>,
    /// `RHID_SELECTED_ACTION` overlay marker used to highlight the
    /// currently-selected slot on the requirements bar.
    pub(super) req_selected: Option<SurfaceHandle>,
}

impl Default for PortraitCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PortraitCache {
    pub fn new() -> Self {
        Self {
            owned_surfaces: Vec::new(),
            renderer_identity: None,
            surfaces: [None; CharacterKind::COUNT],
            action_surfaces: [None; CharacterKind::COUNT],
            localized_names: [const { None }; CharacterKind::COUNT],
            top_scroll_surface: None,
            top_scroll_alt_surface: None,
            bottom_scroll_surface: None,
            allied_portrait_background: None,
            allied_visages: [const { None }; ALLIED_VISAGE_COUNT],
            allied_pin_icons: [None, None],
            allied_action_surfaces: [const { None }; 9],
            border_top_left: None,
            border_top_right: None,
            border_bottom_left: None,
            border_bottom_right: None,
            border_middle: None,
            portrait_page_left: None,
            portrait_page_right: None,
            fighting_surfaces: [None; CharacterKind::COUNT],
            guard_surface: None,
            trumpet_surface: None,
            amulet_surface: None,
            top_scroll_hit_mask: None,
            qa_icon_surface: None,
            qa_icon_recording_surface: None,
            info_popup_bg_tiny: None,
            info_popup_bg_huge: None,
            info_popup_sword: None,
            info_popup_bow: None,
            blazon_tiny_empty: None,
            blazon_tiny_normal: None,
            blazon_tiny_castle: None,
            sub_pictures: HashMap::new(),
            req_yes: None,
            req_no: None,
            req_selected: None,
        }
    }

    /// Load portrait pictures for all known characters.
    ///
    /// Reads each portrait resource from the resource manager, converts
    /// to a renderer surface. Missing resources are logged and skipped.
    /// Replace all artwork as one transaction. Optional resource misses remain empty.
    /// Required-art or upload errors retire the candidate and preserve the live cache.
    pub fn load(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
        files: &robin_engine::sbfile::SbFileSystem,
    ) -> anyhow::Result<()> {
        self.replace_with(renderer, |candidate, renderer| {
            candidate.load_contents(res, renderer, files)
        })
    }

    pub(super) fn validate_renderer_identity(&self, renderer: &Renderer) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.renderer_identity
                .is_none_or(|identity| identity == renderer.identity()),
            "portrait cache belongs to another renderer"
        );
        Ok(())
    }

    pub(super) fn validate_renderer(&self, renderer: &Renderer) -> anyhow::Result<()> {
        self.validate_renderer_identity(renderer)?;
        for surface in &self.owned_surfaces {
            renderer.validate_surface_retirement(surface)?;
        }
        Ok(())
    }

    pub(super) fn replace_with(
        &mut self,
        renderer: &mut Renderer,
        load: impl FnOnce(&mut Self, &mut Renderer) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.validate_renderer(renderer)?;
        let mut candidate = Self::new();
        candidate.renderer_identity = Some(renderer.identity());
        if let Err(error) = load(&mut candidate, renderer) {
            candidate.retire(renderer)?;
            return Err(error);
        }
        candidate.validate_renderer(renderer)?;
        self.retire(renderer)?;
        candidate.localized_names = std::mem::take(&mut self.localized_names);
        *self = candidate;
        Ok(())
    }

    /// Validate the complete bank before changing residency. Repeated retirement is safe.
    pub fn retire(&mut self, renderer: &mut Renderer) -> anyhow::Result<()> {
        self.validate_renderer(renderer)?;
        for surface in self.owned_surfaces.drain(..) {
            renderer.retire_surface(surface);
        }
        let names = std::mem::take(&mut self.localized_names);
        *self = Self::new(); // Drops the directly owned RGBA textures as well.
        self.localized_names = names;
        Ok(())
    }

    /// Upload every asset group in a fixed order (the order of
    /// `owned_surfaces` follows it).
    pub(super) fn load_contents(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
        files: &robin_engine::sbfile::SbFileSystem,
    ) -> anyhow::Result<()> {
        let mut timer = crate::game_session::PhaseTimer::new("portrait cache load");
        self.load_character_portraits(res, renderer)?;
        timer.step("character portraits");
        self.load_allied_art(renderer, files)?;
        timer.step("embedded allied art");
        self.load_scrolls_and_borders(res, renderer)?;
        timer.step("scrolls + borders");
        self.load_action_icons(res, renderer)?;
        timer.step("action icons");
        self.load_fighting_overlays(res, renderer)?;
        self.load_indicators_and_overlays(res, renderer)?;
        timer.step("indicators + blazons + overlays");
        self.load_requirement_tables(res, renderer)?;
        timer.step("requirements tables");
        timer.total();
        Ok(())
    }

    /// Upload the first available of `sub_ids` for an optional resource.
    /// A miss is logged with the last lookup error and leaves the slot empty.
    fn load_optional_picture(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
        res_id: ResourceId,
        sub_ids: &[usize],
        label: &str,
    ) -> anyhow::Result<Option<SurfaceHandle>> {
        let mut last_error = None;
        for &sub_id in sub_ids {
            match res.get_picture(res_id, sub_id) {
                Ok(pic) => {
                    let sid = owned_picture_surface(renderer, &mut self.owned_surfaces, pic)?;
                    tracing::info!(
                        "Loaded {label}: resource {res_id} sub {sub_id}, surface {sid:?} ({}x{})",
                        pic.width,
                        pic.height,
                    );
                    return Ok(Some(sid));
                }
                Err(error) => last_error = Some(error),
            }
        }
        let error = last_error.expect("optional picture lookup needs at least one sub_id");
        tracing::warn!("Failed to load {label} (resource {res_id}): {error}");
        Ok(None)
    }

    fn load_character_portraits(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
    ) -> anyhow::Result<()> {
        for kind in CharacterKind::VARIANTS {
            // Portrait resources are BTTN type with bitmask 0b1110;
            // sub_id 0 is absent, sub_id 1 is the default portrait.
            self.surfaces[kind.as_index()] = self.load_optional_picture(
                res,
                renderer,
                kind.portrait_resource(),
                &[1],
                &format!("portrait for {kind:?}"),
            )?;
        }

        tracing::info!(
            "Portrait cache: {} surfaces loaded",
            self.surfaces.iter().filter(|s| s.is_some()).count(),
        );
        Ok(())
    }

    fn load_allied_art(
        &mut self,
        renderer: &mut Renderer,
        files: &robin_engine::sbfile::SbFileSystem,
    ) -> anyhow::Result<()> {
        self.allied_portrait_background = Some(load_ui_image(
            renderer,
            files,
            "allied_portrait_background.png",
            (ELEMENT_WIDTH, PORTRAIT_TOTAL_HEIGHT),
            "allied portrait background",
        )?);

        for (kind, file) in AlliedVisageKind::VARIANTS.into_iter().zip([
            "allied_portrait_generic.png",
            "allied_portrait_guisbourne.png",
            "allied_portrait_longchamp.png",
            "allied_portrait_prince_john.png",
            "allied_portrait_scathlock.png",
            "allied_portrait_sheriff.png",
        ]) {
            self.allied_visages[kind.index()] = Some(load_ui_image(
                renderer,
                files,
                file,
                (ELEMENT_WIDTH, VISAGE_HEIGHT),
                file,
            )?);
        }

        for (index, (file, label)) in [
            ("allied_pin_unpinned.png", "allied portrait pin (unpinned)"),
            ("allied_pin_pinned.png", "allied portrait pin (pinned)"),
        ]
        .into_iter()
        .enumerate()
        {
            self.allied_pin_icons[index] = Some(load_ui_image(
                renderer,
                files,
                file,
                (ALLIED_PIN_ICON_SIZE, ALLIED_PIN_ICON_SIZE),
                label,
            )?);
        }

        for (index, file) in [
            "allied_stance_hold.png",
            "allied_stance_defensive.png",
            "allied_stance_aggressive.png",
            "allied_patrol_off.png",
            "allied_patrol_on.png",
            "allied_formation_line.png",
            "allied_formation_box.png",
            "allied_formation_staggered.png",
            "allied_formation_flank.png",
        ]
        .into_iter()
        .enumerate()
        {
            self.allied_action_surfaces[index] = Some(load_ui_image(
                renderer,
                files,
                file,
                (ALLIED_ACTION_ICON_WIDTH, ALLIED_ACTION_ICON_HEIGHT),
                &format!("allied state icon {index}"),
            )?);
        }
        Ok(())
    }

    fn load_scrolls_and_borders(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
    ) -> anyhow::Result<()> {
        // ── Load scroll decoration surfaces (generic, shared by all portraits) ──
        // Build pixel-level hit mask for the top scroll so clicks on
        // transparent curved parchment edges fall through.
        if let Ok(pic) = res.get_picture(RHID_TOP_SCROLL, 1) {
            let tc = crate::renderer::TRANSPARENT_COLOR_KEY_16;
            self.top_scroll_hit_mask = Some(picture_hit_mask(pic, tc)?);
            tracing::info!("Built top scroll hit mask ({}x{})", pic.width, pic.height);
        }
        self.top_scroll_surface =
            self.load_optional_picture(res, renderer, RHID_TOP_SCROLL, &[1], "top scroll")?;
        self.top_scroll_alt_surface = self.load_optional_picture(
            res,
            renderer,
            RHID_TOP_SCROLL_ALTERNATE,
            &[1],
            "top scroll alt",
        )?;
        self.bottom_scroll_surface =
            self.load_optional_picture(res, renderer, RHID_BOTTOM_SCROLL, &[1], "bottom scroll")?;

        self.portrait_page_left = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_PORTRAIT_SCROLL_LEFT,
            &[1, 0],
            "portrait page left",
        )?;
        self.portrait_page_right = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_PORTRAIT_SCROLL_RIGHT,
            &[1, 0],
            "portrait page right",
        )?;

        // ── Load panel border frame pieces ──
        // Choose the center piece based on screen width (800 vs 1024).
        let middle_id = if renderer.screen_width() >= 1024 {
            RHID_MIDDLE_1024
        } else {
            RHID_MIDDLE_800
        };
        self.border_top_left = self.load_optional_picture(
            res,
            renderer,
            RHID_TOP_LEFT_CORNER,
            &[0],
            "border top-left",
        )?;
        self.border_top_right = self.load_optional_picture(
            res,
            renderer,
            RHID_TOP_RIGHT_CORNER,
            &[0],
            "border top-right",
        )?;
        self.border_bottom_left = self.load_optional_picture(
            res,
            renderer,
            RHID_BOTTOM_LEFT_CORNER,
            &[0],
            "border bottom-left",
        )?;
        self.border_bottom_right = self.load_optional_picture(
            res,
            renderer,
            RHID_BOTTOM_RIGHT_CORNER,
            &[0],
            "border bottom-right",
        )?;
        self.border_middle =
            self.load_optional_picture(res, renderer, middle_id, &[0], "border middle")?;
        Ok(())
    }

    /// Load action button icons (normal + focused + pressed states).
    fn load_action_icons(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
    ) -> anyhow::Result<()> {
        for kind in CharacterKind::VARIANTS {
            let slot = kind.as_index();
            let action_res_ids = kind.action_resources();
            let mut icons = ActionIconBank::default();
            for (i, opt_id) in action_res_ids.iter().enumerate() {
                let Some(res_id) = opt_id else {
                    continue;
                };
                // Preserve Original radio sub-id order, including focused selected.
                for (state, sub_id) in [
                    (ActionButtonVisual::Disabled, ACTION_SUB_ID_DISABLED),
                    (ActionButtonVisual::Normal, ACTION_SUB_ID_UNSELECTED),
                    (ActionButtonVisual::Hover, ACTION_SUB_ID_FOCUSED),
                    (ActionButtonVisual::Pressed, ACTION_SUB_ID_SELECTED),
                    (
                        ActionButtonVisual::HoverPressed,
                        ACTION_SUB_ID_FOCUSED_SELECTED,
                    ),
                ] {
                    match res.get_picture(*res_id, sub_id) {
                        Ok(pic) => {
                            icons[state][i] = Some(owned_picture_surface(
                                renderer,
                                &mut self.owned_surfaces,
                                pic,
                            )?)
                        }
                        Err(error) if state == ActionButtonVisual::Normal => {
                            tracing::warn!(?kind, action = i, res_id, %error, "Failed to load action icon")
                        }
                        Err(_) => {} // Optional interaction states retain draw-time fallback.
                    }
                }
            }
            self.action_surfaces[slot] = Some(icons);
        }
        tracing::info!(
            "Portrait cache: {} action icon sets loaded",
            self.action_surfaces.iter().filter(|s| s.is_some()).count(),
        );
        Ok(())
    }

    /// Load fighting sword overlay surfaces (per character).
    fn load_fighting_overlays(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
    ) -> anyhow::Result<()> {
        for kind in CharacterKind::VARIANTS {
            // Fighting overlays are PICT type; sub_id 0 is the default
            // picture, sub_id 1 the fallback for resources using BTTN layout.
            self.fighting_surfaces[kind.as_index()] = self.load_optional_picture(
                res,
                renderer,
                kind.fighting_resource(),
                &[0, 1],
                &format!("fighting overlay for {kind:?}"),
            )?;
        }
        tracing::info!(
            "Portrait cache: {} fighting overlays loaded",
            self.fighting_surfaces
                .iter()
                .filter(|s| s.is_some())
                .count(),
        );
        Ok(())
    }

    fn load_indicators_and_overlays(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
    ) -> anyhow::Result<()> {
        // ── Load guard and trumpet indicator surfaces ──
        self.guard_surface = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_GUARD,
            &[0, 1],
            "guard indicator",
        )?;
        self.trumpet_surface = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_TRUMPET,
            &[0, 1],
            "trumpet indicator",
        )?;
        self.amulet_surface = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_CLOVER,
            &[0, 1],
            "amulet/clover indicator",
        )?;

        // ── Load QA icon surfaces (RHID_QUICKACTION / _IN_PROGRESS) ──
        // RHID_QUICKACTION is the normal icon and RHID_QUICKACTION_IN_PROGRESS
        // is the recording-alternate.  Shared across all PCs and all three slots.
        self.qa_icon_surface = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_QUICKACTION,
            &[1, 0],
            "QA icon",
        )?;
        self.qa_icon_recording_surface = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_QUICKACTION_IN_PROGRESS,
            &[1, 0],
            "QA icon (recording)",
        )?;

        // ── Load PC-info popup resources (backgrounds + pips) ──
        // Backgrounds and pip rows both live at sub_id 0.  We blit one pip
        // per lit slot rather than maintaining widget visibility flags.
        self.info_popup_bg_tiny = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_INFO_POPUP_BKGND_TINY,
            &[0, 1],
            "info popup bg (tiny)",
        )?;
        self.info_popup_bg_huge = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_INFO_POPUP_BKGND_HUGE,
            &[0, 1],
            "info popup bg (huge)",
        )?;
        self.info_popup_sword = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_INFO_POPUP_SWORD,
            &[0, 1],
            "info popup sword pip",
        )?;
        self.info_popup_bow = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_INFO_POPUP_BOW,
            &[0, 1],
            "info popup bow pip",
        )?;

        // ── Load blazon-bar icons (tiny set) ──
        // `RHID_BLAZON_TINY` carries 3 sub-pictures: 0 = empty, 1 = normal
        // (won), 2 = castle (to-collect).  The tiny set is the default
        // layout on 800+ width panels.
        let blazon = resource_ids::RHID_BLAZON_TINY;
        self.blazon_tiny_empty =
            self.load_optional_picture(res, renderer, blazon, &[0], "blazon tiny empty")?;
        self.blazon_tiny_normal =
            self.load_optional_picture(res, renderer, blazon, &[1], "blazon tiny normal")?;
        self.blazon_tiny_castle =
            self.load_optional_picture(res, renderer, blazon, &[2], "blazon tiny castle")?;

        // ── Load requirements-bar status overlays (yes/no, selected) ──
        // `RHID_YES_NO` has sub 0 = yes tick, sub 1 = no cross.
        let yes_no = resource_ids::RHID_YES_NO;
        self.req_yes =
            self.load_optional_picture(res, renderer, yes_no, &[0], "requirements yes overlay")?;
        self.req_no =
            self.load_optional_picture(res, renderer, yes_no, &[1], "requirements no overlay")?;
        self.req_selected = self.load_optional_picture(
            res,
            renderer,
            resource_ids::RHID_SELECTED_ACTION,
            &[0],
            "requirements selected overlay",
        )?;
        Ok(())
    }

    /// Pre-load all per-slot sub-pictures of the requirements-bar icon
    /// tables.  Each resource carries one sub-picture per character-profile
    /// or per-action enum value.  Loading the full table here lets
    /// `draw_requirements_bar` blit `(res_id, sub_id)` without ever
    /// re-borrowing the `ResourceManager` at render time.
    fn load_requirement_tables(
        &mut self,
        res: &mut ResourceManager,
        renderer: &mut Renderer,
    ) -> anyhow::Result<()> {
        for res_id in [
            resource_ids::RHID_REQUIRED_PC,
            resource_ids::RHID_REQUIRED_ACTION,
            resource_ids::RHID_OPTIONAL_PC,
        ] {
            let pictures = match res.get_pictures(res_id) {
                Ok(pictures) => pictures,
                Err(error) => {
                    tracing::warn!("Failed to load sub-pictures for resource {res_id}: {error}");
                    continue;
                }
            };
            for (sub_id, picture) in pictures.iter().enumerate() {
                let Some(picture) = picture else { continue };
                let surface = owned_picture_surface(renderer, &mut self.owned_surfaces, picture)?;
                tracing::debug!(
                    "Loaded requirements sub-picture: res {res_id} sub {sub_id}, surface {surface:?} ({}x{})",
                    picture.width,
                    picture.height,
                );
                self.sub_pictures.insert((res_id, sub_id), surface);
            }
        }
        Ok(())
    }

    /// Install a pre-loaded localized-name map.  Read at render time
    /// via [`Self::get_localized_name`] and by the peasant-name
    /// generator.  [`load_localized_character_names`] builds the map
    /// from `Level.res`.
    pub fn install_localized_names(&mut self, names: [Option<String>; CharacterKind::COUNT]) {
        self.localized_names = names;
    }

    /// Refresh data-authored hero/VIP names without rewriting the generated
    /// Merry Men names that are already part of campaign/replay identity.
    pub fn reload_localized_names_preserving_generated(
        &mut self,
        mut names: [Option<String>; CharacterKind::COUNT],
    ) {
        for kind in CharacterKind::VARIANTS {
            let index = kind.as_index();
            if matches!(
                kind,
                CharacterKind::MerryManA | CharacterKind::MerryManB | CharacterKind::MerryManC
            ) && self.localized_names[index].is_some()
            {
                continue;
            }
            self.localized_names[index] = names[index].take();
        }
    }

    /// Look up the renderer surface for a character's face portrait.
    pub fn get_surface(&self, kind: CharacterKind) -> Option<SurfaceHandle> {
        self.surfaces[kind.as_index()]
    }

    pub(super) fn action_icons(
        &self,
        kind: CharacterKind,
        state: ActionButtonVisual,
    ) -> Option<&[Option<SurfaceHandle>; 3]> {
        self.action_surfaces[kind.as_index()]
            .as_ref()
            .map(|icons| &icons[state])
    }

    /// Look up the fighting sword overlay surface for a character.
    pub fn get_fighting_surface(&self, kind: CharacterKind) -> Option<SurfaceHandle> {
        self.fighting_surfaces[kind.as_index()]
    }

    /// Look up the localized display name for a character.
    pub fn get_localized_name(&self, kind: CharacterKind) -> Option<&str> {
        self.localized_names[kind.as_index()].as_deref()
    }

    /// True if at least one portrait has been loaded.
    pub fn is_loaded(&self) -> bool {
        self.surfaces.iter().any(|s| s.is_some())
    }

    /// Look up a pre-loaded `(resource_id, sub_id)` surface.
    ///
    /// Populated at [`PortraitCache::load`] time for the requirements-bar
    /// icon tables (`RHID_REQUIRED_PC` / `RHID_REQUIRED_ACTION` /
    /// `RHID_OPTIONAL_PC`).
    pub fn get_sub_picture(&self, res_id: ResourceId, sub_id: usize) -> Option<SurfaceHandle> {
        self.sub_pictures.get(&(res_id, sub_id)).copied()
    }
}
