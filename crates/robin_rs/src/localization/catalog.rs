//! Single lookup boundary for port-owned text. Missing locales explicitly use English.
use super::{PortTextKey, feature40_text, locale_primary};
use crate::gameplay_settings::GameplaySetting;

#[test]
fn catalogue_rows_are_unique_and_gameplay_keys_have_explicit_fallbacks() {
    for (index, (key, language, value)) in BASIC.iter().enumerate() {
        assert!(!value.is_empty());
        assert!(
            !BASIC[index + 1..]
                .iter()
                .any(|row| row.0 == *key && row.1 == *language)
        );
    }
    for setting in GameplaySetting::ALL {
        assert_eq!(GAMEPLAY.iter().filter(|row| row.0 == setting).count(), 1);
        for key in [
            PortTextKey::GameplayLabel(setting),
            PortTextKey::GameplayTooltip(setting),
        ] {
            assert!(!text(Some("de-DE"), key).is_empty());
            assert_eq!(text(Some("untranslated"), key), text(Some("en"), key));
        }
    }
}

pub(super) fn text(locale: Option<&str>, key: PortTextKey) -> &'static str {
    let language = locale
        .map(locale_primary)
        .unwrap_or(std::borrow::Cow::Borrowed("en"));
    if let PortTextKey::GameplayLabel(setting) | PortTextKey::GameplayTooltip(setting) = key {
        let (_, en_label, en_help, de_label, de_help) = GAMEPLAY
            .iter()
            .find(|row| row.0 == setting)
            .expect("every gameplay setting has catalogue text");
        let (label, help) = if language == "de" {
            (de_label, de_help)
        } else {
            (en_label, en_help)
        };
        return if matches!(key, PortTextKey::GameplayLabel(_)) {
            label
        } else {
            help
        };
    }
    if let Some(text) = feature40_text::text(locale.unwrap_or("en-US"), key)
        .or_else(|| feature40_text::text("en-US", key))
    {
        return text;
    }
    BASIC
        .iter()
        .find(|row| row.0 == key && row.1 == language)
        .or_else(|| BASIC.iter().find(|row| row.0 == key && row.1 == "en"))
        .expect("every port-owned key has English catalogue text")
        .2
}

const BASIC: &[(PortTextKey, &str, &str)] = &[
    (PortTextKey::CampaignClassicMap, "de", "Klassische Karte"),
    (PortTextKey::CampaignClassicMap, "en", "Classic map"),
    (PortTextKey::CampaignProgressTree, "de", "Fortschrittsbaum"),
    (PortTextKey::CampaignProgressTree, "en", "Progress tree"),
    (PortTextKey::CampaignSherwoodMuseum, "de", "Sherwood-Museum"),
    (PortTextKey::CampaignSherwoodMuseum, "en", "Sherwood museum"),
    (
        PortTextKey::GameAutosaved,
        "de",
        "Spiel automatisch gespeichert.",
    ),
    (PortTextKey::GameAutosaved, "en", "Game autosaved."),
    (
        PortTextKey::AutosaveFailed,
        "de",
        "Automatisches Speichern fehlgeschlagen – siehe Protokoll.",
    ),
    (
        PortTextKey::AutosaveFailed,
        "en",
        "Autosave failed - check the log.",
    ),
    (
        PortTextKey::SaveFailed,
        "de",
        "Speichern fehlgeschlagen – vor erneutem Versuch das Protokoll prüfen.",
    ),
    (
        PortTextKey::SaveFailed,
        "en",
        "Save failed - check the log before retrying.",
    ),
    (PortTextKey::Language, "de", "Sprache"),
    (PortTextKey::Automatic, "de", "Automatisch"),
    (PortTextKey::Apply, "de", "Anwenden"),
    (PortTextKey::Language, "fr", "Langue"),
    (PortTextKey::Automatic, "fr", "Automatique"),
    (PortTextKey::Apply, "fr", "Appliquer"),
    (PortTextKey::Language, "it", "Lingua"),
    (PortTextKey::Automatic, "it", "Automatico"),
    (PortTextKey::Apply, "it", "Applica"),
    (PortTextKey::Language, "pt", "Idioma"),
    (PortTextKey::Language, "es", "Idioma"),
    (PortTextKey::Automatic, "pt", "Automático"),
    (PortTextKey::Automatic, "es", "Automático"),
    (PortTextKey::Apply, "pt", "Aplicar"),
    (PortTextKey::Apply, "es", "Aplicar"),
    (PortTextKey::Language, "ru", "Язык"),
    (PortTextKey::Automatic, "ru", "Автоматически"),
    (PortTextKey::Apply, "ru", "Применить"),
    (PortTextKey::Language, "ja", "言語"),
    (PortTextKey::Automatic, "ja", "自動"),
    (PortTextKey::Apply, "ja", "適用"),
    (PortTextKey::Language, "cs", "Jazyk"),
    (PortTextKey::Automatic, "cs", "Automaticky"),
    (PortTextKey::Apply, "cs", "Použít"),
    (PortTextKey::Language, "pl", "Język"),
    (PortTextKey::Automatic, "pl", "Automatycznie"),
    (PortTextKey::Apply, "pl", "Zastosuj"),
    (PortTextKey::Language, "zh", "語言"),
    (PortTextKey::Automatic, "zh", "自動"),
    (PortTextKey::Apply, "zh", "套用"),
    (PortTextKey::Language, "ko", "언어"),
    (PortTextKey::Automatic, "ko", "자동"),
    (PortTextKey::Apply, "ko", "적용"),
    (PortTextKey::Language, "th", "ภาษา"),
    (PortTextKey::Automatic, "th", "อัตโนมัติ"),
    (PortTextKey::Apply, "th", "ใช้"),
    (PortTextKey::Language, "en", "Language"),
    (PortTextKey::Automatic, "en", "Automatic"),
    (PortTextKey::Apply, "en", "Apply"),
    (PortTextKey::InstalledLanguages, "en", "Installed languages"),
    (
        PortTextKey::OptionalEnglishFallback,
        "en",
        "Missing optional voice or cinematics use the installed English pack",
    ),
];
const GAMEPLAY: [(GameplaySetting, &str, &str, &str, &str); GameplaySetting::ALL.len()] = [
    (
        GameplaySetting::FixHardReactionTimes,
        "Fix Hard Reaction Times",
        "Use the intended Hard reaction-time multiplier.",
        "Reaktionszeiten auf Schwer korrigieren",
        "Den vorgesehenen Reaktionszeitfaktor für Schwer verwenden.",
    ),
    (
        GameplaySetting::ControlTacticalUnits,
        "Control Tactical Units",
        "Allow high-level commands for actors authored with the tactical command interface.",
        "Taktische Einheiten steuern",
        "Übergeordnete Befehle für Figuren mit taktischer Befehlsschnittstelle erlauben.",
    ),
    (
        GameplaySetting::EnableUnbinding,
        "Allow Untying NPCs",
        "Allow a hero with Tie to release a tied NPC.",
        "Gefesselte Personen befreien",
        "Helden mit Fesseln dürfen gefesselte Personen befreien.",
    ),
    (
        GameplaySetting::ShowProductionForecast,
        "Sherwood Production Forecast",
        "Show live item-production forecasts in Sherwood.",
        "Produktionsvorschau in Sherwood",
        "Die laufende Gegenstandsproduktion in Sherwood anzeigen.",
    ),
    (
        GameplaySetting::ReusableCloaks,
        "Reusable Cloaks",
        "Allow heroes with shipped cape art to put their cloaks back on.",
        "Umhänge wiederverwenden",
        "Helden mit vorhandenen Umhanggrafiken dürfen ihren Umhang wieder anlegen.",
    ),
    (
        GameplaySetting::CampaignPresentation,
        "Campaign Presentation",
        "Cycle the campaign-map presentation.",
        "Kampagnendarstellung",
        "Die Darstellung der Kampagnenkarte wechseln.",
    ),
    (
        GameplaySetting::CleanHandsNpcKillsInvalidate,
        "NPC Kills Break Clean Hands",
        "Count hostile deaths caused by other NPCs against Clean Hands.",
        "NPC-Tötungen verhindern Saubere Hände",
        "Auch von anderen NPCs getötete Feinde für Saubere Hände zählen.",
    ),
    (
        GameplaySetting::ShowDetailedXp,
        "Detailed Sword/Bow XP",
        "Show detailed sword and bow experience progress.",
        "Detaillierte Schwert-/Bogen-Erfahrung",
        "Den genauen Erfahrungsfortschritt mit Schwert und Bogen anzeigen.",
    ),
    (
        GameplaySetting::ShowSpeedrunTracker,
        "Speedrun Clock",
        "Show the current mission speedrun clock.",
        "Speedrun-Uhr",
        "Die Speedrun-Zeit der aktuellen Mission anzeigen.",
    ),
    (
        GameplaySetting::ShowCleanHandsTracker,
        "Clean Hands Tracker",
        "Show live Clean Hands achievement progress.",
        "Fortschritt: Saubere Hände",
        "Den aktuellen Fortschritt für Saubere Hände anzeigen.",
    ),
    (
        GameplaySetting::ShowGhostTracker,
        "Ghost Tracker",
        "Show live Ghost achievement progress.",
        "Fortschritt: Geist",
        "Den aktuellen Fortschritt für Geist anzeigen.",
    ),
    (
        GameplaySetting::ShowPileOfBonesTracker,
        "Pile-o-Bones Tracker",
        "Show live Pile-o-Bones achievement progress.",
        "Fortschritt: Knochenhaufen",
        "Den aktuellen Fortschritt für Knochenhaufen anzeigen.",
    ),
    (
        GameplaySetting::ShowNewAchievementTrackers,
        "Additional Achievement Trackers",
        "Show live progress for the additional mission achievements.",
        "Weitere Erfolgsanzeigen",
        "Den aktuellen Fortschritt weiterer Missionserfolge anzeigen.",
    ),
    (
        GameplaySetting::ShowAchievementBadges,
        "Campaign Achievement Badges",
        "Show achievement badges in campaign presentations.",
        "Erfolgsabzeichen der Kampagne",
        "Erfolgsabzeichen in den Kampagnendarstellungen anzeigen.",
    ),
    (
        GameplaySetting::ShowAchievementDebrief,
        "Achievement Debrief Details",
        "Include achievement details in mission debriefs.",
        "Erfolge im Missionsbericht",
        "Erfolgsdetails im Missionsbericht anzeigen.",
    ),
    (
        GameplaySetting::TouchCameraGestures,
        "Touch Camera Gestures",
        "Enable one-finger camera panning, anchored pinch zoom, and touch inertia.",
        "Touch-Kameragesten",
        "Kameraschwenken mit einem Finger, verankerten Pinch-Zoom und Nachlauf aktivieren.",
    ),
    (
        GameplaySetting::SherwoodTrading,
        "Sherwood Item Trading",
        "Allow the host to sell Sherwood production inventory for campaign ransom.",
        "Handel in Sherwood",
        "Der Host darf Sherwood-Produkte für das Lösegeld der Kampagne verkaufen.",
    ),
    (
        GameplaySetting::AutosaveEnabled,
        "Rotating Autosaves",
        "Keep rotating campaign and mission autosaves according to the autosave policy.",
        "Rotierende automatische Spielstände",
        "Kampagnen- und Missionsspielstände gemäß den automatischen Speicherregeln rotieren.",
    ),
    (
        GameplaySetting::AppleCombatInterrupt,
        "Apple Combat Interrupt",
        "Let direct apple hits interrupt active swordfights.",
        "Äpfel unterbrechen Kämpfe",
        "Direkte Apfeltreffer dürfen laufende Schwertkämpfe unterbrechen.",
    ),
    (
        GameplaySetting::WaspReliableAcquisition,
        "Reliable Wasp Acquisition",
        "Increase initial wasp acquisition from 50 to 75 world units.",
        "Zuverlässigere Wespensuche",
        "Die anfängliche Suchreichweite der Wespen von 50 auf 75 Welteinheiten erhöhen.",
    ),
    (
        GameplaySetting::StoneGroundDistraction,
        "Stone Ground Distraction",
        "Allow ground-thrown stones to attract eligible hostiles within 240 world units.",
        "Ablenkung durch Bodensteine",
        "Auf den Boden geworfene Steine dürfen geeignete Feinde innerhalb von 240 Welteinheiten anlocken.",
    ),
    (
        GameplaySetting::StoneLongerRange,
        "Longer Stone Range",
        "Use base range 300 for stones instead of the shipped 200.",
        "Größere Steinreichweite",
        "Die Grundreichweite für Steine von 200 auf 300 erhöhen.",
    ),
    (
        GameplaySetting::NetSelectiveImmunity,
        "Selective Net Immunity",
        "Skip VIPs, riders, and Stuteley while catching other people in the net circle.",
        "Gezielte Netz-Ausnahmen",
        "VIPs, Reiter und Stuteley beim Einfangen anderer Personen im Netzbereich überspringen.",
    ),
    (
        GameplaySetting::AleReliableDistraction,
        "Reliable Ale Distraction",
        "Let outdoor non-VIP soldiers with no beer interest accept ale at potency 20.",
        "Zuverlässigere Bierablenkung",
        "Soldaten im Freien, die keine VIPs sind, dürfen bei Stärke 20 auch ohne Bierinteresse Bier annehmen.",
    ),
    (
        GameplaySetting::NoiseDistractionFeedback,
        "Stone Distraction Feedback",
        "Play the optional impact cue for a ground-thrown stone distraction.",
        "Rückmeldung bei Steinablenkung",
        "Das optionale Aufprallgeräusch für einen zur Ablenkung geworfenen Bodenstein abspielen.",
    ),
    (
        GameplaySetting::PreviewAppleEffect,
        "Preview Apple Effect",
        "Explain apple daze, scent, and combat-interrupt eligibility while aiming.",
        "Apfelwirkung vorhersagen",
        "Beim Zielen Benommenheit, Geruch und mögliche Kampfunterbrechung erklären.",
    ),
    (
        GameplaySetting::PreviewStoneDirectEffect,
        "Preview Stone Direct Hit",
        "Explain stone direct-hit damage and concussion while aiming.",
        "Direkten Steintreffer vorhersagen",
        "Beim Zielen Schaden und Benommenheit durch direkte Steintreffer erklären.",
    ),
    (
        GameplaySetting::PreviewStoneDistractionArea,
        "Preview Stone Noise Area",
        "Show the 240-unit ground-stone distraction area.",
        "Stein-Lärmbereich anzeigen",
        "Den Ablenkungsbereich von 240 Welteinheiten für Bodensteine anzeigen.",
    ),
    (
        GameplaySetting::PreviewNetCaptureArea,
        "Preview Net Capture Area",
        "Show the original 40-unit net capture area and friendly-capture behavior.",
        "Netzfangbereich anzeigen",
        "Den ursprünglichen Netzfangbereich von 40 Welteinheiten und das Einfangen von Verbündeten anzeigen.",
    ),
    (
        GameplaySetting::PreviewNetCrumplePrediction,
        "Predict Net Crumpling",
        "Predict victim and terrain conditions that crumple a net.",
        "Zusammenfallendes Netz vorhersagen",
        "Opfer- und Geländebedingungen vorhersagen, die ein Netz zusammenfallen lassen.",
    ),
    (
        GameplaySetting::PreviewAleEffect,
        "Preview Ale Effect",
        "Explain visibility, outdoor, drunkenness, and beer-interest conditions.",
        "Bierwirkung vorhersagen",
        "Bedingungen für Sichtbarkeit, Aufenthalt im Freien, Trunkenheit und Bierinteresse erklären.",
    ),
    (
        GameplaySetting::PreviewPurseEffect,
        "Preview Purse Effect",
        "Explain purse value and money-interest conditions.",
        "Geldbeutelwirkung vorhersagen",
        "Geldbeutelwert und Bedingungen für Geldinteresse erklären.",
    ),
    (
        GameplaySetting::PreviewWaspArea,
        "Preview Wasp Area",
        "Show wasp acquisition range and target eligibility.",
        "Wespenbereich anzeigen",
        "Suchreichweite und geeignete Ziele der Wespen anzeigen.",
    ),
    (
        GameplaySetting::DetailedSaveMetadata,
        "Detailed Save Metadata",
        "Show mission and player provenance, relative age, and expanded save details.",
        "Detaillierte Spielstanddaten",
        "Mission, Spieler, relatives Alter und weitere Spielstanddetails anzeigen.",
    ),
    (
        GameplaySetting::EnableTimedMissions,
        "Authored Mission Timers",
        "Enforce time limits authored by Rust JSON missions.",
        "Vorgegebene Missionszeitlimits",
        "Zeitlimits aus Rust-JSON-Missionen durchsetzen.",
    ),
    (
        GameplaySetting::EnableDynamicAmbience,
        "Dynamic Ambience Gameplay",
        "Advance authored day, night, and fog gameplay schedules.",
        "Dynamische Umgebungsabläufe",
        "Vorgegebene Tag-, Nacht- und Nebelabläufe im Spiel fortschreiten lassen.",
    ),
    (
        GameplaySetting::Diplomacy,
        "Mission Diplomacy",
        "Enable mission-authored and runtime faction relationships.",
        "Missionsdiplomatie",
        "Vorgegebene und während des Spiels geänderte Fraktionsbeziehungen aktivieren.",
    ),
    (
        GameplaySetting::NpcFactionWars,
        "NPC Faction Wars",
        "Allow hostile non-player factions to perceive and fight one another.",
        "Kriege zwischen NPC-Fraktionen",
        "Feindliche Nichtspielerfraktionen dürfen einander wahrnehmen und bekämpfen.",
    ),
    (
        GameplaySetting::MoreCombatGestures,
        "More Combat Gestures",
        "Recognize nine additional sword gestures as composite two-strike techniques.",
        "Weitere Kampfgesten",
        "Neun zusätzliche Schwertgesten als kombinierte Zweischlagtechniken erkennen.",
    ),
    (
        GameplaySetting::GestureQualityDamage,
        "Gesture Quality Damage",
        "Scale sword damage to the recognized gesture's deterministic quality tier.",
        "Gestenqualität beeinflusst Schaden",
        "Schwertschaden gemäß der deterministischen Qualitätsstufe der erkannten Geste skalieren.",
    ),
    (
        GameplaySetting::ShowCombatGestureGuide,
        "Show Combat Gesture Guide",
        "Show reference paths for the additional combat gestures while swordfighting.",
        "Kampfgesten-Vorlagen anzeigen",
        "Im Schwertkampf Vorlagen für die zusätzlichen Kampfgesten anzeigen.",
    ),
    (
        GameplaySetting::CombatGestureCoach,
        "Combat Gesture Coach",
        "Briefly show the recognized or nearest gesture and its quality after drawing.",
        "Kampfgesten-Trainer",
        "Nach dem Zeichnen kurz die erkannte oder ähnlichste Geste und ihre Qualität anzeigen.",
    ),
    (
        GameplaySetting::PlanQuickActions,
        "Plan Quick Actions",
        "Allow the rebindable Plan modifier and touch HUD to queue quick actions.",
        "Schnellaktionen planen",
        "Mit der frei belegbaren Planen-Taste und dem Touch-HUD Schnellaktionen einreihen.",
    ),
    (
        GameplaySetting::FogOfWar,
        "Fog of War",
        "Enable shared allied sight, explored terrain, and temporary hostile intelligence.",
        "Nebel des Krieges",
        "Gemeinsame Sicht von Verbündeten, erkundetes Gelände und vorübergehende Feindinformationen aktivieren.",
    ),
    (
        GameplaySetting::EnableSpellforgeMissions,
        "Allow Spellforge Missions (Next Launch)",
        "Allow executable Spellforge custom missions. This takes effect on the next mission launch.",
        "Spellforge-Missionen erlauben (nächster Start)",
        "Ausführbare Spellforge-Missionen beim nächsten Missionsstart erlauben.",
    ),
    (
        GameplaySetting::ReversibleBackgroundPatches,
        "Reversible Background Patches (Next Launch)",
        "Repeat animated terrain triggers to reverse their animation, obstacles and doors. Applies to newly launched missions; off preserves original one-shot patches.",
        "Umkehrbare Hintergrundänderungen (nächster Start)",
        "Animierte Geländeauslöser wiederholen, um Animation, Hindernisse und Türen umzukehren. Gilt für neu gestartete Missionen; ausgeschaltet bleibt das ursprüngliche einmalige Verhalten.",
    ),
];
