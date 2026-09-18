//! Test helpers shared by the combat, movement, AI and bow-shot suites.

use crate::element::{ElementData, ElementKind};

/// Default element data with only `kind` and `active` set.
pub(crate) fn test_element(kind: ElementKind, active: bool) -> ElementData {
    let mut element = ElementData::default();
    element.kind = kind;
    element.active = active;
    element
}
