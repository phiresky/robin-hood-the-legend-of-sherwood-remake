//! Serialization-only diagnostic projections must never become load authority.

/// Reject deserialization of a diagnostic-only type with its domain-specific error.
/// Serialization remains owned by the caller: some projections use custom output.
/// `$message` is any `&'static str` expression, so callers inside their own macros
/// may build it with `concat!(stringify!(..), ..)`.
#[macro_export]
macro_rules! deny_deserialize {
    ($ty:ty, $message:expr) => {
        impl<'de> ::serde::Deserialize<'de> for $ty {
            fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                Err(::serde::de::Error::custom($message))
            }
        }
    };
}

#[cfg(test)]
mod tests {
    #[derive(Debug, serde::Serialize)]
    struct Projection;
    crate::deny_deserialize!(Projection, "diagnostic projection is not load authority");

    #[test]
    fn rejects_even_well_formed_json() {
        assert!(
            serde_json::from_str::<Projection>("null")
                .unwrap_err()
                .to_string()
                .contains("diagnostic projection is not load authority")
        );
    }
}
