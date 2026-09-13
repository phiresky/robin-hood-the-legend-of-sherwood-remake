//! Shared minimum safety policy for text displayed as a player or content identity.
//! Domain-specific length, spelling, and normalization rules belong to callers.

use unicode_security::{GeneralSecurityProfile, general_security_profile::IdentifierType};

/// Reject control characters and invisible formatting that can conceal an identity.
/// Ordinary ASCII spaces remain allowed; callers decide whether to trim them.
pub fn is_unsafe_display_character(character: char) -> bool {
    character.is_control()
        || (character.is_whitespace() && character != ' ')
        || character.identifier_type() == Some(IdentifierType::Default_Ignorable)
        || character == '\u{2800}'
        || matches!(
            character,
            '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_international_names_and_spaces_are_allowed() {
        for character in "Robin Hood Éowyn 张伟 محمد 🏹".chars() {
            assert!(!is_unsafe_display_character(character), "{character:?}");
        }
    }

    #[test]
    fn invisible_controls_and_formatting_are_rejected() {
        for character in [
            '\0', '\n', '\t', '\u{00ad}', '\u{00a0}', '\u{061c}', '\u{200b}', '\u{200f}',
            '\u{202e}', '\u{2066}', '\u{2800}', '\u{feff}',
        ] {
            assert!(is_unsafe_display_character(character), "{character:?}");
        }
    }
}
