/// The compatibility archive belongs only to the one fully static official
/// authoring target. It must never alter ordinary desktop, browser, Android,
/// verifier, or non-authoring feature graphs.
pub fn needs_musl_libdl_compatibility_archive(
    target_env: &str,
    projection_export_enabled: bool,
) -> bool {
    target_env == "musl" && projection_export_enabled
}

#[cfg(test)]
mod tests {
    use super::needs_musl_libdl_compatibility_archive;

    #[test]
    fn libdl_compatibility_is_exactly_musl_projection_only() {
        assert!(needs_musl_libdl_compatibility_archive("musl", true));
        for (target_env, projection_export_enabled) in [
            ("musl", false),
            ("gnu", true),
            ("gnu", false),
            ("msvc", true),
            ("", true),
        ] {
            assert!(!needs_musl_libdl_compatibility_archive(
                target_env,
                projection_export_enabled
            ));
        }
    }
}
