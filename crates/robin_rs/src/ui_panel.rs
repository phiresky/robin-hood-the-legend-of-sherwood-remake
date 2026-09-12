//! Bottom UI panel rendering — portraits, minimap frame, and action buttons.
//!
//! The panel is composited from multiple overlapping widget bitmaps; this
//! module renders character portraits loaded from resource files with
//! selection highlighting and health bars.
//!
//! Layout reference:
//! - 5 portrait slots across the bottom, 32px margin on each side
//! - Each portrait: 112px wide, stacked vertically from bottom:
//!   border(3) + bottom_scroll(23) + actions(35) + visage(50) + top_scroll(23)
//! - Minimap button at top-right of panel area
//!
//! The `PANNEL_HEIGHT` used by the engine camera (130px in engine.rs) represents
//! the full UI chrome height including the panel and its transition zone.

use crate::host::HostFrontend;
use robin_assets::picture::Picture;
use robin_engine::character_kind::CharacterKind;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::{ScreenBBox, ScreenPoint};
use robin_engine::engine::PresentationView;
use robin_engine::player_command::PlayerId;
use robin_engine::profiles as engine_profiles;
use robin_engine::sprite::BBox;
use robin_engine::tactical_control::{
    CombatStance, TacticalDuty, TacticalFormation, TacticalPinnedGroup,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;

use crate::gfx_types::{BlendMode, Rect};
use crate::ingame_menu::layout;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, GpuImage, OwnedSurface, Renderer, SurfaceHandle};
use crate::widget::requirements::{RequirementSlot, RequirementStatus};
use robin_assets::resource_manager::{ResourceId, ResourceManager};
use robin_engine::element::{Entity, EntityId};
use robin_engine::minimap::HitMask;
use robin_engine::profiles::Action;
use robin_engine::titbit::SpriteRow;

// ─── Layout constants ─────────────────────────────────────────────

/// Horizontal margin on each side of the portrait area (pixels).
const MARGIN: u16 = 32;

/// Width of a single portrait element (pixels).
const ELEMENT_WIDTH: u16 = 112;
const ALLIED_ACTION_ICON_WIDTH: u16 = 34;
const ALLIED_ACTION_ICON_HEIGHT: u16 = 32;
const ALLIED_PIN_ICON_SIZE: u16 = 27;
const ALLIED_VISAGE_COUNT: usize = 6;
// Scale the old 18px icon around its center, then move that center 9px
// right/up. The resulting pin hangs slightly over the scroll's top-right.
const ALLIED_PIN_LEFT: u16 = 86;
const ALLIED_PIN_RISE: u16 = 10;

/// Border gap at the very bottom of the screen.
const BORDURE: u16 = 3;

// Vertical heights of portrait sub-elements (open state).
const BOTTOM_SCROLL_HEIGHT: u16 = 23;
const ACTION_HEIGHT: u16 = 35;
const VISAGE_HEIGHT: u16 = 50;
const TOP_SCROLL_HEIGHT: u16 = 23;

/// Total height of a fully open portrait widget.
const PORTRAIT_TOTAL_HEIGHT: u16 =
    BORDURE + BOTTOM_SCROLL_HEIGHT + ACTION_HEIGHT + VISAGE_HEIGHT + TOP_SCROLL_HEIGHT;

// Vertical positions measured from the bottom of the screen.
// Open state (selected PCs) — full layout with action buttons.
const POSITION_BOTTOM_SCROLL: u16 = BORDURE + BOTTOM_SCROLL_HEIGHT;
const POSITION_ACTION: u16 = POSITION_BOTTOM_SCROLL + ACTION_HEIGHT;
const POSITION_VISAGE: u16 = POSITION_ACTION + VISAGE_HEIGHT;
const POSITION_TOP_SCROLL: u16 = POSITION_VISAGE + TOP_SCROLL_HEIGHT;

// Closed state (non-selected PCs) — no action buttons, scrolls compressed.
const CLOSE_POSITION_BOTTOM_SCROLL: u16 = POSITION_BOTTOM_SCROLL;
const CLOSE_POSITION_VISAGE: u16 = CLOSE_POSITION_BOTTOM_SCROLL + VISAGE_HEIGHT;
const CLOSE_POSITION_TOP_SCROLL: u16 = CLOSE_POSITION_VISAGE + TOP_SCROLL_HEIGHT;

// Action button widths (3-button mode).
const ACTION1_WIDTH: u16 = 40;
const ACTION2_WIDTH: u16 = 32;
const ACTION3_WIDTH: u16 = 40;

// Action button widths (2-button mode — peasants whose third action is NoAction).
const ACTIONA_WIDTH: u16 = 56;
const ACTIONB_WIDTH: u16 = 56;

// Quick-action slot icon strip — each icon is 33 px wide, placed 20 px above
// the upper scroll top.
const QA_ICON_WIDTH: u16 = 33;
/// Height of the QA icon strip above the upper scroll.
const QA_ICON_HEIGHT: u16 = 20;
/// Cast of [`robin_engine::macro_store::NUMBER_OF_QA_MEMORY`] for the draw loop.
const NUMBER_OF_QA_MEMORY_U16: u16 = robin_engine::macro_store::NUMBER_OF_QA_MEMORY as u16;

// ─── Colors (RGB565) ───────────────────────────────────────────────

/// Fallback fill for the visage slot when the portrait sprite fails to load.
fn color_visage_fill() -> u16 {
    Renderer::create_color_16(50, 40, 30)
}

/// Fallback fill for an action button slot when its icon sprite is missing.
fn color_action_fill() -> u16 {
    Renderer::create_color_16(40, 50, 35)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionButtonVisual {
    Disabled,
    Normal,
    Hover,
    Pressed,
    HoverPressed,
}

/// Profile-specific visage art for player-controlled soldiers. The five named
/// variants are cropped from Original's dialogue portraits; `Generic` keeps
/// the established helmet portrait for every other soldier profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlliedVisageKind {
    Generic,
    Guisbourne,
    Longchamp,
    PrinceJohn,
    Scathlock,
    Sheriff,
}

impl AlliedVisageKind {
    const VARIANTS: [Self; ALLIED_VISAGE_COUNT] = [
        Self::Generic,
        Self::Guisbourne,
        Self::Longchamp,
        Self::PrinceJohn,
        Self::Scathlock,
        Self::Sheriff,
    ];

    fn from_profile_filename(filename: &str) -> Self {
        match filename {
            "Guisbourne" => Self::Guisbourne,
            "Longchamp" => Self::Longchamp,
            "PrinceJohn" => Self::PrinceJohn,
            "Scatlock" => Self::Scathlock,
            "sherif" | "Sherif" => Self::Sheriff,
            _ => Self::Generic,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn pc_action_template(self) -> Option<CharacterKind> {
        match self {
            Self::Generic => None,
            Self::Guisbourne | Self::PrinceJohn => {
                Some(CharacterKind::RobinHood { is_town: false })
            }
            Self::Longchamp => Some(CharacterKind::WillScarlet),
            Self::Scathlock => Some(CharacterKind::FriarTuck),
            Self::Sheriff => Some(CharacterKind::Stutely),
        }
    }
}

const ACTION_SUB_ID_DISABLED: usize = 0;
const ACTION_SUB_ID_UNSELECTED: usize = 1;
const ACTION_SUB_ID_FOCUSED: usize = 2;
const ACTION_SUB_ID_SELECTED: usize = 3;
const ACTION_SUB_ID_FOCUSED_SELECTED: usize = 4;

fn action_button_visual(
    is_active: bool,
    is_disabled: bool,
    is_hovered: bool,
) -> ActionButtonVisual {
    if is_disabled {
        ActionButtonVisual::Disabled
    } else if is_active && is_hovered {
        ActionButtonVisual::HoverPressed
    } else if is_active {
        ActionButtonVisual::Pressed
    } else if is_hovered {
        ActionButtonVisual::Hover
    } else {
        ActionButtonVisual::Normal
    }
}

use robin_engine::resource_ids;

// ─── Scroll decoration resource IDs ───────────────────────────────
// These are generic parchment frame bitmaps shared by all portrait widgets.

/// Top scroll parchment banner (character name area).
const RHID_TOP_SCROLL: ResourceId = resource_ids::RHID_TOP_SCROLL;
/// Top scroll alternate (HP gauge overlay).
const RHID_TOP_SCROLL_ALTERNATE: ResourceId = resource_ids::RHID_TOP_SCROLL_ALTERNATE;
/// Bottom scroll parchment banner (ammo count area).
const RHID_BOTTOM_SCROLL: ResourceId = resource_ids::RHID_BOTTOM_SCROLL;

// ─── Panel border frame resource IDs ─────────────────────────────
// These form the ornamental frame around the bottom panel area.

const RHID_TOP_LEFT_CORNER: ResourceId = resource_ids::RHID_TOP_LEFT_CORNER;
const RHID_TOP_RIGHT_CORNER: ResourceId = resource_ids::RHID_TOP_RIGHT_CORNER;
const RHID_BOTTOM_LEFT_CORNER: ResourceId = resource_ids::RHID_BOTTOM_LEFT_CORNER;
const RHID_BOTTOM_RIGHT_CORNER: ResourceId = resource_ids::RHID_BOTTOM_RIGHT_CORNER;
const RHID_MIDDLE_800: ResourceId = resource_ids::RHID_MIDDLE_800;
const RHID_MIDDLE_1024: ResourceId = resource_ids::RHID_MIDDLE_1024;

// Border piece dimensions are derived from the bitmap surface sizes at runtime;
// the renderer auto-fits its bounding box to the resource size.

// Portrait resource IDs are pulled directly from `resource_ids` below.

// ─── Action button resource IDs ─────────────────────────────────

// ─── Localized name string resource IDs ────────────────────────

// ─── Portrait cache ───────────────────────────────────────────────

// ─── Requirements-bar per-slot sub-picture mapping ────────────────

/// Sub-picture index within `RHID_REQUIRED_ACTION` for a required action.
///
/// Returns the `UnknownAction=0` fallback for actions the widget does
/// not visualise.
pub(crate) fn required_action_sub_id(action: robin_engine::profiles::Action) -> usize {
    // Sub-id mapping: UnknownAction=0, Bow=1, Carry=2, Climb=3, Jump=4,
    //                 Lever=5, Lockpick=6, Stun=7, Tie=8, Eat=9, Search=10.
    use robin_engine::profiles::Action;
    match action {
        Action::Bow => 1,
        Action::LittleJohnCarry | Action::FarmerCarry => 2,
        Action::Climb => 3,
        Action::Jump => 4,
        Action::Lever => 5,
        Action::Lockpick => 6,
        Action::Hit | Action::HitHard => 7,
        Action::Tie => 8,
        Action::Eat | Action::Guzzle => 9,
        Action::Search => 10,
        _ => 0,
    }
}

/// Upload a 16-bit picture into a new renderer surface.
fn owned_picture_surface(
    renderer: &mut Renderer,
    owners: &mut Vec<OwnedSurface>,
    pic: &Picture,
) -> anyhow::Result<SurfaceHandle> {
    let owned = renderer
        .upload_rgb565_bytes(pic.width, pic.height, &pic.data)
        .ok_or_else(|| anyhow::anyhow!("portrait dimensions must match complete RGB565 payload"))?;
    let handle = owned.handle();
    owners.push(owned);
    Ok(handle)
}

fn picture_hit_mask(pic: &Picture, transparent_color: u16) -> anyhow::Result<HitMask> {
    anyhow::ensure!(
        pic.pixel_format == robin_assets::picture::PixelFormat::Rgb16,
        "portrait hit mask requires an RGB565 picture"
    );
    let (pixels, remainder) = pic.data.as_chunks::<2>();
    anyhow::ensure!(
        remainder.is_empty(),
        "portrait hit mask contains an incomplete pixel"
    );
    let opaque = pixels
        .iter()
        .map(|&bytes| u16::from_le_bytes(bytes) != transparent_color)
        .collect();
    HitMask::from_opacity(pic.width, pic.height, opaque).map_err(anyhow::Error::msg)
}

pub(crate) fn pic_to_surface(renderer: &mut Renderer, pic: &Picture) -> OwnedSurface {
    renderer
        .upload_rgb565_bytes(pic.width, pic.height, &pic.data)
        .expect("pic_to_surface: decoded picture dimensions must match complete RGB565 payload")
}

/// Read an engine-shipped UI asset through the virtual filesystem.
///
/// These files ship in `assets/core-datadir/Data/Interface/UI/` and are
/// resolved through the overlay system, so mods can restyle them by
/// overlaying the same path. They are required — a failed read means the
/// core overlay datadir is missing next to the game, which is an
/// installation error worth failing loudly on.
fn read_ui_asset(
    name: &str,
    files: &robin_engine::sbfile::SbFileSystem,
) -> anyhow::Result<robin_util::asset_fs::AssetBytes> {
    let path = format!("Data/Interface/UI/{name}");
    files.read_shared(&path).map_err(|error| {
        anyhow::anyhow!(
            "required UI asset {path} could not be read (error {error}); \
             is the core overlay datadir (assets/core-datadir/) missing?"
        )
    })
}

fn load_ui_image(
    renderer: &mut Renderer,
    files: &robin_engine::sbfile::SbFileSystem,
    file: &str,
    expected: (u16, u16),
    label: &str,
) -> anyhow::Result<GpuImage> {
    let (width, height, pixels) =
        decode_embedded_png_rgba(&read_ui_asset(file, files)?).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        (width, height) == expected,
        "UI image {file} has dimensions {width}x{height}, expected {}x{}",
        expected.0,
        expected.1
    );
    renderer
        .create_rgba_gpu_image(width, height, &pixels, label)
        .ok_or_else(|| anyhow::anyhow!("UI image {file} dimensions do not match its payload"))
}

fn decode_embedded_png_rgba(bytes: &[u8]) -> Result<(u16, u16, Vec<u8>), String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("decode embedded PNG header: {error}"))?;
    let header = reader.info();
    u16::try_from(header.width).map_err(|_| "embedded PNG width exceeds u16".to_owned())?;
    u16::try_from(header.height).map_err(|_| "embedded PNG height exceeds u16".to_owned())?;
    if header.bit_depth != png::BitDepth::Eight {
        return Err(format!(
            "embedded PNG uses unsupported bit depth {:?}",
            header.bit_depth
        ));
    }
    if !matches!(
        header.color_type,
        png::ColorType::Rgb | png::ColorType::Rgba
    ) {
        return Err(format!(
            "embedded PNG uses unsupported color type {:?}",
            header.color_type
        ));
    }
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| "embedded PNG has no known output size".to_owned())?
    ];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("decode embedded PNG frame: {error}"))?;
    buffer.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        color_type => {
            return Err(format!(
                "embedded PNG uses unsupported color type {color_type:?}"
            ));
        }
    };
    Ok((info.width as u16, info.height as u16, pixels))
}

/// The [`CharacterKind`] for a PC entity, if it is a PC whose profile
/// matched one of the 10 known characters at level-load time.
fn pc_character_kind(entity: &Entity) -> Option<CharacterKind> {
    match entity {
        Entity::Pc(pc) => pc.pc.kind,
        _ => None,
    }
}

fn pc_custom_visage_kind(
    entity: &Entity,
    profiles: &engine_profiles::ProfileManager,
) -> Option<AlliedVisageKind> {
    let pc = entity.pc_data()?;
    let profile = profiles.get_character(pc.profile_index).unwrap_or_else(|| {
        panic!(
            "PC references missing character profile {}",
            pc.profile_index
        )
    });
    let kind = AlliedVisageKind::from_profile_filename(&profile.filename);
    (kind != AlliedVisageKind::Generic).then_some(kind)
}

fn pc_action_character_kind(
    entity: &Entity,
    profiles: &engine_profiles::ProfileManager,
) -> Option<CharacterKind> {
    pc_character_kind(entity).or_else(|| {
        pc_custom_visage_kind(entity, profiles).and_then(AlliedVisageKind::pc_action_template)
    })
}

// ─── Helpers ───────────────────────────────────────────────────────

/// Minimum number of layout slots the bar is divided into. Matches the
/// original fixed five-slot layout so a small party stays spread out
/// instead of packing into the left corner.
const NUMBER_OF_SLOTS: usize = 5;

/// Maximum number of portraits that physically fit across the panel.
pub fn portrait_capacity(screen_width: u16) -> usize {
    usize::from(((screen_width.saturating_sub(2 * MARGIN)) / ELEMENT_WIDTH).max(1))
}

/// Number of layout slots the bar is divided into for `num_items` portraits.
pub(crate) fn portrait_slot_count(screen_width: u16, num_items: usize) -> usize {
    num_items.clamp(
        NUMBER_OF_SLOTS,
        portrait_capacity(screen_width).max(NUMBER_OF_SLOTS),
    )
}

/// Compute the left X of a portrait element within its slot.
pub(crate) fn slot_left_x(screen_width: u16, slot_index: u16, slot_count: usize) -> u16 {
    let sw = screen_width.saturating_sub(2 * MARGIN) / slot_count.max(1) as u16;
    let position_in_slot = MARGIN + sw.saturating_sub(ELEMENT_WIDTH) / 2;
    slot_index * sw + position_in_slot
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortraitTarget {
    Pc(EntityId),
    AlliedSelection,
    AlliedGroup(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum PortraitBarItem<'a> {
    Pc(EntityId),
    AlliedGroup {
        id: u32,
        members: Cow<'a, [EntityId]>,
    },
    AlliedSelection(Cow<'a, [EntityId]>),
}

impl PortraitBarItem<'_> {
    pub(crate) fn target(&self) -> PortraitTarget {
        match self {
            Self::Pc(id) => PortraitTarget::Pc(*id),
            Self::AlliedGroup { id, .. } => PortraitTarget::AlliedGroup(*id),
            Self::AlliedSelection(_) => PortraitTarget::AlliedSelection,
        }
    }

    pub(crate) fn members(&self) -> &[EntityId] {
        match self {
            Self::Pc(id) => std::slice::from_ref(id),
            Self::AlliedGroup { members, .. } | Self::AlliedSelection(members) => members,
        }
    }

    fn queue_strip_identity(&self) -> crate::host::QueueStripIdentity {
        use crate::host::QueueStripIdentity;
        match self.target() {
            PortraitTarget::Pc(id) => QueueStripIdentity::Pc(id),
            PortraitTarget::AlliedGroup(id) => QueueStripIdentity::AlliedGroup(id),
            PortraitTarget::AlliedSelection => {
                let mut members = self.members().to_vec();
                members.sort_unstable();
                QueueStripIdentity::AlliedSelection(members)
            }
        }
    }
}

pub(crate) fn portrait_bar_items<'a>(
    engine: &'a PresentationView<'_>,
    seat: PlayerId,
    screen_width: u16,
) -> (Vec<PortraitBarItem<'a>>, bool) {
    build_portrait_page(
        &engine.displayed_pc_ids(),
        engine.tactical_pinned_groups(seat),
        engine.tactical_selection(seat),
        portrait_capacity(screen_width),
        engine.tactical_first_visible_portrait(seat),
    )
}

/// Construct only visible items, keeping heroes, pinned groups, and the
/// optional transient selection in their established cyclic order.
fn build_portrait_page<'a>(
    pcs: &[EntityId],
    groups: &'a [TacticalPinnedGroup],
    selection: &'a [EntityId],
    capacity: usize,
    first_visible: usize,
) -> (Vec<PortraitBarItem<'a>>, bool) {
    let include_selection =
        !selection.is_empty() && !groups.iter().any(|group| group.members == selection);
    let count = pcs.len() + groups.len() + usize::from(include_selection);
    let paged = count > capacity;
    let offset = if paged { first_visible % count } else { 0 };
    let items = (offset..count)
        .chain(0..offset)
        .take(capacity)
        .map(|index| {
            if let Some(&pc) = pcs.get(index) {
                PortraitBarItem::Pc(pc)
            } else if let Some(group) = groups.get(index - pcs.len()) {
                PortraitBarItem::AlliedGroup {
                    id: group.id,
                    members: Cow::Borrowed(&group.members),
                }
            } else {
                assert!(
                    include_selection,
                    "portrait page index must refer to an existing item"
                );
                PortraitBarItem::AlliedSelection(Cow::Borrowed(selection))
            }
        })
        .collect();
    (items, paged)
}

fn bbox(x1: u16, y1: u16, x2: u16, y2: u16) -> BBox {
    screen_bbox_to_sprite_bbox(ScreenBBox::from_coords(
        x1 as f32, y1 as f32, x2 as f32, y2 as f32,
    ))
}

fn screen_bbox_to_sprite_bbox(bbox: ScreenBBox) -> BBox {
    let min = bbox.top_left();
    let max = bbox.bottom_right();
    BBox::from_coords(min.x, min.y, max.x, max.y)
}

fn portrait_surface_dimensions(renderer: &Renderer, surface: SurfaceHandle) -> (u16, u16) {
    renderer
        .surface_dimensions(surface)
        .expect("live portrait surface")
}

fn blit_to_screen_widget(
    renderer: &mut Renderer,
    surface: SurfaceHandle,
    src: Option<&BBox>,
    dst: Option<&BBox>,
    flags: u32,
) {
    // The old widget bridge used a zero-origin transform and integer rectangles.
    // Preserve that geometry while retaining renderer provenance through submission.
    let (width, height) = portrait_surface_dimensions(renderer, surface);
    let src = src
        .copied()
        .unwrap_or_else(|| BBox::from_coords(0.0, 0.0, width as f32, height as f32));
    let dst = dst.copied().unwrap_or(src);
    let integer_rect = |rect: BBox| {
        let x = rect.min.x as i32;
        let y = rect.min.y as i32;
        BBox::from_coords(
            x as f32,
            y as f32,
            (x + rect.width() as i32) as f32,
            (y + rect.height() as i32) as f32,
        )
    };
    renderer
        .draw_surface(
            surface,
            Some(&integer_rect(src)),
            Some(&integer_rect(dst)),
            flags,
        )
        .expect("portrait draw requires the originating renderer and a live upload");
}

/// Check if a PC is in coma state (amulet death-save, still alive but burned).
///
/// Coma PCs have `in_coma=true` in their campaign PcStatus and
/// life_points=5 (set by wound handling). They render as burned portraits
/// with the health gauge visible. Fully dead PCs have life_points<=0
/// and are NOT in coma — their scrolls are hidden entirely.
fn is_pc_in_coma(engine: &PresentationView<'_>, entity: &Entity) -> bool {
    let profile_idx = match entity.pc_data() {
        Some(pc) => pc.profile_index,
        None => return false,
    };
    let Some(desc) = engine.campaign().characters.get(usize::from(profile_idx)) else {
        tracing::warn!(
            ?profile_idx,
            "portrait PC is missing its campaign character descriptor"
        );
        return false;
    };
    desc.status.in_coma
}

/// Determine which action button index (0..=2) is currently active for a PC.
///
/// Compares the PC's `current_action` against the profile's `actions[]` array.
/// Returns `None` if `current_action == NoAction` or doesn't match any slot.
fn active_action_index(profiles: &engine_profiles::ProfileManager, entity: &Entity) -> Option<u8> {
    use robin_engine::profiles::Action;
    let pc = entity.pc_data()?;
    if pc.current_action == Action::NoAction {
        return None;
    }
    let profile = profiles.get_character(pc.profile_index)?;
    profile
        .actions
        .iter()
        .position(|a| *a == pc.current_action)
        .map(|i| i as u8)
}

fn action_index(
    profiles: &engine_profiles::ProfileManager,
    entity: &Entity,
    action: engine_profiles::Action,
) -> Option<u8> {
    if action == engine_profiles::Action::NoAction {
        return None;
    }
    let profile = profiles.get_character(entity.pc_data()?.profile_index)?;
    profile
        .actions
        .iter()
        .position(|candidate| *candidate == action)
        .map(|index| index as u8)
}

fn allied_action_index(relative_x: f32) -> u8 {
    ((relative_x / (ELEMENT_WIDTH as f32 / 3.0)).floor() as u8).min(2)
}

fn allied_state_icon_index(
    action: u8,
    order: Option<&robin_engine::tactical_control::TacticalUnitOrder>,
) -> usize {
    match action {
        0 => match order.map_or(CombatStance::Defensive, |order| order.stance) {
            CombatStance::Hold => 0,
            CombatStance::Defensive => 1,
            CombatStance::Aggressive => 2,
        },
        1 => {
            if order.is_some_and(|order| matches!(order.duty, TacticalDuty::Patrol { .. })) {
                4
            } else {
                3
            }
        }
        2 => match order.map_or(TacticalFormation::Line, |order| order.formation) {
            TacticalFormation::Line => 5,
            TacticalFormation::Box => 6,
            TacticalFormation::Staggered => 7,
            TacticalFormation::Flank => 8,
        },
        _ => panic!("allied action index {action} is outside the three-button row"),
    }
}

fn render_allied_portrait_layer(
    renderer: &mut Renderer,
    image: &GpuImage,
    x: u16,
    sh: u16,
    selected: bool,
) {
    if selected {
        let top = sh - PORTRAIT_TOTAL_HEIGHT;
        renderer.render_gpu_image(
            image,
            None,
            Some(&bbox(
                x,
                top,
                x + ELEMENT_WIDTH,
                top + PORTRAIT_TOTAL_HEIGHT,
            )),
            BlendMode::Blend,
        );
        return;
    }

    // Closed portraits omit the 35-pixel action row. Preserve the authored
    // layer boundaries and close the gap exactly like the native PC widget.
    for (source, destination) in [
        (
            bbox(0, 0, ELEMENT_WIDTH, TOP_SCROLL_HEIGHT),
            bbox(
                x,
                sh - CLOSE_POSITION_TOP_SCROLL,
                x + ELEMENT_WIDTH,
                sh - CLOSE_POSITION_VISAGE,
            ),
        ),
        (
            bbox(
                0,
                TOP_SCROLL_HEIGHT,
                ELEMENT_WIDTH,
                TOP_SCROLL_HEIGHT + VISAGE_HEIGHT,
            ),
            bbox(
                x,
                sh - CLOSE_POSITION_VISAGE,
                x + ELEMENT_WIDTH,
                sh - CLOSE_POSITION_BOTTOM_SCROLL,
            ),
        ),
        (
            bbox(
                0,
                TOP_SCROLL_HEIGHT + VISAGE_HEIGHT + ACTION_HEIGHT,
                ELEMENT_WIDTH,
                PORTRAIT_TOTAL_HEIGHT - BORDURE,
            ),
            bbox(
                x,
                sh - CLOSE_POSITION_BOTTOM_SCROLL,
                x + ELEMENT_WIDTH,
                sh - BORDURE,
            ),
        ),
    ] {
        renderer.render_gpu_image(image, Some(&source), Some(&destination), BlendMode::Blend);
    }
}

/// Advance visible automatic strips once per fixed tick, not once per capture
/// or physical-display refresh. This is presentation-only animation state.
pub(crate) fn prepare_auto_queue_animations(
    frontend: &mut HostFrontend,
    engine: &PresentationView<'_>,
    seat: PlayerId,
    screen_width: u16,
) {
    let (items, _) = portrait_bar_items(engine, seat, screen_width);
    let mut prepared = Vec::with_capacity(items.len());
    for item in items {
        if let PortraitTarget::Pc(id) = item.target() {
            let entity = engine
                .get_entity(id)
                .expect("displayed portrait must have an entity");
            if matches!(entity, Entity::Pc(pc) if pc.pc.life_points <= 0)
                || is_pc_in_coma(engine, entity)
            {
                continue;
            }
        }
        assert!(
            !item.members().is_empty(),
            "automatic queue strip cannot have an empty member list"
        );
        let count = item
            .members()
            .iter()
            .map(|id| engine.automatic_quick_action_count(*id))
            .sum();
        prepared.push((item.queue_strip_identity(), count));
    }
    frontend.prepare_queue_strip_animations(seat, prepared);
}

fn render_auto_queue_ticks(
    frontend: &HostFrontend,
    renderer: &mut Renderer,
    engine: &PresentationView<'_>,
    seat: PlayerId,
    identity: crate::host::QueueStripIdentity,
    members: &[EntityId],
    x: u16,
    base_y: i32,
) {
    assert!(
        !members.is_empty(),
        "automatic queue strip cannot have an empty member list"
    );
    let queue_count: usize = members
        .iter()
        .map(|member| engine.automatic_quick_action_count(*member))
        .sum();
    // Missing animation is the legitimate first, pre-update thumbnail state:
    // a strip has no previous queue from which to animate a collapse.
    let fall_offset =
        frontend
            .queue_strip_animations()
            .displayed_offset(seat, &identity, queue_count);
    if queue_count == 0 {
        return;
    }
    let color = Renderer::create_color_16(238, 192, 55);
    let visible = queue_count.min(12);
    for index in 0..visible {
        // Original-game falling-button behavior offsets the surviving
        // quick-action icon horizontally, then reduces that elevation on
        // every fixed tick. Keep the automatic strip independent, but preserve
        // the same right-to-left tetris collapse.
        let left = i32::from(x) + 5 + index as i32 * 8 + fall_offset;
        let height = if index == 11 && queue_count > 12 {
            8
        } else {
            5
        };
        renderer.draw_line_screen(left, base_y, left, base_y + height, color);
        renderer.draw_line_screen(left + 1, base_y, left + 1, base_y + height, color);
    }
}

fn render_allied_portrait(
    frontend: &HostFrontend,
    renderer: &mut Renderer,
    portraits: &PortraitCache,
    engine: &PresentationView<'_>,
    profiles: &engine_profiles::ProfileManager,
    seat: PlayerId,
    item: &PortraitBarItem<'_>,
    x: u16,
    sh: u16,
    hovered_action: Option<u8>,
) {
    let selected = engine.tactical_selection(seat) == item.members();
    let top_scroll = if selected {
        POSITION_TOP_SCROLL
    } else {
        CLOSE_POSITION_TOP_SCROLL
    };
    render_allied_portrait_layer(
        renderer,
        portraits
            .allied_portrait_background
            .as_ref()
            .expect("allied portrait background must be loaded before drawing the HUD"),
        x,
        sh,
        selected,
    );
    let visage_kind = allied_visage_kind(engine, profiles, item.members());
    let visage = portraits.allied_visages[visage_kind.index()]
        .as_ref()
        .unwrap_or_else(|| panic!("allied visage {visage_kind:?} was not loaded"));
    let visage_top = if selected {
        sh - POSITION_VISAGE
    } else {
        sh - CLOSE_POSITION_VISAGE
    };
    renderer.render_gpu_image(
        visage,
        None,
        Some(&bbox(
            x,
            visage_top,
            x + ELEMENT_WIDTH,
            visage_top + VISAGE_HEIGHT,
        )),
        BlendMode::Blend,
    );

    // Reuse the native merry-man crossed-swords overlay for controlled
    // soldiers. A group counts as fighting while any surviving member is in
    // a sword action or has an active melee opponent. Match hero portraits:
    // the overlay stays visible on the open portrait and blinks while closed.
    let is_sword_fighting = item.members().iter().any(|member| {
        engine.get_entity(*member).is_some_and(|entity| {
            entity
                .actor_data()
                .is_some_and(|actor| actor.action_state.is_sword())
                || entity
                    .human_data()
                    .is_some_and(|human| !human.opponents.is_empty())
        })
    });
    let fighting_visible = selected || (engine.frame_counter() / 10).is_multiple_of(2);
    if is_sword_fighting
        && fighting_visible
        && let Some(surface) = portraits.get_fighting_surface(CharacterKind::MerryManA)
    {
        let visage_top = if selected {
            sh - POSITION_VISAGE
        } else {
            sh - CLOSE_POSITION_VISAGE
        };
        let (width, height) = portrait_surface_dimensions(renderer, surface);
        blit_to_screen_widget(
            renderer,
            surface,
            None,
            Some(&bbox(x, visage_top, x + width, visage_top + height)),
            BLIT_SOURCE_TRANSPARENT,
        );
    }

    // Pin/unpin button remains visible in both open and closed states.
    let pin_x = x + ALLIED_PIN_LEFT;
    let pin_y = sh - top_scroll - ALLIED_PIN_RISE;
    let pin_index = if matches!(item.target(), PortraitTarget::AlliedGroup(_)) {
        1
    } else {
        0
    };
    let pin = portraits.allied_pin_icons[pin_index]
        .as_ref()
        .expect("allied pin icon must be loaded before drawing the HUD");
    renderer.render_gpu_image(
        pin,
        None,
        Some(&bbox(
            pin_x,
            pin_y,
            pin_x + ALLIED_PIN_ICON_SIZE,
            pin_y + ALLIED_PIN_ICON_SIZE,
        )),
        BlendMode::Blend,
    );

    // Automatic work is separate from Original's three macro slots. Give
    // tactical group portraits an unambiguous pending-work strip: one gold tick
    // per queued soldier action, capped to the portrait width with a final
    // longer overflow tick.
    render_auto_queue_ticks(
        frontend,
        renderer,
        engine,
        seat,
        item.queue_strip_identity(),
        item.members(),
        x,
        i32::from(sh - top_scroll + 4),
    );

    if selected {
        let action_top = sh - POSITION_ACTION;
        let action_bottom = sh - POSITION_BOTTOM_SCROLL;
        let order = item
            .members()
            .first()
            .and_then(|soldier| engine.tactical_order(*soldier));
        let button_w = ELEMENT_WIDTH / 3;
        for index in 0..3 {
            let left = x + index as u16 * button_w;
            let right = if index == 2 {
                x + ELEMENT_WIDTH
            } else {
                left + button_w
            };
            let active = index == 1
                && order.is_some_and(|order| matches!(order.duty, TacticalDuty::Patrol { .. }));
            let icon_index = allied_state_icon_index(index as u8, order);
            let image = portraits.allied_action_surfaces[icon_index]
                .as_ref()
                .unwrap_or_else(|| panic!("allied state icon {icon_index} was not loaded"));
            let scale = f32::min(
                (right - left - 2) as f32 / ALLIED_ACTION_ICON_WIDTH as f32,
                (action_bottom - action_top - 2) as f32 / ALLIED_ACTION_ICON_HEIGHT as f32,
            );
            let width = ((ALLIED_ACTION_ICON_WIDTH as f32 * scale).round() as u16).max(1);
            let height = ((ALLIED_ACTION_ICON_HEIGHT as f32 * scale).round() as u16).max(1);
            let icon_x = left + (right - left - width) / 2;
            let icon_y = action_top + (action_bottom - action_top - height) / 2;
            renderer.render_gpu_image(
                image,
                None,
                Some(&bbox(icon_x, icon_y, icon_x + width, icon_y + height)),
                BlendMode::Blend,
            );

            if active || hovered_action == Some(index as u8) {
                let color = if active {
                    Renderer::create_color_16(238, 192, 55)
                } else {
                    Renderer::create_color_16(224, 211, 157)
                };
                renderer.draw_line_screen(
                    i32::from(left + 3),
                    i32::from(action_bottom - 2),
                    i32::from(right - 4),
                    i32::from(action_bottom - 2),
                    color,
                );
            }
        }
    }
}

fn allied_visage_kind(
    engine: &PresentationView<'_>,
    profiles: &engine_profiles::ProfileManager,
    members: &[EntityId],
) -> AlliedVisageKind {
    let mut resolved = members.iter().map(|member| {
        let entity = engine
            .get_entity(*member)
            .unwrap_or_else(|| panic!("allied portrait member {member:?} disappeared"));
        let Entity::Soldier(soldier) = entity else {
            panic!("allied portrait member {member:?} is not a soldier");
        };
        let profile = profiles
            .get_soldier(soldier.soldier.soldier_profile_index)
            .unwrap_or_else(|| {
                panic!(
                    "allied portrait member {member:?} references missing soldier profile {}",
                    soldier.soldier.soldier_profile_index
                )
            });
        AlliedVisageKind::from_profile_filename(&profile.filename)
    });
    let Some(first) = resolved.next() else {
        panic!("allied portrait has no members");
    };
    if resolved.all(|kind| kind == first) {
        first
    } else {
        AlliedVisageKind::Generic
    }
}

// ─── Public API ────────────────────────────────────────────────────

/// Load the 7-hero localized display name map from `Level.res`.
///
/// Tries the campaign menu text table first, then demo variants in
/// order.  The map is fed to [`PortraitCache::install_localized_names`]
/// and consulted at render time by the HUD's `entity_display_name`
/// (PC branch) and by the peasant-name generator (to avoid colliding
/// with hero names).
pub fn load_localized_character_names(
    text_res: &mut ResourceManager,
) -> [Option<String>; CharacterKind::COUNT] {
    let mut out: [Option<String>; CharacterKind::COUNT] = [const { None }; CharacterKind::COUNT];
    let mut loaded = 0usize;
    for kind in CharacterKind::VARIANTS {
        // Display names are generated once at gang-creation time from the
        // campaign character profile, which is always forest Robin; the
        // per-level forest/town profile swap never regenerates the name.
        // "Robin Town" (id 145) is therefore never shown — both Robin
        // variants display the forest name.
        let str_id = match (CharacterKind::RobinHood { is_town: false })
            .localized_name_string_id()
            .filter(|_| kind.is_robin())
            .or_else(|| kind.localized_name_string_id())
        {
            Some(id) => id,
            None => continue,
        };
        if let Some((localized, table_id, sub_id)) = menu_text_string(text_res, str_id) {
            tracing::info!(
                "Localized name for {kind:?}: {localized:?} (table {table_id}, sub {sub_id})"
            );
            out[kind.as_index()] = Some(localized);
            loaded += 1;
        }
    }
    tracing::info!("Loaded {loaded} localized character names");
    out
}

/// Resolve Original text; presentation-only callers log malformed tables and
/// retain their existing optional-label policy.
pub(crate) fn menu_text_string(
    res: &mut ResourceManager,
    sub_id: usize,
) -> Option<(String, ResourceId, usize)> {
    match robin_assets::original_text::menu_text_string(res, sub_id) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(sub_id, "Cannot resolve Original menu text: {error:#}");
            None
        }
    }
}

/// Build a `BBox` from signed i32 screen coordinates (helper for layouts
/// that compute positions in signed space — e.g. the `-1` offset of the
/// requirements-bar selected ring).
fn bbox_i32(x0: i32, y0: i32, x1: i32, y1: i32) -> BBox {
    screen_bbox_to_sprite_bbox(ScreenBBox::from_coords(
        x0 as f32, y0 as f32, x1 as f32, y1 as f32,
    ))
}

// ─── PC info popup overlay ────────────────────────────────────────

/// Render the hovered-PC info popup.
///
/// Resolves the hovered PC's sword/bow capacity from the campaign
/// `HumanStatus`, re-computes the pip layout each frame (skills are
/// re-read on every show), then blits the background + lit pip sprites.
/// Does nothing when the overlay is not visible.
///
/// `mouse` is the current mouse cursor position — the overlay clamps
/// itself to the screen bounds each frame.
pub fn draw_pc_info_overlay(
    frontend: &HostFrontend,
    engine: &PresentationView<'_>,
    profiles: &engine_profiles::ProfileManager,
    renderer: &mut Renderer,
    portraits: &PortraitCache,
    mouse: ScreenPoint,
) {
    use crate::pc_info_overlay::{LEVEL_NUMBER, PcInfoOverlay};

    let ov = &frontend.presentation.pc_info_overlay;
    if !ov.visible {
        return;
    }
    let Some(pc_id) = ov.pc_id else { return };
    let Some(Entity::Pc(pc)) = engine.get_entity(pc_id) else {
        return;
    };

    // Pull sword / bow capacity from the campaign character descriptor.
    let campaign = engine.campaign();
    let Some(desc) = campaign.characters.get(usize::from(pc.pc.profile_index)) else {
        return;
    };
    let sword_cap = desc.status.human_status.hand_to_hand.capacity;
    let bow_cap = desc.status.human_status.bow.capacity;

    // Archer iff the PC's profile lists a Bow action.
    let Some(profile) = profiles.get_character(pc.pc.profile_index) else {
        tracing::warn!(
            ?pc_id,
            profile_index = ?pc.pc.profile_index,
            "PC-info overlay is missing its character profile"
        );
        return;
    };
    let is_archer = profile.actions.contains(&Action::Bow);

    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;

    // Compute positions + pip counts for this frame (recomputed every
    // show, since skills can change between displays).
    let mut frame_ov = PcInfoOverlay::default();
    frame_ov.show(pc_id, mouse, (sw, sh), is_archer, sword_cap, bow_cap);

    // ── Background ──
    let bg_sid = if is_archer {
        portraits.info_popup_bg_huge
    } else {
        portraits.info_popup_bg_tiny
    };
    if let Some(sid) = bg_sid {
        let (w, h) = portrait_surface_dimensions(renderer, sid);
        let (x, y) = (frame_ov.position.0 as u16, frame_ov.position.1 as u16);
        let dst = bbox(x, y, x + w, y + h);
        blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }

    // ── Sword pips ──
    if let Some(sid) = portraits.info_popup_sword {
        let (w, h) = portrait_surface_dimensions(renderer, sid);
        for i in 0..frame_ov.sword_pips.min(LEVEL_NUMBER) {
            let (px, py) = frame_ov.sword_pip_position(i);
            let dst = bbox(px as u16, py as u16, px as u16 + w, py as u16 + h);
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
    }

    // ── Bow pips (archer only) ──
    if is_archer && let Some(sid) = portraits.info_popup_bow {
        let (w, h) = portrait_surface_dimensions(renderer, sid);
        for i in 0..frame_ov.bow_pips.min(LEVEL_NUMBER) {
            let (px, py) = frame_ov.bow_pip_position(i);
            let dst = bbox(px as u16, py as u16, px as u16 + w, py as u16 + h);
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
    }
}

// ─── Dotted-chain rendering (world space) ─────────────────────────

/// Render the per-PC macro dotted chains.
///
/// For every PC with at least one non-empty macro slot, walks the
/// recorded steps and calls
/// `DrawManager::draw_dotted_line(… DISTANCE_DOT, 1, 0x0000 …)` for each
/// segment starting at the PC's map position.  The dot phase is a
/// single field (`TitbitManager::dotted_start`) shared across all PCs.
pub fn render_macro_dotted_chains(
    draw_manager: &crate::draw_manager::DrawManager,
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
) {
    use robin_engine::macro_store::DISTANCE_DOT;

    // Snapshot PC positions before composing each recorded chain. Drawing
    // uses only the frontend draw manager and never writes simulation state.
    let mut per_pc: Vec<(
        robin_engine::element::EntityId,
        engine_coordinates::MapPoint,
    )> = Vec::with_capacity(engine.pc_ids().len());
    for &pc_id in engine.pc_ids() {
        if let Some(ent) = engine.get_entity(pc_id) {
            let pos = ent.element_data().position_map();
            per_pc.push((pc_id, pos));
        }
    }

    // The dotted-phase is chained across every segment draw within a
    // frame; since the engine-owned phase is advanced once per tick
    // (`TitbitManager::prepare_refresh`), the renderer reads the current
    // phase and chains locally across segments.  Not writing back
    // preserves the mutation-free invariant — next frame's tick will
    // re-advance the canonical phase.
    let mut phase = engine.titbit_dotted_start();
    for (pc_id, pc_pos) in per_pc {
        let Some(state) = engine.portrait_macro(pc_id) else {
            continue;
        };
        if state.slots().iter().all(|s| s.is_empty()) {
            continue;
        }

        // Gather every QA-memory titbit into a single list and walk it
        // once with `from` carrying forward across slots — the polyline
        // is `PC → slot0 → slot1 → slot2`, not three separate fans from
        // the PC.
        let mut from = pc_pos;
        for slot in state.slots() {
            for step in &slot.steps {
                let to = step.position;
                draw_manager.draw_dotted_line(
                    renderer,
                    from,
                    to,
                    &mut phase,
                    DISTANCE_DOT,
                    1.0,
                    0x0000,
                );
                from = to;
            }
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use tests::verify_portrait_gpu_ownership;

mod portrait_cache;
pub use portrait_cache::PortraitCache;
mod panel;
pub use panel::draw_panel;
mod blazon_bar;
pub use blazon_bar::{BlazonSlotKind, blazon_bar_slot_kinds, draw_blazon_bar, hit_test_blazon_bar};
mod requirements_bar;
#[cfg(test)]
use requirements_bar::*;
pub use requirements_bar::{draw_requirements_bar, hit_test_requirements_bar};
mod tooltips;
pub use tooltips::{
    BlazonTooltipTracker, HoverTooltipTracker, PC_ACTION_TOOLTIP_DELAY_TICKS,
    PcActionTooltipTracker, REQUIREMENTS_TOOLTIP_DELAY_TICKS, RequirementsTooltipTracker,
    action_button_tooltip_mt_id, blazon_slot_tooltip_mt_id, draw_screen_tooltip,
    item_action_tooltip_extension, requirements_slot_tooltip_mt_id,
};
mod hit_test;
pub use hit_test::{PortraitHit, PortraitHitArea, hit_test_portrait, hit_test_portrait_detailed};
