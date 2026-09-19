//! Explicit remote-code consent and durable trust-revocation screens.
//!
//! Consent is intentionally not the generic Yes/No dialog: Return is never
//! bound to approval, Escape and window close cancel, and the exact hashes,
//! authenticated distributor identity, licence attestation, and byte size are
//! visible before the player can click the approval button.

use crate::application::require;
use crate::cache_maintenance::CacheClearStatus;
use crate::host::ApplicationContext;
use crate::localization::PortTextKey;
use crate::native_font::Font;
use crate::renderer::Renderer;
use crate::spellforge_trust::{SpellforgeTrustGrant, SpellforgeTrustKey, SpellforgeTrustMetadata};
use crate::widget::FrameWnd;

use super::layout::{
    MenuTransform, align_bottom_right, draw_screen_background,
    elide_text_to_width_by as elide_to_width_by, render_text_virt_font as render_text_virt,
    wrap_text_for_box_font,
};
use super::resources::{IngameMenuResources, MT_BTN_BACK};
use super::widget_bridge::{self, ModalInputState, ModalScreenIo, ScreenFrame, ScreenKey};

const ID_ACCEPT: u32 = 1;
const ID_CANCEL: u32 = 2;
const ID_REVOKE_BASE: u32 = 100;
const ID_PREVIOUS: u32 = 200;
const ID_NEXT: u32 = 201;
const ID_REVOKE_ALL: u32 = 202;
const ID_CLEAR_CACHE: u32 = 203;
const ID_RESET_TRUST: u32 = 204;
const ID_BACK: u32 = 205;
const GRANTS_PER_PAGE: usize = 3;
const GRANT_ROW_START_Y: i32 = 72;
const GRANT_ROW_HEIGHT: i32 = 76;
const GRANT_BUTTON_OFFSET_Y: i32 = 42;
const CONTENT_TEXT_X: i32 = 28;
const CONTENT_TEXT_WIDTH: i32 = 584;
const CONSENT_BODY_START_Y: i32 = 58;
const CONSENT_STATUS_BOUNDARY_Y: i32 = 390;
#[cfg(test)]
const CONSENT_PRE_HASH_MAX_LINES: usize = 8;
#[cfg(test)]
const CONSENT_HASH_BLOCK_LINES: usize = 6;
const CONSENT_WARNING_MAX_LINES: usize = 2;
#[cfg(test)]
const CONSENT_TOTAL_MAX_LINES: usize =
    CONSENT_PRE_HASH_MAX_LINES + CONSENT_HASH_BLOCK_LINES + CONSENT_WARNING_MAX_LINES;

const SCREEN: &str = "Spellforge content screen";

fn localized_text(application_context: &ApplicationContext, key: PortTextKey) -> &'static str {
    require(application_context.port_text(key), SCREEN)
}

fn localized_format(
    application_context: &ApplicationContext,
    key: PortTextKey,
    arguments: &[(&str, &str)],
) -> String {
    require(application_context.format_port_text(key, arguments), SCREEN)
}

fn bounded_display_lines(font: &Font, text: &str, max_width: i32, max_lines: usize) -> Vec<String> {
    let wrapped = wrap_text_for_box_font(font, text, max_width, max_lines);
    let last_index = wrapped.lines.len().checked_sub(1);
    wrapped
        .lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            elide_to_width_by(
                &line.text,
                max_width,
                Some(index) == last_index && !wrapped.remaining.is_empty(),
                |candidate| font.text_width(candidate),
            )
        })
        .collect()
}

fn render_bounded_text_with_width(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    y: &mut i32,
    max_width: i32,
    max_lines: usize,
) {
    for line in bounded_display_lines(font, text, max_width, max_lines) {
        render_text_virt(renderer, font, transform, &line, CONTENT_TEXT_X, *y);
        *y += font.height() as i32 + 2;
    }
}

fn render_bounded_text(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    y: &mut i32,
    max_lines: usize,
) {
    render_bounded_text_with_width(
        renderer,
        font,
        transform,
        text,
        y,
        CONTENT_TEXT_WIDTH,
        max_lines,
    );
}

fn render_complete_bounded_text(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    y: &mut i32,
    max_lines: usize,
) {
    let wrapped = wrap_text_for_box_font(font, text, CONTENT_TEXT_WIDTH, max_lines);
    assert!(
        wrapped.remaining.is_empty(),
        "authenticated identity did not fit its fixed full-display budget"
    );
    for line in wrapped.lines {
        assert!(
            font.text_width(&line.text) <= CONTENT_TEXT_WIDTH,
            "authenticated identity contains a grapheme wider than its display budget"
        );
        render_text_virt(renderer, font, transform, &line.text, CONTENT_TEXT_X, *y);
        *y += font.height() as i32 + 2;
    }
}

fn render_exact_hash(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    hash: &str,
    y: &mut i32,
) {
    render_exact_hash_with_width(renderer, font, transform, hash, y, CONTENT_TEXT_WIDTH);
}

fn render_exact_hash_with_width(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    hash: &str,
    y: &mut i32,
    max_width: i32,
) {
    for half in exact_hash_lines(hash) {
        assert!(
            font.text_width(half) <= max_width,
            "localized content font cannot display an invariant SHA-256 half"
        );
        render_text_virt(renderer, font, transform, half, CONTENT_TEXT_X, *y);
        *y += font.height() as i32 + 2;
    }
}

fn exact_hash_lines(hash: &str) -> [&str; 2] {
    assert_eq!(hash.len(), 64, "SHA-256 must be 64 hexadecimal characters");
    assert!(
        hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "SHA-256 display contains a non-hexadecimal character"
    );
    [&hash[..32], &hash[32..]]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellforgeConsentOutcome {
    AlreadyTrusted,
    Approved,
    Cancelled,
}

/// Confirm that the local host intends to redistribute this complete mod.
/// When catalog metadata has no licence, the returned text is an explicit
/// attestation bound to the authenticated host public key and source URL.
pub async fn show_host_distribution_attestation(
    application_context: &ApplicationContext,
    io: &mut ModalScreenIo<'_, '_>,
    launch: &crate::main_menu::custom_missions::CustomMissionLaunch,
    host_endpoint_id: &str,
) -> Result<Option<String>, String> {
    for (label, value) in [
        ("mod title", launch.mod_title.as_str()),
        ("claimed author", launch.claimed_author.as_str()),
        ("version", launch.version.as_str()),
        ("source URL", launch.source_url.as_str()),
    ] {
        robin_engine::multiplayer::validate_safe_display_text(label, value, 4 * 1024)?;
    }
    robin_engine::multiplayer::validate_safe_display_text(
        "authenticated host key",
        host_endpoint_id,
        robin_engine::multiplayer::DistributedModOffer::AUTHENTICATED_HOST_ID_BYTE_LIMIT,
    )?;
    if !launch.license.is_empty() {
        robin_engine::multiplayer::validate_safe_display_text(
            "licence",
            &launch.license,
            4 * 1024,
        )?;
    }
    let mut state = HostDistributionAttestationState::new(application_context, io);
    Ok(widget_bridge::run_modal(io, |io| {
        state.tick(application_context, io, launch, host_endpoint_id)
    })
    .await)
}

// Owns live input/capture state, not a restorable screen document.
struct HostDistributionAttestationState {
    frame: FrameWnd,
    input: ModalInputState,
}

impl HostDistributionAttestationState {
    fn new(application_context: &ApplicationContext, io: &ModalScreenIo<'_, '_>) -> Self {
        let resources = io.resources;
        let mut frame = FrameWnd::interactive();
        widget_bridge::add_bottom_right_pair(
            &mut frame,
            resources,
            (
                ID_ACCEPT,
                localized_text(application_context, PortTextKey::SpellforgeHostAndAttest),
            ),
            (ID_CANCEL, &resources.menu_text.get(MT_BTN_BACK)),
        );
        let input = ModalInputState::for_screen(io.window, io.renderer);
        Self { frame, input }
    }

    /// One frame; `Some(None)` cancels and `Some(Some(text))` attests.
    fn tick(
        &mut self,
        application_context: &ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
        launch: &crate::main_menu::custom_missions::CustomMissionLaunch,
        host_endpoint_id: &str,
    ) -> Option<Option<String>> {
        let screen = ScreenFrame::begin(io, &mut self.input);
        if screen
            .keys()
            .any(|key| matches!(key, ScreenKey::Quit | ScreenKey::Cancel))
        {
            return Some(None);
        }
        let (_, activated) = ScreenFrame::dispatch(&mut self.input, &mut self.frame);
        if let Some(id) = activated {
            match id {
                ID_CANCEL => return Some(None),
                ID_ACCEPT => {
                    return Some(Some(if launch.license.trim().is_empty() {
                        localized_format(
                            application_context,
                            PortTextKey::SpellforgeHostAttestation,
                            &[
                                ("host", host_endpoint_id),
                                ("title", &launch.mod_title),
                                ("source", &launch.source_url),
                            ],
                        )
                    } else {
                        launch.license.clone()
                    }));
                }
                _ => {}
            }
        }

        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let transform = screen.transform;
        screen.begin_draw(renderer);
        if let Some(background) = resources.menu_bg[2] {
            draw_screen_background(renderer, &background);
        }
        if let Some(font) = resources.title_font_any() {
            render_text_virt(
                renderer,
                font,
                transform,
                localized_text(
                    application_context,
                    PortTextKey::SpellforgeDistributeCompleteMod,
                ),
                20,
                16,
            );
        }
        if let Some(font) = resources.label_font_any() {
            let licence = if launch.license.trim().is_empty() {
                localized_text(
                    application_context,
                    PortTextKey::SpellforgeMissingLicenseWarning,
                )
            } else {
                launch.license.as_str()
            };
            let mut y = 62;
            for (key, value, lines) in [
                (
                    PortTextKey::SpellforgeMissionField,
                    launch.mod_title.as_str(),
                    2,
                ),
                (
                    PortTextKey::SpellforgeClaimedAuthorField,
                    launch.claimed_author.as_str(),
                    1,
                ),
                (
                    PortTextKey::SpellforgeVersionField,
                    launch.version.as_str(),
                    1,
                ),
                (
                    PortTextKey::SpellforgeSourceField,
                    launch.source_url.as_str(),
                    2,
                ),
                (PortTextKey::SpellforgeLicenseField, licence, 2),
            ] {
                let text = localized_format(application_context, key, &[("value", value)]);
                render_bounded_text(renderer, font, transform, &text, &mut y, lines);
            }
            let host_identity = localized_format(
                application_context,
                PortTextKey::SpellforgeAuthenticatedHostKeyField,
                &[("value", host_endpoint_id)],
            );
            render_complete_bounded_text(renderer, font, transform, &host_identity, &mut y, 2);
            for key in [
                PortTextKey::SpellforgeCompleteModDeliveryWarning,
                PortTextKey::SpellforgeRedistributionWarning,
            ] {
                render_bounded_text(
                    renderer,
                    font,
                    transform,
                    localized_text(application_context, key),
                    &mut y,
                    2,
                );
            }
        }
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &self.frame);
        screen.finish(io, &self.input);
        None
    }
}

pub async fn show_spellforge_consent(
    application_context: &ApplicationContext,
    io: &mut ModalScreenIo<'_, '_>,
    key: SpellforgeTrustKey,
    metadata: SpellforgeTrustMetadata,
) -> Result<SpellforgeConsentOutcome, String> {
    if key.package_sha256.is_some()
        && !application_context
            .with_active_profile(|profile| profile.gameplay_config.enable_spellforge_missions)?
    {
        return Err(
            localized_text(application_context, PortTextKey::SpellforgeDisabled).to_owned(),
        );
    }
    if application_context.is_spellforge_content_trusted(key)? {
        return Ok(SpellforgeConsentOutcome::AlreadyTrusted);
    }

    let mut state = SpellforgeConsentState::new(application_context, io, key, metadata);
    Ok(widget_bridge::run_modal(io, |io| state.tick(application_context, io)).await)
}

// Trust metadata is ordinary data; pending keyboard/capture ownership is not.
struct SpellforgeConsentState {
    frame: FrameWnd,
    input: ModalInputState,
    status: String,
    key: SpellforgeTrustKey,
    metadata: SpellforgeTrustMetadata,
}

impl SpellforgeConsentState {
    fn new(
        application_context: &ApplicationContext,
        io: &ModalScreenIo<'_, '_>,
        key: SpellforgeTrustKey,
        metadata: SpellforgeTrustMetadata,
    ) -> Self {
        let resources = io.resources;
        let mut frame = FrameWnd::interactive();
        widget_bridge::add_bottom_right_pair(
            &mut frame,
            resources,
            (
                ID_ACCEPT,
                localized_text(application_context, PortTextKey::SpellforgeTrustExactMod),
            ),
            (ID_CANCEL, &resources.menu_text.get(MT_BTN_BACK)),
        );
        let input = ModalInputState::for_screen(io.window, io.renderer);
        let status = String::new();

        Self {
            frame,
            input,
            status,
            key,
            metadata,
        }
    }

    fn tick(
        &mut self,
        application_context: &ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
    ) -> Option<SpellforgeConsentOutcome> {
        let screen = ScreenFrame::begin(io, &mut self.input);
        // Only window close and Escape are handled: deliberately no
        // Return/KpEnter approval accelerator.
        if screen
            .keys()
            .any(|key| matches!(key, ScreenKey::Quit | ScreenKey::Cancel))
        {
            return Some(SpellforgeConsentOutcome::Cancelled);
        }
        let (_, activated) = ScreenFrame::dispatch(&mut self.input, &mut self.frame);
        if let Some(id) = activated {
            match id {
                ID_CANCEL => {
                    return Some(SpellforgeConsentOutcome::Cancelled);
                }
                ID_ACCEPT => {
                    let result = current_unix_seconds().and_then(|approved_unix_seconds| {
                        application_context.grant_spellforge_content_trust(
                            self.key,
                            self.metadata.clone(),
                            approved_unix_seconds,
                        )
                    });
                    match result {
                        Ok(()) => {
                            return Some(SpellforgeConsentOutcome::Approved);
                        }
                        Err(error) => {
                            self.status = localized_format(
                                application_context,
                                PortTextKey::SpellforgeApprovalNotPersisted,
                                &[("error", &error)],
                            )
                        }
                    }
                }
                _ => {}
            }
        }

        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let transform = screen.transform;
        screen.begin_draw(renderer);
        if let Some(background) = resources.menu_bg[2] {
            draw_screen_background(renderer, &background);
        }
        if let Some(font) = resources.title_font_any() {
            render_text_virt(
                renderer,
                font,
                transform,
                localized_text(application_context, PortTextKey::SpellforgeTrustHostContent),
                20,
                16,
            );
        }
        if let Some(font) = resources.label_font_any() {
            let package_hash = self
                .key
                .package_sha256
                .map(|hash| robin_engine::spellforge::hex_hash(&hash))
                .unwrap_or_else(|| {
                    localized_text(application_context, PortTextKey::SpellforgeVanillaPackage)
                        .to_owned()
                });
            let full_hash = robin_engine::spellforge::hex_hash(&self.key.full_mod_sha256);
            let mut y = CONSENT_BODY_START_Y;
            for (field_key, value, lines) in [
                (
                    PortTextKey::SpellforgeMissionField,
                    self.metadata.title.as_str(),
                    1,
                ),
                (
                    PortTextKey::SpellforgeClaimedAuthorField,
                    self.metadata.claimed_author.as_str(),
                    1,
                ),
                (
                    PortTextKey::SpellforgeVersionField,
                    self.metadata.version.as_str(),
                    1,
                ),
                (
                    PortTextKey::SpellforgeLicensePermissionField,
                    self.metadata.license.as_str(),
                    1,
                ),
                (
                    PortTextKey::SpellforgeSourceField,
                    self.metadata.source_url.as_str(),
                    1,
                ),
            ] {
                let text = localized_format(application_context, field_key, &[("value", value)]);
                render_bounded_text(renderer, font, transform, &text, &mut y, lines);
            }
            let distributor_identity = localized_format(
                application_context,
                PortTextKey::SpellforgeDistributorKeyField,
                &[("value", &self.metadata.host_endpoint_id)],
            );
            render_complete_bounded_text(
                renderer,
                font,
                transform,
                &distributor_identity,
                &mut y,
                2,
            );
            let bytes = self.metadata.compressed_bytes.to_string();
            let download = localized_format(
                application_context,
                PortTextKey::SpellforgeDownloadBytesField,
                &[("bytes", &bytes)],
            );
            render_bounded_text(renderer, font, transform, &download, &mut y, 1);

            render_bounded_text(
                renderer,
                font,
                transform,
                localized_text(application_context, PortTextKey::SpellforgeFullModHash),
                &mut y,
                1,
            );
            render_exact_hash(renderer, font, transform, &full_hash, &mut y);
            render_bounded_text(
                renderer,
                font,
                transform,
                localized_text(application_context, PortTextKey::SpellforgeLuaPackageHash),
                &mut y,
                1,
            );
            if self.key.package_sha256.is_some() {
                render_exact_hash(renderer, font, transform, &package_hash, &mut y);
            } else {
                render_bounded_text(renderer, font, transform, &package_hash, &mut y, 1);
            }
            let warning_key = if self.key.package_sha256.is_some() {
                PortTextKey::SpellforgeExecutableWarning
            } else {
                PortTextKey::SpellforgeNonExecutableWarning
            };
            render_bounded_text(
                renderer,
                font,
                transform,
                localized_text(application_context, warning_key),
                &mut y,
                CONSENT_WARNING_MAX_LINES,
            );
            assert!(
                y <= CONSENT_STATUS_BOUNDARY_Y,
                "consent content exceeded its fixed body budget"
            );
            if !self.status.is_empty() {
                let mut status_y = 390;
                render_bounded_text(renderer, font, transform, &self.status, &mut status_y, 1);
            }
        }
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &self.frame);
        screen.finish(io, &self.input);
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpellforgeContentSettingsOutcome {
    Pending,
    Closed,
    ExitRequested,
}

/// One-frame state for the trust/cache manager. The standalone menu drives it
/// in a compatibility loop; the in-mission Options task drives exactly one
/// tick per mission frame so multiplayer networking and simulation continue.
pub(crate) struct SpellforgeContentSettingsState {
    page: usize,
    status: String,
    input: ModalInputState,
    transform: MenuTransform,
    frame: FrameWnd,
    grants: Vec<SpellforgeTrustGrant>,
    settings_text_width: i32,
    /// Last observed application state, not the operation's completion receiver.
    cache_clear: Result<CacheClearStatus, String>,
}

impl SpellforgeContentSettingsState {
    pub(crate) fn new(
        application_context: &ApplicationContext,
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
    ) -> Self {
        let transform = MenuTransform::for_renderer(renderer);
        let input = ModalInputState::from_window(event_pump, transform);
        let mut state = Self {
            page: 0,
            status: String::new(),
            input,
            transform,
            frame: FrameWnd::default(),
            grants: Vec::new(),
            settings_text_width: 1,
            cache_clear: Ok(CacheClearStatus::Idle),
        };
        state.poll_cache_clear(application_context, resources);
        state.reload(application_context, resources);
        state
    }

    fn page_count(&self) -> usize {
        self.grants.len().div_ceil(GRANTS_PER_PAGE).max(1)
    }

    fn visible_range(&self) -> std::ops::Range<usize> {
        let start = self.page * GRANTS_PER_PAGE;
        start..self.grants.len().min(start + GRANTS_PER_PAGE)
    }

    fn reload(
        &mut self,
        application_context: &ApplicationContext,
        resources: &IngameMenuResources,
    ) {
        self.grants = match application_context.active_spellforge_trust_grants() {
            Ok(grants) => grants,
            Err(error) => {
                self.status = error;
                Vec::new()
            }
        };
        self.page = self.page.min(self.page_count() - 1);
        let shown = &self.grants[self.visible_range()];
        let (field_w, field_h) = resources.input_field_dimensions();
        let field_w = field_w.min(592);
        let field_h = field_h.min(34);
        let mut frame = FrameWnd::interactive();
        let revoke_font = resources
            .menu_button_font_any(true)
            .unwrap_or_else(|| panic!("Spellforge content settings require a menu-button font"));
        for (index, grant) in shown.iter().enumerate() {
            let hash = robin_engine::spellforge::hex_hash(&grant.key.full_mod_sha256);
            let raw_label = localized_format(
                application_context,
                PortTextKey::SpellforgeRevokeGrant,
                &[("hash", &hash[..12]), ("title", &grant.metadata.title)],
            );
            let label = elide_to_width_by(&raw_label, field_w - 16, false, |candidate| {
                revoke_font.text_width(candidate)
            });
            frame.add_widget_absolute(widget_bridge::make_button(
                ID_REVOKE_BASE + index as u32,
                &label,
                24,
                GRANT_ROW_START_Y + index as i32 * GRANT_ROW_HEIGHT + GRANT_BUTTON_OFFSET_Y,
                field_w,
                field_h,
            ));
        }

        let (button_w, button_h) = resources.button_dimensions();
        let back_label = resources.menu_text.get(MT_BTN_BACK);
        let labels = [
            (
                localized_text(application_context, PortTextKey::SpellforgePrevious),
                self.page > 0,
            ),
            (
                localized_text(application_context, PortTextKey::SpellforgeNext),
                self.page + 1 < self.page_count(),
            ),
            (
                localized_text(application_context, PortTextKey::SpellforgeRevokeAll),
                !self.grants.is_empty(),
            ),
            (
                localized_text(application_context, PortTextKey::SpellforgeClearCache),
                matches!(&self.cache_clear, Ok(status) if !status.is_pending()),
            ),
            (
                localized_text(application_context, PortTextKey::SpellforgeResetTrust),
                true,
            ),
            (back_label.as_str(), true),
        ];
        let bottom = align_bottom_right(&labels, button_w, button_h);
        self.settings_text_width = (bottom[0].x - CONTENT_TEXT_X - 8).max(1);
        for (index, id) in [
            ID_PREVIOUS,
            ID_NEXT,
            ID_REVOKE_ALL,
            ID_CLEAR_CACHE,
            ID_RESET_TRUST,
            ID_BACK,
        ]
        .into_iter()
        .enumerate()
        {
            let button = &bottom[index];
            let display_label = crate::ingame_menu::gameplay::fit_button_label(
                resources,
                &button.label,
                button.enabled,
                button.w,
            );
            frame.add_widget_absolute(widget_bridge::make_button_enabled(
                id,
                &display_label,
                button.enabled,
                button.x,
                button.y,
                button.w,
                button.h,
            ));
        }
        self.frame = frame;
    }

    pub(crate) fn tick(
        &mut self,
        application_context: &ApplicationContext,
        io: &mut widget_bridge::ModalScreenIo<'_, '_>,
    ) -> SpellforgeContentSettingsOutcome {
        let resources = io.resources;
        self.poll_cache_clear(application_context, resources);
        let screen = ScreenFrame::begin(io, &mut self.input);
        self.transform = screen.transform;
        for key in screen.keys() {
            match key {
                ScreenKey::Quit => return SpellforgeContentSettingsOutcome::ExitRequested,
                ScreenKey::Cancel => return SpellforgeContentSettingsOutcome::Closed,
                _ => {}
            }
        }

        let (_, activated) = ScreenFrame::dispatch(&mut self.input, &mut self.frame);
        if let Some(id) = activated {
            match id {
                ID_BACK => return SpellforgeContentSettingsOutcome::Closed,
                ID_PREVIOUS if self.page > 0 => self.page -= 1,
                ID_NEXT if self.page + 1 < self.page_count() => self.page += 1,
                ID_REVOKE_ALL => {
                    self.status = match application_context.revoke_all_spellforge_content_trust() {
                        Ok(count) => localized_format(
                            application_context,
                            PortTextKey::SpellforgeRevokedApprovals,
                            &[("count", &count.to_string())],
                        ),
                        Err(error) => localized_format(
                            application_context,
                            PortTextKey::SpellforgeRevocationFailed,
                            &[("error", &error)],
                        ),
                    };
                }
                ID_CLEAR_CACHE => {
                    self.begin_cache_clear(application_context, resources);
                }
                ID_RESET_TRUST => {
                    self.status = match application_context.reset_spellforge_trust_store() {
                        Ok(()) => localized_text(
                            application_context,
                            PortTextKey::SpellforgeTrustResetSuccess,
                        )
                        .to_owned(),
                        Err(error) => localized_format(
                            application_context,
                            PortTextKey::SpellforgeTrustResetFailed,
                            &[("error", &error)],
                        ),
                    };
                }
                id if (ID_REVOKE_BASE..ID_REVOKE_BASE + GRANTS_PER_PAGE as u32).contains(&id) => {
                    let grant_index = self.page * GRANTS_PER_PAGE + (id - ID_REVOKE_BASE) as usize;
                    if let Some(grant) = self.grants.get(grant_index).cloned() {
                        self.status =
                            match application_context.revoke_spellforge_content_trust(grant.key) {
                                Ok(true) => localized_format(
                                    application_context,
                                    PortTextKey::SpellforgeRevokedApproval,
                                    &[("title", &grant.metadata.title)],
                                ),
                                Ok(false) => localized_text(
                                    application_context,
                                    PortTextKey::SpellforgeApprovalAlreadyAbsent,
                                )
                                .to_owned(),
                                Err(error) => localized_format(
                                    application_context,
                                    PortTextKey::SpellforgeRevocationFailed,
                                    &[("error", &error)],
                                ),
                            };
                    }
                }
                _ => {}
            }
            self.reload(application_context, resources);
        }

        self.render(application_context, io, &screen);
        SpellforgeContentSettingsOutcome::Pending
    }

    fn poll_cache_clear(
        &mut self,
        application_context: &ApplicationContext,
        resources: &IngameMenuResources,
    ) {
        self.observe_cache_clear(
            application_context.cache_clear_status(),
            application_context,
            resources,
        );
    }

    fn observe_cache_clear(
        &mut self,
        observed: Result<CacheClearStatus, String>,
        application_context: &ApplicationContext,
        resources: &IngameMenuResources,
    ) {
        if self.cache_clear == observed {
            return;
        }
        self.cache_clear = observed;
        let completed = match &self.cache_clear {
            Ok(CacheClearStatus::Completed { result, .. }) => Some(result.clone()),
            Err(error) => Some(Err(error.clone())),
            Ok(CacheClearStatus::Idle | CacheClearStatus::Pending { .. }) => None,
        };
        self.status = match completed {
            None => String::new(),
            Some(Ok(count)) => localized_format(
                application_context,
                PortTextKey::SpellforgeRemovedCachedMods,
                &[("count", &count.to_string())],
            ),
            Some(Err(error)) => localized_format(
                application_context,
                PortTextKey::SpellforgeCacheClearFailed,
                &[("error", &error)],
            ),
        };
        self.reload(application_context, resources);
    }

    fn begin_cache_clear(
        &mut self,
        application_context: &ApplicationContext,
        resources: &IngameMenuResources,
    ) {
        self.observe_cache_clear(
            application_context.begin_distributed_mod_cache_clear(),
            application_context,
            resources,
        );
    }

    /// Draw the manager page, then the cursor, and present.
    fn render(
        &self,
        application_context: &ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
        screen: &ScreenFrame,
    ) {
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        screen.begin_draw(renderer);
        if let Some(background) = resources.menu_bg[2] {
            draw_screen_background(renderer, &background);
        }
        if let Some(font) = resources.title_font_any() {
            render_text_virt(
                renderer,
                font,
                self.transform,
                localized_text(application_context, PortTextKey::SpellforgeContentTitle),
                20,
                18,
            );
        }
        if let Some(font) = resources.label_font_any() {
            let current_page = (self.page + 1).to_string();
            let total_pages = self.page_count().to_string();
            let page_label = localized_format(
                application_context,
                PortTextKey::SpellforgeApprovalsPage,
                &[("page", &current_page), ("pages", &total_pages)],
            );
            render_text_virt(renderer, font, self.transform, &page_label, 24, 54);
            for (index, grant) in self.grants[self.visible_range()].iter().enumerate() {
                let hash = robin_engine::spellforge::hex_hash(&grant.key.full_mod_sha256);
                let mut hash_y = GRANT_ROW_START_Y + index as i32 * GRANT_ROW_HEIGHT;
                render_exact_hash_with_width(
                    renderer,
                    font,
                    self.transform,
                    &hash,
                    &mut hash_y,
                    self.settings_text_width,
                );
            }
            if self.grants.is_empty() && self.status.is_empty() {
                render_text_virt(
                    renderer,
                    font,
                    self.transform,
                    localized_text(application_context, PortTextKey::SpellforgeNoApprovals),
                    24,
                    84,
                );
            }
            if !self.status.is_empty() {
                let mut status_y = 390;
                render_bounded_text_with_width(
                    renderer,
                    font,
                    self.transform,
                    &self.status,
                    &mut status_y,
                    self.settings_text_width,
                    1,
                );
            }
        }
        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);
        screen.finish(io, &self.input);
    }
}

pub async fn show_spellforge_content_settings(
    application_context: &ApplicationContext,
    io: &mut ModalScreenIo<'_, '_>,
) {
    let mut state = SpellforgeContentSettingsState::new(
        application_context,
        io.window,
        io.renderer,
        io.resources,
    );
    widget_bridge::run_modal(io, |io| match state.tick(application_context, io) {
        SpellforgeContentSettingsOutcome::Pending => None,
        SpellforgeContentSettingsOutcome::Closed
        | SpellforgeContentSettingsOutcome::ExitRequested => Some(()),
    })
    .await
}

#[cfg(not(target_arch = "wasm32"))]
fn current_unix_seconds() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("system clock precedes the Unix epoch: {error}"))
}

#[cfg(target_arch = "wasm32")]
fn current_unix_seconds() -> Result<u64, String> {
    unix_seconds_from_millis(js_sys::Date::now())
}

#[cfg(any(target_arch = "wasm32", test))]
fn unix_seconds_from_millis(unix_millis: f64) -> Result<u64, String> {
    if !unix_millis.is_finite() {
        return Err("browser clock returned a non-finite timestamp".to_owned());
    }
    if unix_millis < 0.0 {
        return Err("browser clock precedes the Unix epoch".to_owned());
    }
    let unix_seconds = (unix_millis / 1_000.0).floor();
    // `u64::MAX as f64` rounds to 2^64, which is the exact exclusive upper
    // bound needed before Rust's saturating float-to-int conversion.
    if unix_seconds >= u64::MAX as f64 {
        return Err("browser clock timestamp exceeds the u64 Unix range".to_owned());
    }
    Ok(unix_seconds as u64)
}

#[cfg(test)]
mod tests {
    use super::{
        CONSENT_BODY_START_Y, CONSENT_HASH_BLOCK_LINES, CONSENT_PRE_HASH_MAX_LINES,
        CONSENT_STATUS_BOUNDARY_Y, CONSENT_TOTAL_MAX_LINES, elide_to_width_by, exact_hash_lines,
        unix_seconds_from_millis,
    };

    fn character_width(text: &str) -> i32 {
        text.chars().count() as i32
    }

    #[test]
    fn bounded_elision_preserves_unicode_boundaries_and_marks_overflow() {
        assert_eq!(
            elide_to_width_by("abcdef", 4, false, character_width),
            "abc…"
        );
        assert_eq!(
            elide_to_width_by("猫犬鳥", 2, false, character_width),
            "猫…"
        );
        assert_eq!(elide_to_width_by("word", 5, true, character_width), "word…");
        assert_eq!(
            elide_to_width_by("e\u{301}clair", 3, false, character_width),
            "e\u{301}…"
        );
    }

    #[test]
    fn bounded_elision_handles_a_huge_unbroken_word() {
        let huge = "x".repeat(32 * 1024);
        let fitted = elide_to_width_by(&huge, 8, false, character_width);
        assert_eq!(fitted, "xxxxxxx…");
        assert_eq!(character_width(&fitted), 8);
    }

    #[test]
    fn security_hash_lines_are_complete_and_invariant() {
        let hash = "0123456789abcdef0123456789abcdefFEDCBA9876543210FEDCBA9876543210";
        assert_eq!(
            exact_hash_lines(hash),
            [
                "0123456789abcdef0123456789abcdef",
                "FEDCBA9876543210FEDCBA9876543210",
            ]
        );
    }

    #[test]
    #[should_panic(expected = "non-hexadecimal")]
    fn security_hash_lines_reject_non_hex_text() {
        exact_hash_lines("g123456789abcdef0123456789abcdefFEDCBA9876543210FEDCBA9876543210");
    }

    #[test]
    fn consent_line_budget_keeps_complete_hashes_above_status_boundary() {
        // The largest supported label-font cell in this fixed 640x480 modal is
        // 18 px high. Metadata, both invariant two-line hashes and their
        // labels, and the warning all remain above the status/button boundary.
        const MAX_SUPPORTED_LABEL_FONT_HEIGHT: i32 = 18;
        let bottom = CONSENT_BODY_START_Y
            + CONSENT_TOTAL_MAX_LINES as i32 * (MAX_SUPPORTED_LABEL_FONT_HEIGHT + 2);
        assert_eq!(CONSENT_PRE_HASH_MAX_LINES, 8);
        assert_eq!(CONSENT_HASH_BLOCK_LINES, 6);
        assert!(bottom <= CONSENT_STATUS_BOUNDARY_Y);
    }

    #[test]
    fn consent_has_no_enter_accelerator_by_construction() {
        // The dedicated consent event loop handles only Escape. Keep this
        // explicit regression marker next to the security-sensitive UI.
        let source = include_str!("spellforge_content.rs");
        let consent = source
            .split("pub async fn show_spellforge_consent")
            .nth(1)
            .unwrap();
        let consent = consent
            .split("pub async fn show_spellforge_content_settings")
            .next()
            .unwrap();
        assert!(!consent.contains("Keycode::Return"));
        assert!(!consent.contains("Keycode::KpEnter"));
        // Screens classify Return/KpEnter as `ScreenKey::Confirm`.
        assert!(!consent.contains("ScreenKey::Confirm"));
    }

    #[test]
    fn browser_clock_conversion_rejects_invalid_and_out_of_range_values() {
        assert!(unix_seconds_from_millis(-1.0).is_err());
        assert!(unix_seconds_from_millis(f64::NAN).is_err());
        assert!(unix_seconds_from_millis(f64::INFINITY).is_err());
        assert!(unix_seconds_from_millis(f64::NEG_INFINITY).is_err());
        assert!(unix_seconds_from_millis((u64::MAX as f64) * 1_000.0).is_err());
    }

    #[test]
    fn browser_clock_conversion_uses_complete_nonnegative_seconds() {
        assert_eq!(unix_seconds_from_millis(0.0), Ok(0));
        assert_eq!(unix_seconds_from_millis(1_999.9), Ok(1));
        assert_eq!(
            unix_seconds_from_millis(1_700_000_000_123.0),
            Ok(1_700_000_000)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_clock_returns_a_real_post_epoch_timestamp() {
        assert!(super::current_unix_seconds().expect("native system clock") > 0);
    }
}
