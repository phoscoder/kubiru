//! Light, dark, or follow the system.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Window};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// The value stored in preferences.
    pub fn key(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|a| a.key() == key)
            .unwrap_or_default()
    }

    /// Switch the theme to match.
    pub fn apply(self, window: Option<&mut Window>, cx: &mut App) {
        match self {
            Appearance::System => Theme::sync_system_appearance(window, cx),
            Appearance::Light => Theme::change(ThemeMode::Light, window, cx),
            Appearance::Dark => Theme::change(ThemeMode::Dark, window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_unknown_values_fall_back_to_system() {
        for a in Appearance::ALL {
            assert_eq!(Appearance::from_key(a.key()), a);
        }
        assert_eq!(Appearance::from_key("sepia"), Appearance::System);
    }
}
