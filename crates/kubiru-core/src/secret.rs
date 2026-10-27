//! A string that holds a credential.
//!
//! [`Secret`] keeps bearer tokens out of logs and debug output and wipes its
//! memory when dropped. Read the value with [`Secret::expose`] only at the
//! point where it is sent over the wire or written to the keychain.

use std::fmt;

use zeroize::Zeroizing;

/// A credential that never shows up in `Debug` output and is zeroed on drop.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// Wrap `value`, trimming the surrounding whitespace that copy-paste
    /// from a terminal tends to add.
    pub fn new(value: impl Into<String>) -> Self {
        let mut value: String = value.into();
        let trimmed = value.trim();
        if trimmed.len() != value.len() {
            let owned = trimmed.to_owned();
            zeroize::Zeroize::zeroize(&mut value);
            value = owned;
        }
        Self(Zeroizing::new(value))
    }

    /// The raw value. Keep the borrow as short as possible.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_value() {
        let secret = Secret::new("eyJhbGciOiJSUzI1NiJ9.super-secret");
        let printed = format!("{secret:?}");
        assert!(!printed.contains("super-secret"));
        assert_eq!(printed, "Secret([redacted])");
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(Secret::new("  token\n").expose(), "token");
    }

    #[test]
    fn blank_input_is_empty() {
        assert!(Secret::new(" \t\n").is_empty());
    }
}
