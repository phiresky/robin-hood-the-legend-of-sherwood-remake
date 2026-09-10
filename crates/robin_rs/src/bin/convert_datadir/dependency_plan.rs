//! Inspectable dependency closure, independent of codecs and storage.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DependencyRoot {
    Mission(String),
    Character(u32),
    SavedWorld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedRhs {
    pub asset: String,
    pub profiles: BTreeSet<String>,
    pub roots: BTreeSet<DependencyRoot>,
    pub inclusion_reason: String,
    /// Actual codec grouping is filled after sprite-bank analysis.
    pub grouping: Option<String>,
    pub destination_payloads: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyPlan {
    /// False for the dependency-only checkpoint; true after packaging succeeds.
    pub completed: bool,
    pub rhs: BTreeMap<String, PlannedRhs>,
    pub missions: BTreeMap<String, PlannedMission>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedMission {
    /// Source references retain logical names before codec/package selection.
    pub sources: BTreeMap<String, String>,
    pub destination_payloads: Vec<String>,
}

impl DependencyPlan {
    pub fn include(
        &mut self,
        root: DependencyRoot,
        requirements: &BTreeMap<String, BTreeSet<String>>,
    ) {
        for (asset, profiles) in requirements {
            let entry = self.rhs.entry(asset.clone()).or_insert_with(|| PlannedRhs {
                asset: asset.clone(),
                profiles: BTreeSet::new(),
                roots: BTreeSet::new(),
                inclusion_reason:
                    "reachable RHS profile (including runtime action and saved-object closure)"
                        .to_owned(),
                grouping: None,
                destination_payloads: Vec::new(),
            });
            entry.profiles.extend(profiles.iter().cloned());
            entry.roots.insert(root.clone());
        }
    }

    /// Count distinct mission consumers for family-hub first-load weighting.
    /// Keys are ASCII-lowercased; case aliases and character/save roots must
    /// not increase that weight. Build once before evaluating family candidates.
    pub fn mission_use_counts(&self) -> BTreeMap<String, usize> {
        let mut missions_by_asset = BTreeMap::<String, BTreeSet<&str>>::new();
        for (asset, planned) in &self.rhs {
            let missions = missions_by_asset
                .entry(asset.to_ascii_lowercase())
                .or_default();
            missions.extend(planned.roots.iter().filter_map(|root| match root {
                DependencyRoot::Mission(mission) => Some(mission.as_str()),
                DependencyRoot::Character(_) | DependencyRoot::SavedWorld => None,
            }));
        }
        missions_by_asset
            .into_iter()
            .map(|(asset, missions)| (asset, missions.len()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_weights_match_case_insensitive_root_scans() {
        let mut plan = DependencyPlan::default();
        let assets = [
            "Hero.rhs",
            "HERO.RHS",
            "hero.rhs",
            "Élite.rhs",
            "élite.rhs",
            "save.rhs",
        ];
        for (index, asset) in assets.iter().enumerate() {
            for mission in 0..5 {
                if (index + mission) % 3 != 0 && *asset != "save.rhs" {
                    plan.include(
                        DependencyRoot::Mission(format!("mission-{mission}")),
                        &BTreeMap::from([(asset.to_string(), BTreeSet::new())]),
                    );
                }
            }
            plan.include(
                DependencyRoot::SavedWorld,
                &BTreeMap::from([(asset.to_string(), BTreeSet::new())]),
            );
        }
        let counts = plan.mission_use_counts();
        for asset in assets {
            let expected = plan
                .rhs
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(asset))
                .flat_map(|(_, planned)| planned.roots.iter())
                .filter_map(|root| match root {
                    DependencyRoot::Mission(mission) => Some(mission),
                    _ => None,
                })
                .collect::<BTreeSet<_>>()
                .len();
            assert_eq!(counts[&asset.to_ascii_lowercase()], expected, "{asset}");
        }
        assert_eq!(counts["save.rhs"], 0);
        assert!(!counts.contains_key("absent.rhs"));
        assert!(DependencyPlan::default().mission_use_counts().is_empty());
    }

    #[test]
    fn hub_weight_counts_missions_once_across_case_aliases() {
        let mut plan = DependencyPlan::default();
        for (root, asset) in [
            (DependencyRoot::Mission("A".into()), "Hero.rhs"),
            (DependencyRoot::Mission("A".into()), "hero.rhs"),
            (DependencyRoot::Mission("B".into()), "HERO.RHS"),
            (DependencyRoot::Character(7), "hero.rhs"),
            (DependencyRoot::SavedWorld, "hero.rhs"),
        ] {
            plan.include(root, &BTreeMap::from([(asset.into(), BTreeSet::new())]));
        }
        assert_eq!(
            plan.mission_use_counts(),
            BTreeMap::from([("hero.rhs".into(), 2)])
        );
    }

    #[test]
    fn closure_unions_profiles_without_losing_roots() {
        let mut plan = DependencyPlan::default();
        plan.include(
            DependencyRoot::Mission("A".into()),
            &BTreeMap::from([("hero.rhs".into(), BTreeSet::from(["walk".into()]))]),
        );
        plan.include(
            DependencyRoot::Character(7),
            &BTreeMap::from([("hero.rhs".into(), BTreeSet::from(["run".into()]))]),
        );
        plan.include(DependencyRoot::SavedWorld, &BTreeMap::new());
        assert_eq!(plan.rhs.len(), 1);
        assert_eq!(plan.rhs["hero.rhs"].roots.len(), 2);
        assert_eq!(plan.mission_use_counts()["hero.rhs"], 1);
        assert_eq!(
            plan.rhs["hero.rhs"].profiles,
            BTreeSet::from(["run".into(), "walk".into()])
        );
        assert_eq!(
            serde_json::from_str::<DependencyPlan>(&serde_json::to_string(&plan).unwrap()).unwrap(),
            plan
        );
    }
}
