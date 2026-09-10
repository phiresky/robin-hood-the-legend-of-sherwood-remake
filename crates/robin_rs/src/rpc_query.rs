//! Validated native URL query decoding. Invalid input never becomes an absent option.
use crate::http_server::{RpcError, ScreenshotFlags, ScreenshotRequest};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, btree_map::Entry},
    str::FromStr,
};

#[derive(Debug, Serialize, Deserialize)]
struct QueryParameters(BTreeMap<String, String>);

impl QueryParameters {
    fn parse(query: &str) -> Result<Self, RpcError> {
        let mut values = BTreeMap::new();
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let key = decode_component(key)?;
            let value = decode_component(value)?;
            if key.is_empty() {
                return Err(RpcError::invalid_request("empty query parameter name"));
            }
            match values.entry(key) {
                Entry::Vacant(entry) => {
                    entry.insert(value);
                }
                Entry::Occupied(entry) => {
                    return Err(RpcError::invalid_request(format!(
                        "duplicate query parameter: {}",
                        entry.key()
                    )));
                }
            }
        }
        Ok(Self(values))
    }

    fn number<T: FromStr>(&mut self, key: &str) -> Result<Option<T>, RpcError> {
        self.0
            .remove(key)
            .map(|value| {
                value.parse().map_err(|_| {
                    RpcError::invalid_request(format!("invalid numeric query parameter: {key}"))
                })
            })
            .transpose()
    }

    fn flag(&mut self, key: &str) -> Result<Option<bool>, RpcError> {
        self.0
            .remove(key)
            .map(|value| match value.to_ascii_lowercase().as_str() {
                "" | "1" | "true" | "yes" | "on" => Ok(true),
                "0" | "false" | "no" | "off" => Ok(false),
                _ => Err(RpcError::invalid_request(format!(
                    "invalid boolean query parameter: {key}"
                ))),
            })
            .transpose()
    }

    fn finish(self) -> Result<(), RpcError> {
        match self.0.first_key_value() {
            None => Ok(()),
            Some((key, _)) => Err(RpcError::invalid_request(format!(
                "unknown query parameter: {key}"
            ))),
        }
    }
}

fn decode_component(raw: &str) -> Result<String, RpcError> {
    // percent_decode deliberately tolerates malformed escapes; reject those
    // before handing decoding and UTF-8 validation to the library.
    let bytes = raw.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
        {
            return Err(RpcError::invalid_request("malformed query percent escape"));
        }
    }
    let form = if raw.contains('+') {
        Cow::Owned(raw.replace('+', " "))
    } else {
        Cow::Borrowed(raw)
    };
    let decoded = percent_encoding::percent_decode_str(&form)
        .decode_utf8()
        .map_err(|_| RpcError::invalid_request("query parameter is not valid UTF-8"))?;
    Ok(match decoded {
        Cow::Borrowed(_) => form.into_owned(),
        Cow::Owned(value) => value,
    })
}

pub(crate) fn screenshot(query: &str) -> Result<ScreenshotRequest, RpcError> {
    let mut query = QueryParameters::parse(query)?;
    let request = ScreenshotRequest {
        frame: query.number("frame")?,
        width: query.number("w")?,
        height: query.number("h")?,
        hide_ui: query.flag("hide_ui")?.unwrap_or(false),
        full_map: query.flag("full_map")?.unwrap_or(false),
        flags: ScreenshotFlags {
            view_cones: query.flag("view_cones")?,
            pc_sight: query.flag("pc_sight")?,
            motion_graph: query.flag("motion_graph")?,
            surface: query.flag("surface")?,
            all_obstacles: query.flag("all_obstacles")?,
            elevation: query.flag("elevation")?,
            noise: query.flag("noise")?,
            sound_source: query.flag("sound_source")?,
            actor_info: query.flag("actor_info")?,
            script_zones: query.flag("script_zones")?,
            door: query.flag("door")?,
            projection_areas: query.flag("projection_areas")?,
            railroad: query.flag("railroad")?,
            probability: query.flag("probability")?,
            company_number: query.flag("company_number")?,
            combat_energy: query.flag("combat_energy")?,
            light_zones: query.flag("light_zones")?,
            animation_lines: query.flag("animation_lines")?,
            seek_points: query.flag("seek_points")?,
            fps: query.flag("fps")?,
            sprite_masks: query.flag("sprite_masks")?,
            // Preserve the native endpoint's existing default-on ID overlay.
            entity_ids: Some(query.flag("entity_ids")?.unwrap_or(true)),
        },
    };
    query.finish()?;
    if request.width == Some(0) || request.height == Some(0) {
        return Err(RpcError::invalid_request(
            "screenshot width/height must be > 0",
        ));
    }
    Ok(request)
}

pub(crate) fn decompile_class(query: &str) -> Result<Option<String>, RpcError> {
    let mut query = QueryParameters::parse(query)?;
    let class = query.0.remove("class");
    query.finish()?;
    Ok(class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_decoding_preserves_form_rules_and_decodes_only_once() {
        for (input, expected) in [
            ("", ""),
            ("plain", "plain"),
            ("雪", "雪"),
            ("two+words", "two words"),
            ("%E9%9B%AA", "雪"),
            ("%2B+%252B", "+ %2B"),
            ("left%26right%3Dvalue", "left&right=value"),
        ] {
            assert_eq!(decode_component(input).unwrap(), expected, "{input}");
        }
        for input in ["%", "%2", "%GG", "text+%"] {
            assert_eq!(
                decode_component(input).unwrap_err().message,
                "malformed query percent escape"
            );
        }
        assert_eq!(
            decode_component("%FF").unwrap_err().message,
            "query parameter is not valid UTF-8"
        );
    }

    #[test]
    fn valid_values_and_native_defaults_are_preserved() {
        let value =
            screenshot("frame=10&w=640&h=480&full_map=1&hide_ui=TRUE&entity_ids=0").unwrap();
        assert_eq!(value.frame, Some(10));
        assert_eq!((value.width, value.height), (Some(640), Some(480)));
        assert!(value.full_map && value.hide_ui);
        assert_eq!(value.flags.entity_ids, Some(false));
        let defaults = screenshot("").unwrap();
        assert_eq!(defaults.flags.entity_ids, Some(true));
        assert_eq!(defaults.frame, None);
        assert!(!defaults.hide_ui);
    }

    #[test]
    fn bare_and_empty_flags_are_true() {
        let value = screenshot("view_cones&pc_sight=&hide_ui&full_map").unwrap();
        assert_eq!(value.flags.view_cones, Some(true));
        assert_eq!(value.flags.pc_sight, Some(true));
        assert!(value.hide_ui && value.full_map);
        for value in ["yes", "on", "TRUE", "1"] {
            assert_eq!(
                screenshot(&format!("view_cones={value}"))
                    .unwrap()
                    .flags
                    .view_cones,
                Some(true)
            );
        }
        for value in ["no", "off", "FALSE", "0"] {
            assert_eq!(
                screenshot(&format!("view_cones={value}"))
                    .unwrap()
                    .flags
                    .view_cones,
                Some(false)
            );
        }
    }

    #[test]
    fn malformed_values_are_errors_not_defaults() {
        for query in [
            "frame=oops",
            "frame=",
            "frame=-1",
            "frame=4294967296",
            "w=65536",
            "w=0",
            "h=0",
            "hide_ui=maybe",
            "entity_ids=nah",
            "view_cones=2",
            "unknown=1",
            "=1",
            "frame=1&frame=2",
            "frame=1&%66rame=2",
            "view_cones&view_cones=0",
            "w=%",
            "w=%2",
            "w=%GG",
            "w=%FF",
        ] {
            let error = screenshot(query).expect_err(query);
            assert_eq!(
                error.kind,
                crate::http_server::RpcErrorKind::InvalidRequest,
                "{query}"
            );
        }
    }

    #[test]
    fn decompile_names_decode_once_and_reject_ambiguous_queries() {
        assert_eq!(
            decompile_class("class=Guard%20A%2BB").unwrap().as_deref(),
            Some("Guard A+B")
        );
        assert_eq!(
            decompile_class("class=Guard+A").unwrap().as_deref(),
            Some("Guard A")
        );
        assert_eq!(
            decompile_class("class=%2520").unwrap().as_deref(),
            Some("%20")
        );
        assert_eq!(decompile_class("").unwrap(), None);
        for query in ["class=a&class=b", "class=%FF", "class=%GG", "clas=a"] {
            assert!(decompile_class(query).is_err(), "{query}");
        }
    }
}
