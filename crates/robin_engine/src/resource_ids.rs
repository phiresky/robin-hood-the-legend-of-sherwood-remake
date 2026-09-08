//! Resource ID constants.
//!
//! These are used to load sprites, cursors, portraits, UI elements, and other
//! game resources by numeric ID.

// ── General UI / QA buttons ──────────────────────────────────────────────────

/// QA recording button
pub const RHID_CLOCK: i32 = 1;
/// Button to tell character to crouch
pub const RHID_DOWN_ARROW: i32 = 2;
/// Button to tell character to stand up
pub const RHID_UP_ARROW: i32 = 3;
/// Zoom up button
pub const RHID_ZOOM_UP: i32 = 4;
/// Zoom down button
pub const RHID_ZOOM_DOWN: i32 = 5;
/// Image indicating a character is selected
pub const RHID_GROUND_SELECT: i32 = 6;
/// Powder
pub const RHID_POWDER: i32 = 7;
/// Burn effect for special mouse cursor
pub const RHID_BLIND: i32 = 8;

// ── Stun stars animation (5 frames) ─────────────────────────────────────────

pub const RHID_FIVE_STARS: i32 = 9;
pub const RHID_FOUR_STARS: i32 = 10;
pub const RHID_THREE_STARS: i32 = 11;
pub const RHID_TWO_STARS: i32 = 12;
pub const RHID_ONE_STAR: i32 = 13;

// ── Quick Action UI ─────────────────────────────────────────────────────────

/// Standard widget button for QA icons
pub const RHID_QUICKACTION: i32 = 14;
/// Image shown during QA recording
pub const RHID_QUICKACTION_IN_PROGRESS: i32 = 15;
/// Small QA icon collection displayed on characters
pub const RHID_QUICKACTION_TITBITS: i32 = 16;
/// Quick action start button
pub const RHID_QUICKSTART: i32 = 23;
/// Small drawing for QA (not on this level)
pub const DVID_QUICKACTION_NOTONTHISLEVEL: i32 = 34;

// ── Mouse cursors ───────────────────────────────────────────────────────────

/// Default mouse cursor
pub const RHMOUSE_DEFAULT: i32 = 17;
/// Confirmation mouse cursor
pub const RHMOUSE_OK: i32 = 22;
/// Default cursor with shift held (outline)
pub const RHMOUSE_DEFAULT_OUTLINE: i32 = 28;
/// Can't-go-there cursor with shift held (outline)
pub const RHMOUSE_CANTGOTHERE_OUTLINE: i32 = 29;
/// Can't-go-there cursor
pub const RHMOUSE_CANTGOTHERE: i32 = 35;
/// Climbing / floor change cursor
pub const RHMOUSE_CLIMBING: i32 = 36;
/// Aiming cursor (bow)
pub const RHMOUSE_BOW_YES: i32 = 52;
/// Aiming impossible cursor (bow)
pub const RHMOUSE_BOW_NO: i32 = 53;
/// Peanut throw cursor (yes)
pub const RHMOUSE_PEANUT_YES: i32 = 58;
/// Peanut throw cursor (no)
pub const RHMOUSE_PEANUT_NO: i32 = 59;
/// Aiming at VIP cursor
pub const RHMOUSE_BOW_VIP: i32 = 62;
/// Aiming at civilian cursor
pub const RHMOUSE_BOW_CIVIL: i32 = 63;
/// Sword fight cursor
pub const RHMOUSE_SWORDFIGHT_YES: i32 = 64;
/// Finish downed enemy cursor
pub const RHMOUSE_FINISH_HIM: i32 = 65;
pub const RHMOUSE_JUMP_HIGH: i32 = 66;
pub const RHMOUSE_JUMP_LOW: i32 = 67;
/// Pick up item cursor (yes)
pub const RHMOUSE_GET_YES: i32 = 91;
/// Pick up item cursor (no)
pub const RHMOUSE_GET_NO: i32 = 92;
pub const RHMOUSE_WAKE_UP: i32 = 93;
/// Hit cursor (yes)
pub const RHMOUSE_HIT_YES: i32 = 94;
/// Hit cursor (no)
pub const RHMOUSE_HIT_NO: i32 = 95;
/// Lockpick cursor (yes)
pub const RHMOUSE_LOCKPICK_YES: i32 = 96;
/// Lockpick cursor (no)
pub const RHMOUSE_LOCKPICK_NO: i32 = 97;
/// Shield cursor (big)
pub const RHMOUSE_BIG_SHIELD_YES: i32 = 98;
/// Short leg-up cursor
pub const RHMOUSE_SHORT_LEG: i32 = 99;
/// Throw money cursor (yes)
pub const RHMOUSE_PURSE_YES: i32 = 100;
/// Throw money cursor (no)
pub const RHMOUSE_PURSE_NO: i32 = 101;
pub const RHMOUSE_TIE: i32 = 102;
pub const RHMOUSE_PAY_YES: i32 = 103;
pub const RHMOUSE_PAY_NO: i32 = 104;
pub const RHMOUSE_HEAL_YES: i32 = 105;
pub const RHMOUSE_HEAL_NO: i32 = 106;
/// Barrel throw cursor (Donkey Kong style)
pub const RHMOUSE_DONKEY_KONG_YES: i32 = 107;
/// Barrel throw cursor (no)
pub const RHMOUSE_DONKEY_KONG_NO: i32 = 108;
pub const RHMOUSE_BEND_BOW_YES: i32 = 109;
pub const RHMOUSE_BEND_BOW_CIVIL: i32 = 110;
pub const RHMOUSE_BEND_BOW_VIP: i32 = 111;
pub const RHMOUSE_SEARCH: i32 = 112;
/// Long-range bow VIP cursor
pub const RHMOUSE_BOW_VIP_LONG: i32 = 166;
/// Long-range bow civilian cursor
pub const RHMOUSE_BOW_CIVILIAN_LONG: i32 = 167;
/// Long-range bow cursor (yes)
pub const RHMOUSE_BOW_YES_LONG: i32 = 168;
/// Strangle cursor (yes)
pub const RHMOUSE_STRANGLE_YES: i32 = 169;
/// Strangle cursor (no)
pub const RHMOUSE_STRANGLE_NO: i32 = 170;
/// Ale cursor (yes)
pub const RHMOUSE_ALE_YES: i32 = 171;
/// Ale cursor (no)
pub const RHMOUSE_ALE_NO: i32 = 172;
/// Net cursor (yes)
pub const RHMOUSE_NET_YES: i32 = 173;
/// Net cursor (no)
pub const RHMOUSE_NET_NO: i32 = 174;
/// Wasp nest cursor (yes)
pub const RHMOUSE_WASP_NEST_YES: i32 = 175;
/// Wasp nest cursor (no)
pub const RHMOUSE_WASP_NEST_NO: i32 = 176;
/// Apple cursor (yes)
pub const RHMOUSE_APPLE_YES: i32 = 177;
/// Apple cursor (no)
pub const RHMOUSE_APPLE_NO: i32 = 178;
/// Stone cursor (yes)
pub const RHMOUSE_STONE_YES: i32 = 179;
/// Stone cursor (no)
pub const RHMOUSE_STONE_NO: i32 = 180;
/// View cursor
pub const RHMOUSE_VIEW: i32 = 181;
/// Lever cursor (yes)
pub const RHMOUSE_LEVER_YES: i32 = 182;
/// Lever cursor (no)
pub const RHMOUSE_LEVER_NO: i32 = 183;
/// Target cut cursor (no)
pub const RHMOUSE_TARGET_CUT_NO: i32 = 196;
/// Target cut cursor (yes)
pub const RHMOUSE_TARGET_CUT_YES: i32 = 197;
/// Target handle cursor (yes)
pub const RHMOUSE_TARGET_HANDLE_YES: i32 = 198;
/// Target handle cursor (no)
pub const RHMOUSE_TARGET_HANDLE_NO: i32 = 199;
/// Pick up item cursor variants (1-5)
pub const RHMOUSE_GET_YES_1: i32 = 202;
pub const RHMOUSE_GET_YES_2: i32 = 203;
pub const RHMOUSE_GET_YES_3: i32 = 204;
pub const RHMOUSE_GET_YES_4: i32 = 205;
pub const RHMOUSE_GET_YES_5: i32 = 206;
/// Shield cursor (yes)
pub const RHMOUSE_SHIELD_YES: i32 = 245;
/// Big shield point cursor
pub const RHMOUSE_BIG_SHIELD_POINT: i32 = 246;
/// Shield point cursor
pub const RHMOUSE_SHIELD_POINT: i32 = 247;
/// Talk cursor
pub const RHMOUSE_TALK: i32 = 248;
/// Big shield cursor (no)
pub const RHMOUSE_BIG_SHIELD_NO: i32 = 268;
/// Shield cursor (no)
pub const RHMOUSE_SHIELD_NO: i32 = 269;
/// Door cursor (yes)
pub const RHMOUSE_DOOR_YES: i32 = 270;
/// Door cursor (no)
pub const RHMOUSE_DOOR_NO: i32 = 271;
/// Gate cursor (yes)
pub const RHMOUSE_GATE_YES: i32 = 272;
/// Gate cursor (no)
pub const RHMOUSE_GATE_NO: i32 = 273;
/// Foot lever cursor (yes)
pub const RHMOUSE_LEVER_FOOT_YES: i32 = 274;
/// Bow out-of-range cursor
pub const RHMOUSE_BOW_OUT: i32 = 275;
/// NPC interaction cursor
pub const RHMOUSE_INTERRACT_NPC: i32 = 222;
/// PC interaction cursor
pub const RHMOUSE_INTERRACT_PC: i32 = 223;

// ── Minimap ─────────────────────────────────────────────────────────────────

/// Minimap item point types
pub const RHMAP_ITEMS: i32 = 30;
/// Minimap selection corner
pub const RHMAP_CORNER: i32 = 61;

// ── Weather / visual effects ────────────────────────────────────────────────

/// Rain effect sprite
pub const RHID_RAIN: i32 = 31;

// ── Titbits (small visual effects) ──────────────────────────────────────────

/// Water walking effect
pub const RHID_TITBIT_WATER: i32 = 33;
/// Splash effect
pub const RHID_TITBIT_PLOUF: i32 = 200;
/// Apple smell effect
pub const RHID_TITBIT_APPLE_SMELL: i32 = 220;
/// NPC speak icon titbit
pub const RHID_TITBIT_SPEAK: i32 = 221;
/// Danger point indicator
pub const RHID_TITBIT_DANGER_POINT: i32 = 249;
/// Hidden indicator
pub const RHID_TITBIT_HIDDEN: i32 = 250;

// ── Text system ─────────────────────────────────────────────────────────────

/// Table containing all in-game text strings
pub const RHID_TEXT_SYSTEM: i32 = 37;

// ── Confirmation / dialog UI ────────────────────────────────────────────────

/// Confirmation dialog image
pub const RHID_CONFIRMATION_BITMAP: i32 = 38;

// ── Scroll UI ───────────────────────────────────────────────────────────────

/// Top scroll button
pub const RHID_TOP_SCROLL: i32 = 39;
/// Top scroll button alternate (gauge effect)
pub const RHID_TOP_SCROLL_ALTERNATE: i32 = 40;
/// Bottom scroll button
pub const RHID_BOTTOM_SCROLL: i32 = 41;

// ── View angle selector ─────────────────────────────────────────────────────

/// View angle selection button
pub const RHID_SIGHT: i32 = 60;

// ── Ground indicators ───────────────────────────────────────────────────────

/// Movement destination indicator
pub const RHID_GROUND_FOCUS: i32 = 68;
/// Selected character in combat indicator
pub const RHID_GROUND_SELECT_SWORD: i32 = 69;
/// Mouse trail effect
pub const RHID_MOUSE_TRAIL: i32 = 70;

// ── Corner / border images ──────────────────────────────────────────────────

/// Top-left corner of game surface
pub const RHID_TOP_LEFT_CORNER: i32 = 46;
/// Top-right corner of game surface
pub const RHID_TOP_RIGHT_CORNER: i32 = 47;
/// Bottom-left corner
pub const RHID_BOTTOM_LEFT_CORNER: i32 = 48;
/// Bottom-right corner
pub const RHID_BOTTOM_RIGHT_CORNER: i32 = 49;
/// Center border image (800x600)
pub const RHID_MIDDLE_800: i32 = 50;
/// Center border image (1024x768)
pub const RHID_MIDDLE_1024: i32 = 51;

// ── Character portraits ─────────────────────────────────────────────────────

/// Little John portrait
pub const RHID_PORTRAIT_LITTLE_JOHN: i32 = 42;
/// Robin Hood portrait
pub const RHID_PORTRAIT_ROBIN_HOOD: i32 = 71;
/// Friar Tuck portrait
pub const RHID_PORTRAIT_FRIAR_TUCK: i32 = 72;
/// Lady Marian portrait
pub const RHID_PORTRAIT_LADY_MARIAN: i32 = 73;
/// Stuteley portrait
pub const RHID_PORTRAIT_STUTELEY: i32 = 74;
/// Will Scarlet portrait
pub const RHID_PORTRAIT_WILL_SCARLET: i32 = 75;
/// Merry Men A portrait
pub const RHID_PORTRAIT_MERRYMEN_A: i32 = 113;
/// Merry Men B portrait
pub const RHID_PORTRAIT_MERRYMEN_B: i32 = 114;
/// Merry Men C portrait
pub const RHID_PORTRAIT_MERRYMEN_C: i32 = 115;
/// Portrait scroll left button
pub const RHID_PORTRAIT_SCROLL_LEFT: i32 = 184;
/// Portrait scroll right button
pub const RHID_PORTRAIT_SCROLL_RIGHT: i32 = 185;

// ── Character actions ───────────────────────────────────────────────────────

// Little John actions
pub const RHID_LJ_ACTION_1: i32 = 43;
pub const RHID_LJ_ACTION_2: i32 = 44;
pub const RHID_LJ_ACTION_3: i32 = 45;

// Robin Hood actions
pub const RHID_RH_ACTION_1: i32 = 76;
pub const RHID_RH_ACTION_2: i32 = 77;
pub const RHID_RH_ACTION_3: i32 = 78;

// Friar Tuck actions
pub const RHID_FT_ACTION_1: i32 = 79;
pub const RHID_FT_ACTION_2: i32 = 80;
pub const RHID_FT_ACTION_3: i32 = 81;

// Lady Marian actions
pub const RHID_LM_ACTION_1: i32 = 82;
pub const RHID_LM_ACTION_2: i32 = 83;
pub const RHID_LM_ACTION_3: i32 = 84;

// Stuteley actions
pub const RHID_ST_ACTION_1: i32 = 85;
pub const RHID_ST_ACTION_2: i32 = 86;
pub const RHID_ST_ACTION_3: i32 = 87;

// Will Scarlet actions
pub const RHID_WS_ACTION_1: i32 = 88;
pub const RHID_WS_ACTION_2: i32 = 89;
pub const RHID_WS_ACTION_3: i32 = 90;

// Merry Men A actions
pub const RHID_MA_ACTION_1: i32 = 192;
pub const RHID_MA_ACTION_2: i32 = 193;

// Merry Men C actions
pub const RHID_MC_ACTION_1: i32 = 194;
pub const RHID_MC_ACTION_2: i32 = 195;

// Merry Men B actions
pub const RHID_MB_ACTION_1: i32 = 116;
pub const RHID_MB_ACTION_2: i32 = 117;

// ── Emoticons ───────────────────────────────────────────────────────────────

pub const RHID_EMOTICONS_WHAT1: i32 = 54;
pub const RHID_EMOTICONS_WHAT2: i32 = 55;
pub const RHID_EMOTICONS_ACH: i32 = 56;
pub const RHIDEMOTICONS_ZZZ: i32 = 57;
pub const RHID_EMOTICONS_KO: i32 = 118;
pub const RHID_EMOTICONS_ANGRY: i32 = 119;
pub const RHID_EMOTICONS_DISAPPOINTED: i32 = 120;
/// "Burp!"
pub const RHID_EMOTICONS_DRUNKEN: i32 = 121;
pub const RHID_EMOTICONS_HAPPY: i32 = 122;

// ── Campaign map ────────────────────────────────────────────────────────────

/// Campaign map background
pub const RHID_CAMPAIGN_MAP: i32 = 123;
/// Campaign map close button
pub const RHID_CAMPAIGN_MAP_CLOSE: i32 = 124;
/// York castle button
pub const RHID_YORK: i32 = 125;
/// Lincoln castle button
pub const RHID_LINCOLN: i32 = 126;
/// Nottingham castle button
pub const RHID_NOTTINGHAM: i32 = 127;
/// Derby castle button
pub const RHID_DERBY: i32 = 128;
/// Leicester castle button
pub const RHID_LEICESTER: i32 = 129;
/// Road cross button 1
pub const RHID_CROSS_1: i32 = 130;
/// Road cross button 2
pub const RHID_CROSS_2: i32 = 131;
/// Road cross button 3
pub const RHID_CROSS_3: i32 = 132;
/// Short mission description background
pub const RHID_SHORT_MISSION_DESCRIPTION: i32 = 133;
/// Display campaign map button
pub const RHID_DISPLAY_CAMPAIGN_MAP: i32 = 251;

// ── Blazons ─────────────────────────────────────────────────────────────────

/// Large blazon picture set
pub const RHID_BLAZON_HUGE: i32 = 134;
/// Small blazon picture set
pub const RHID_BLAZON_TINY: i32 = 135;
/// Buy blazons button
pub const RHID_BUY_BLAZONS_BUTTON: i32 = 144;
/// Convert money to blazons button
pub const RHID_CONVERT_MONEY_TO_BLAZONS: i32 = 157;
/// Convert peasants to blazons button
pub const RHID_CONVERT_PEASANTS_TO_BLAZONS: i32 = 158;
/// Convert mission to blazons button
pub const RHID_CONVERT_MISSION_TO_BLAZONS: i32 = 159;
/// Start mission for blazons button
pub const RHID_START_MISSION_FOR_BLAZONS: i32 = 160;
/// Single tiny blazon
pub const RHID_MINI_BLAZON: i32 = 238;
/// Single big blazon
pub const RHID_MAXI_BLAZON: i32 = 239;
/// Richard's flag for blazon missions
pub const RHID_RICHARD_FLAG: i32 = 225;

// ── Required characters / actions ───────────────────────────────────────────

pub const RHID_RH_REQUIRED: i32 = 136;
pub const RHID_FT_REQUIRED: i32 = 137;
pub const RHID_LJ_REQUIRED: i32 = 138;
pub const RHID_LM_REQUIRED: i32 = 139;
pub const RHID_ST_REQUIRED: i32 = 140;
pub const RHID_WS_REQUIRED: i32 = 141;
pub const RHID_CARRY_REQUIRED: i32 = 148;
pub const RHID_BOW_REQUIRED: i32 = 149;
pub const RHID_JUMP_REQUIRED: i32 = 150;
pub const RHID_CLIMB_REQUIRED: i32 = 151;
pub const RHID_TIE_REQUIRED: i32 = 152;
pub const RHID_LEVER_REQUIRED: i32 = 153;
pub const RHID_LOCKPICK_REQUIRED: i32 = 154;
pub const RHID_STUN_REQUIRED: i32 = 155;
/// Picture collection for required PCs
pub const RHID_REQUIRED_PC: i32 = 242;
/// All required actions
pub const RHID_REQUIRED_ACTION: i32 = 243;
/// All optional PCs
pub const RHID_OPTIONAL_PC: i32 = 244;

// ── Generic UI buttons ──────────────────────────────────────────────────────

pub const RHID_YES_NO: i32 = 142;
/// Generic OK button
pub const RHID_OK: i32 = 145;
/// Generic cancel button
pub const RHID_CANCEL: i32 = 146;
/// Floating OK button
pub const RHID_FLOATING_OK: i32 = 281;
/// Floating cancel button
pub const RHID_FLOATING_CANCEL: i32 = 282;
/// Horizontal separator for short briefings
pub const RHID_SEPARATOR: i32 = 283;

// ── Parchments / pictures ───────────────────────────────────────────────────

/// Large parchment picture
pub const RHID_PARCHMENT_HUGE: i32 = 147;
/// Small parchment picture
pub const RHID_PARCHMENT_TINY: i32 = 162;
/// Small picture frame
pub const RHID_PICTURE_FRAME: i32 = 156;
/// Missing file placeholder
pub const RHID_MISSING_FILE: i32 = 161;
/// Default popup scroll text
pub const RHID_DEFAULT_POPUP_SCROLL_TEXT: i32 = 163;
/// Default popup scroll picture
pub const RHID_DEFAULT_POPUP_SCROLL_PICTURE: i32 = 164;
/// Button with clover
pub const RHID_CLOVER: i32 = 165;

// ── Menu UI ─────────────────────────────────────────────────────────────────

pub const RHID_MENU_BACKGROUND_0: i32 = 186;
pub const RHID_MENU_BACKGROUND_1: i32 = 187;
pub const RHID_MENU_BACKGROUND_2: i32 = 188;
pub const RHID_MENU_BACKGROUND_3: i32 = 189;
pub const RHID_MENU_BUTTON: i32 = 190;
pub const RHID_MENU_INPUT_FIELD: i32 = 191;
pub const RHID_MENU_LIST_BOX: i32 = 201;
pub const RHID_MENU_BACKGROUND_SMALL: i32 = 237;
/// Slider widget
pub const RHID_SLIDER: i32 = 210;
/// Generic radio button
pub const RHID_RADIO: i32 = 211;

// ── Ghost / special character sprites ───────────────────────────────────────

/// Ghost Little John (short legs)
pub const RHID_GHOST_LITTLE_JOHN_SHORT_LEGS: i32 = 208;

// ── Guard ───────────────────────────────────────────────────────────────────

/// Guard portrait button
pub const RHID_GUARD: i32 = 209;

// ── Info popup ──────────────────────────────────────────────────────────────

/// Bow icon for PC info popup
pub const RHID_INFO_POPUP_BOW: i32 = 216;
/// Sword icon for PC info popup
pub const RHID_INFO_POPUP_SWORD: i32 = 217;
/// Small PC info popup background
pub const RHID_INFO_POPUP_BKGND_TINY: i32 = 218;
/// Large PC info popup background
pub const RHID_INFO_POPUP_BKGND_HUGE: i32 = 219;

// ── Trumpet / events ────────────────────────────────────────────────────────

pub const RHID_TRUMPET: i32 = 224;

// ── Attack arrows ───────────────────────────────────────────────────────────

pub const RHID_ATTACK_0: i32 = 235;
pub const RHID_ATTACK_1: i32 = 227;
pub const RHID_ATTACK_2: i32 = 228;
pub const RHID_ATTACK_3: i32 = 229;
pub const RHID_ATTACK_4: i32 = 230;
pub const RHID_ATTACK_5: i32 = 231;
pub const RHID_ATTACK_6: i32 = 232;
pub const RHID_ATTACK_7: i32 = 233;
pub const RHID_ATTACK_8: i32 = 234;
pub const RHID_ATTACK_9: i32 = 236;

// ── Selection / navigation ──────────────────────────────────────────────────

/// Select all PCs in Sherwood button
pub const RHID_SELECT_ALL: i32 = 240;
/// Go to exit button
pub const RHID_GO_TO_EXIT: i32 = 241;
/// Selected action indicator
pub const RHID_SELECTED_ACTION: i32 = 276;

// ── Dialogue sequence portraits ─────────────────────────────────────────────

pub const RHID_DLG_ALLAN: i32 = 252;
pub const RHID_DLG_GODWIN: i32 = 253;
pub const RHID_DLG_RANULPH: i32 = 254;
pub const RHID_DLG_SOLDIER: i32 = 255;
pub const RHID_DLG_LITTLE_JOHN: i32 = 256;
pub const RHID_DLG_MARIANNE: i32 = 257;
pub const RHID_DLG_ROBIN: i32 = 258;
pub const RHID_DLG_SCARLET: i32 = 259;
pub const RHID_DLG_STUTELEY: i32 = 260;
pub const RHID_DLG_TUCK: i32 = 261;
pub const RHID_DLG_GUISBOURNE: i32 = 262;
pub const RHID_DLG_LONGCHAMP: i32 = 263;
pub const RHID_DLG_PRINCE_JOHN: i32 = 264;
pub const RHID_DLG_SCATHLOCK: i32 = 265;
pub const RHID_DLG_SHERIF: i32 = 266;
pub const RHID_DLG_BAD_PORTRAIT: i32 = 267;

// ── Mission end UI ──────────────────────────────────────────────────────────

/// Reload mission button (lose screen)
pub const RHID_LOAD: i32 = 277;
/// Restart mission button (lose screen)
pub const RHID_RESTART: i32 = 278;
/// Mission lifetime illustration
pub const RHID_MISSION_LIFETIME: i32 = 279;

// ── Sherwood work icons (production/training zones) ─────────────────────────

pub const RHWORKICON_ARROWS: i32 = 284;
pub const RHWORKICON_PURSES: i32 = 285;
pub const RHWORKICON_STONES: i32 = 286;
pub const RHWORKICON_APPLES: i32 = 287;
pub const RHWORKICON_BEER: i32 = 288;
pub const RHWORKICON_LEGS: i32 = 289;
pub const RHWORKICON_PLANTS: i32 = 290;
pub const RHWORKICON_NETS: i32 = 291;
pub const RHWORKICON_WASPS: i32 = 292;
pub const RHWORKICON_BOW_TRAINING: i32 = 293;
pub const RHWORKICON_SWORD_TRAINING: i32 = 294;
pub const RHWORKICON_REGENERATE: i32 = 295;

// ── Intro / outro / credits ─────────────────────────────────────────────────

/// Intro video button
pub const RHID_INTRO: i32 = 297;
/// Outro video button
pub const RHID_OUTRO: i32 = 298;
/// Credits picture
pub const RHID_CREDITS_PICTURE: i32 = 308;
/// Credits background
pub const RHID_BK_CREDITS: i32 = 309;

// ── Fighting portraits ──────────────────────────────────────────────────────

pub const RHID_FIGHTING_ROBIN: i32 = 299;
pub const RHID_FIGHTING_STUTELEY: i32 = 300;
pub const RHID_FIGHTING_SCARLET: i32 = 301;
pub const RHID_FIGHTING_JOHN: i32 = 302;
pub const RHID_FIGHTING_TUCK: i32 = 303;
pub const RHID_FIGHTING_MARIAN: i32 = 304;
pub const RHID_FIGHTING_MERRYMEN_A: i32 = 305;
pub const RHID_FIGHTING_MERRYMEN_B: i32 = 306;
pub const RHID_FIGHTING_MERRYMEN_C: i32 = 307;
