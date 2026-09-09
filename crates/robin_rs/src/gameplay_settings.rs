//! Stable gameplay-row identities and keyed presentation metadata.
//! Persisted GameplayConfig fields and simulation mutation remain unchanged.
use crate::localization::PortTextKey;
use serde::{Deserialize, Serialize};

// One declaration defines identity, order and required text. Labels and help
// cannot drift into different positional tables when a row is added.
macro_rules! settings {
    ($($name:ident => ($label:literal, $help:literal, $de_label:literal, $de_help:literal)),* $(,)?) => {
        #[repr(usize)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum GameplaySetting { $($name),* }
        impl GameplaySetting {
            pub const ALL: [Self; [$(stringify!($name)),*].len()] = [$(Self::$name),*];
            pub const fn index(self) -> usize { self as usize }
            pub fn from_index(index: usize) -> Option<Self> { Self::ALL.get(index).copied() }
            pub(crate) fn text(self, language: &str) -> Option<(&'static str, &'static str)> {
                match language {
                    "en" => Some(match self { $(Self::$name => ($label, $help)),* }),
                    "de" => Some(match self { $(Self::$name => ($de_label, $de_help)),* }),
                    // Missing translations use the localization service's
                    // explicit English fallback, never a UI-specific table.
                    _ => None,
                }
            }
            pub(crate) const fn label_key(self) -> PortTextKey {
                match self {
                    Self::EnableSpellforgeMissions => PortTextKey::SpellforgeGameplayAllowLabel,
                    _ => PortTextKey::GameplayLabel(self),
                }
            }
            pub(crate) const fn tooltip_key(self) -> PortTextKey {
                match self {
                    Self::EnableSpellforgeMissions => PortTextKey::SpellforgeGameplayAllowTooltip,
                    _ => PortTextKey::GameplayTooltip(self),
                }
            }
        }
    };
}
settings! {
    FixHardReactionTimes => ("Fix Hard Reaction Times", "Use the intended Hard reaction-time multiplier.", "Reaktionszeiten auf Schwer korrigieren", "Den vorgesehenen Reaktionszeitfaktor für Schwer verwenden."),
    ControlTacticalUnits => ("Control Tactical Units", "Allow high-level commands for actors authored with the tactical command interface.", "Taktische Einheiten steuern", "Übergeordnete Befehle für Figuren mit taktischer Befehlsschnittstelle erlauben."),
    EnableUnbinding => ("Allow Untying NPCs", "Allow a hero with Tie to release a tied NPC.", "Gefesselte Personen befreien", "Helden mit Fesseln dürfen gefesselte Personen befreien."),
    ShowProductionForecast => ("Sherwood Production Forecast", "Show live item-production forecasts in Sherwood.", "Produktionsvorschau in Sherwood", "Die laufende Gegenstandsproduktion in Sherwood anzeigen."),
    ReusableCloaks => ("Reusable Cloaks", "Allow heroes with shipped cape art to put their cloaks back on.", "Umhänge wiederverwenden", "Helden mit vorhandenen Umhanggrafiken dürfen ihren Umhang wieder anlegen."),
    CampaignPresentation => ("Campaign Presentation", "Cycle the campaign-map presentation.", "Kampagnendarstellung", "Die Darstellung der Kampagnenkarte wechseln."),
    CleanHandsNpcKillsInvalidate => ("NPC Kills Break Clean Hands", "Count hostile deaths caused by other NPCs against Clean Hands.", "NPC-Tötungen verhindern Saubere Hände", "Auch von anderen NPCs getötete Feinde für Saubere Hände zählen."),
    ShowDetailedXp => ("Detailed Sword/Bow XP", "Show detailed sword and bow experience progress.", "Detaillierte Schwert-/Bogen-Erfahrung", "Den genauen Erfahrungsfortschritt mit Schwert und Bogen anzeigen."),
    ShowSpeedrunTracker => ("Speedrun Clock", "Show the current mission speedrun clock.", "Speedrun-Uhr", "Die Speedrun-Zeit der aktuellen Mission anzeigen."),
    ShowCleanHandsTracker => ("Clean Hands Tracker", "Show live Clean Hands achievement progress.", "Fortschritt: Saubere Hände", "Den aktuellen Fortschritt für Saubere Hände anzeigen."),
    ShowGhostTracker => ("Ghost Tracker", "Show live Ghost achievement progress.", "Fortschritt: Geist", "Den aktuellen Fortschritt für Geist anzeigen."),
    ShowPileOfBonesTracker => ("Pile-o-Bones Tracker", "Show live Pile-o-Bones achievement progress.", "Fortschritt: Knochenhaufen", "Den aktuellen Fortschritt für Knochenhaufen anzeigen."),
    ShowNewAchievementTrackers => ("Additional Achievement Trackers", "Show live progress for the additional mission achievements.", "Weitere Erfolgsanzeigen", "Den aktuellen Fortschritt weiterer Missionserfolge anzeigen."),
    ShowAchievementBadges => ("Campaign Achievement Badges", "Show achievement badges in campaign presentations.", "Erfolgsabzeichen der Kampagne", "Erfolgsabzeichen in den Kampagnendarstellungen anzeigen."),
    ShowAchievementDebrief => ("Achievement Debrief Details", "Include achievement details in mission debriefs.", "Erfolge im Missionsbericht", "Erfolgsdetails im Missionsbericht anzeigen."),
    TouchCameraGestures => ("Touch Camera Gestures", "Enable one-finger camera panning, anchored pinch zoom, and touch inertia.", "Touch-Kameragesten", "Kameraschwenken mit einem Finger, verankerten Pinch-Zoom und Nachlauf aktivieren."),
    SherwoodTrading => ("Sherwood Item Trading", "Allow the host to sell Sherwood production inventory for campaign ransom.", "Handel in Sherwood", "Der Host darf Sherwood-Produkte für das Lösegeld der Kampagne verkaufen."),
    AutosaveEnabled => ("Rotating Autosaves", "Keep rotating campaign and mission autosaves according to the autosave policy.", "Rotierende automatische Spielstände", "Kampagnen- und Missionsspielstände gemäß den automatischen Speicherregeln rotieren."),
    AppleCombatInterrupt => ("Apple Combat Interrupt", "Let direct apple hits interrupt active swordfights.", "Äpfel unterbrechen Kämpfe", "Direkte Apfeltreffer dürfen laufende Schwertkämpfe unterbrechen."),
    WaspReliableAcquisition => ("Reliable Wasp Acquisition", "Increase initial wasp acquisition from 50 to 75 world units.", "Zuverlässigere Wespensuche", "Die anfängliche Suchreichweite der Wespen von 50 auf 75 Welteinheiten erhöhen."),
    StoneGroundDistraction => ("Stone Ground Distraction", "Allow ground-thrown stones to attract eligible hostiles within 240 world units.", "Ablenkung durch Bodensteine", "Auf den Boden geworfene Steine dürfen geeignete Feinde innerhalb von 240 Welteinheiten anlocken."),
    StoneLongerRange => ("Longer Stone Range", "Use base range 300 for stones instead of the shipped 200.", "Größere Steinreichweite", "Die Grundreichweite für Steine von 200 auf 300 erhöhen."),
    NetSelectiveImmunity => ("Selective Net Immunity", "Skip VIPs, riders, and Stuteley while catching other people in the net circle.", "Gezielte Netz-Ausnahmen", "VIPs, Reiter und Stuteley beim Einfangen anderer Personen im Netzbereich überspringen."),
    AleReliableDistraction => ("Reliable Ale Distraction", "Let outdoor non-VIP soldiers with no beer interest accept ale at potency 20.", "Zuverlässigere Bierablenkung", "Soldaten im Freien, die keine VIPs sind, dürfen bei Stärke 20 auch ohne Bierinteresse Bier annehmen."),
    NoiseDistractionFeedback => ("Stone Distraction Feedback", "Play the optional impact cue for a ground-thrown stone distraction.", "Rückmeldung bei Steinablenkung", "Das optionale Aufprallgeräusch für einen zur Ablenkung geworfenen Bodenstein abspielen."),
    PreviewAppleEffect => ("Preview Apple Effect", "Explain apple daze, scent, and combat-interrupt eligibility while aiming.", "Apfelwirkung vorhersagen", "Beim Zielen Benommenheit, Geruch und mögliche Kampfunterbrechung erklären."),
    PreviewStoneDirectEffect => ("Preview Stone Direct Hit", "Explain stone direct-hit damage and concussion while aiming.", "Direkten Steintreffer vorhersagen", "Beim Zielen Schaden und Benommenheit durch direkte Steintreffer erklären."),
    PreviewStoneDistractionArea => ("Preview Stone Noise Area", "Show the 240-unit ground-stone distraction area.", "Stein-Lärmbereich anzeigen", "Den Ablenkungsbereich von 240 Welteinheiten für Bodensteine anzeigen."),
    PreviewNetCaptureArea => ("Preview Net Capture Area", "Show the original 40-unit net capture area and friendly-capture behavior.", "Netzfangbereich anzeigen", "Den ursprünglichen Netzfangbereich von 40 Welteinheiten und das Einfangen von Verbündeten anzeigen."),
    PreviewNetCrumplePrediction => ("Predict Net Crumpling", "Predict victim and terrain conditions that crumple a net.", "Zusammenfallendes Netz vorhersagen", "Opfer- und Geländebedingungen vorhersagen, die ein Netz zusammenfallen lassen."),
    PreviewAleEffect => ("Preview Ale Effect", "Explain visibility, outdoor, drunkenness, and beer-interest conditions.", "Bierwirkung vorhersagen", "Bedingungen für Sichtbarkeit, Aufenthalt im Freien, Trunkenheit und Bierinteresse erklären."),
    PreviewPurseEffect => ("Preview Purse Effect", "Explain purse value and money-interest conditions.", "Geldbeutelwirkung vorhersagen", "Geldbeutelwert und Bedingungen für Geldinteresse erklären."),
    PreviewWaspArea => ("Preview Wasp Area", "Show wasp acquisition range and target eligibility.", "Wespenbereich anzeigen", "Suchreichweite und geeignete Ziele der Wespen anzeigen."),
    DetailedSaveMetadata => ("Detailed Save Metadata", "Show mission and player provenance, relative age, and expanded save details.", "Detaillierte Spielstanddaten", "Mission, Spieler, relatives Alter und weitere Spielstanddetails anzeigen."),
    EnableTimedMissions => ("Authored Mission Timers", "Enforce time limits authored by Rust JSON missions.", "Vorgegebene Missionszeitlimits", "Zeitlimits aus Rust-JSON-Missionen durchsetzen."),
    EnableDynamicAmbience => ("Dynamic Ambience Gameplay", "Advance authored day, night, and fog gameplay schedules.", "Dynamische Umgebungsabläufe", "Vorgegebene Tag-, Nacht- und Nebelabläufe im Spiel fortschreiten lassen."),
    Diplomacy => ("Mission Diplomacy", "Enable mission-authored and runtime faction relationships.", "Missionsdiplomatie", "Vorgegebene und während des Spiels geänderte Fraktionsbeziehungen aktivieren."),
    NpcFactionWars => ("NPC Faction Wars", "Allow hostile non-player factions to perceive and fight one another.", "Kriege zwischen NPC-Fraktionen", "Feindliche Nichtspielerfraktionen dürfen einander wahrnehmen und bekämpfen."),
    MoreCombatGestures => ("More Combat Gestures", "Recognize nine additional sword gestures as composite two-strike techniques.", "Weitere Kampfgesten", "Neun zusätzliche Schwertgesten als kombinierte Zweischlagtechniken erkennen."),
    GestureQualityDamage => ("Gesture Quality Damage", "Scale sword damage to the recognized gesture's deterministic quality tier.", "Gestenqualität beeinflusst Schaden", "Schwertschaden gemäß der deterministischen Qualitätsstufe der erkannten Geste skalieren."),
    ShowCombatGestureGuide => ("Show Combat Gesture Guide", "Show reference paths for the additional combat gestures while swordfighting.", "Kampfgesten-Vorlagen anzeigen", "Im Schwertkampf Vorlagen für die zusätzlichen Kampfgesten anzeigen."),
    CombatGestureCoach => ("Combat Gesture Coach", "Briefly show the recognized or nearest gesture and its quality after drawing.", "Kampfgesten-Trainer", "Nach dem Zeichnen kurz die erkannte oder ähnlichste Geste und ihre Qualität anzeigen."),
    PlanQuickActions => ("Plan Quick Actions", "Allow the rebindable Plan modifier and touch HUD to queue quick actions.", "Schnellaktionen planen", "Mit der frei belegbaren Planen-Taste und dem Touch-HUD Schnellaktionen einreihen."),
    FogOfWar => ("Fog of War", "Enable shared allied sight, explored terrain, and temporary hostile intelligence.", "Nebel des Krieges", "Gemeinsame Sicht von Verbündeten, erkundetes Gelände und vorübergehende Feindinformationen aktivieren."),
    EnableSpellforgeMissions => ("Allow Spellforge Missions (Next Launch)", "Allow executable Spellforge custom missions. This takes effect on the next mission launch.", "Spellforge-Missionen erlauben (nächster Start)", "Ausführbare Spellforge-Missionen beim nächsten Missionsstart erlauben."),
    ReversibleBackgroundPatches => ("Reversible Background Patches (Next Launch)", "Repeat animated terrain triggers to reverse their animation, obstacles and doors. Applies to newly launched missions; off preserves original one-shot patches.", "Umkehrbare Hintergrundänderungen (nächster Start)", "Animierte Geländeauslöser wiederholen, um Animation, Hindernisse und Türen umzukehren. Gilt für neu gestartete Missionen; ausgeschaltet bleibt das ursprüngliche einmalige Verhalten."),
}
