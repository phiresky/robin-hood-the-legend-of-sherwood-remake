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
    /// Case aliases and character/save roots must not increase that weight.
    pub fn mission_use_count(&self, asset: &str) -> usize {
        self.rhs
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(asset))
            .flat_map(|(_, planned)| planned.roots.iter())
            .filter_map(|root| match root {
                DependencyRoot::Mission(mission) => Some(mission.as_str()),
                DependencyRoot::Character(_) | DependencyRoot::SavedWorld => None,
            })
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(plan.mission_use_count("hero.rhs"), 2);
        assert_eq!(plan.mission_use_count("unreferenced.rhs"), 0);
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
        assert_eq!(plan.mission_use_count("HERO.RHS"), 1);
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
