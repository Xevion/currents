//! Rebindable viewer actions and the key chords that trigger them.

use std::fmt;
use std::str::FromStr;

use serde::de::value::StrDeserializer;
use serde::{Deserialize, Serialize};
use snafu::{OptionExt, Snafu};
use strum::{Display, EnumMessage, IntoStaticStr, VariantArray};
use winit::keyboard::{KeyCode, ModifiersState};

/// Something a key does in the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Display, IntoStaticStr, VariantArray, EnumMessage)]
#[strum(serialize_all = "snake_case")]
pub enum Action {
    #[strum(message = "Open or close the settings menu")]
    ToggleMenu,
    #[strum(message = "Pause or resume the flow")]
    Pause,
    #[strum(message = "Ease the view back to the whole cover")]
    ResetView,
    #[strum(message = "Cycle the flow field debug views")]
    CycleDebug,
    #[strum(message = "Switch blending frames with the ones before them")]
    ToggleTemporal,
    #[strum(message = "Quit the viewer")]
    Quit,
}

impl Action {
    /// Whether the action means anything here; a browser tab is closed, not quit.
    pub const fn available(self) -> bool {
        !matches!(self, Self::Quit) || cfg!(not(target_arch = "wasm32"))
    }
}

/// A physical key and the exact modifiers held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub code: KeyCode,
    pub mods: ModifiersState,
}

/// Each modifier with the name a chord spells it with.
const MODIFIERS: [(ModifiersState, &str); 4] = [
    (ModifiersState::CONTROL, "Ctrl"),
    (ModifiersState::SHIFT, "Shift"),
    (ModifiersState::ALT, "Alt"),
    (ModifiersState::SUPER, "Super"),
];

impl Chord {
    pub const fn new(code: KeyCode, mods: ModifiersState) -> Self {
        Self { code, mods }
    }

    pub const fn plain(code: KeyCode) -> Self {
        Self::new(code, ModifiersState::empty())
    }

    pub const fn ctrl(code: KeyCode) -> Self {
        Self::new(code, ModifiersState::CONTROL)
    }

    /// Whether `code` is a modifier on its own, which never makes a chord.
    pub const fn is_modifier(code: KeyCode) -> bool {
        matches!(
            code,
            KeyCode::ControlLeft
                | KeyCode::ControlRight
                | KeyCode::ShiftLeft
                | KeyCode::ShiftRight
                | KeyCode::AltLeft
                | KeyCode::AltRight
                | KeyCode::SuperLeft
                | KeyCode::SuperRight
        )
    }

    /// The chord as a person reads it: `Esc`, `Ctrl+Q`.
    pub fn label(self) -> String {
        let code = format!("{:?}", self.code);
        let key = match self.code {
            KeyCode::Escape => "Esc",
            _ => code.strip_prefix("Key").or_else(|| code.strip_prefix("Digit")).unwrap_or(&code),
        };
        self.prefixed(key)
    }

    fn prefixed(self, key: &str) -> String {
        let mods = MODIFIERS.iter().filter(|(m, _)| self.mods.contains(*m)).map(|(_, name)| *name);
        mods.chain([key]).collect::<Vec<_>>().join("+")
    }
}

/// The settings file spelling: modifiers then the key's code name, as in `Ctrl+KeyQ`.
impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.prefixed(&format!("{:?}", self.code)))
    }
}

#[derive(Debug, Snafu)]
pub enum ChordError {
    #[snafu(display("`{name}` is not a key code such as Escape, KeyQ or F1"))]
    Code { name: String },
    #[snafu(display("`{name}` is not a modifier: use Ctrl, Shift, Alt or Super"))]
    Modifier { name: String },
    #[snafu(display("a modifier alone is not a chord"))]
    ModifierOnly,
}

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let name = parts.pop().unwrap_or_default();
        let code = KeyCode::deserialize(StrDeserializer::<serde::de::value::Error>::new(name))
            .ok()
            .context(CodeSnafu { name })?;
        if Self::is_modifier(code) {
            return ModifierOnlySnafu.fail();
        }
        let mut mods = ModifiersState::empty();
        for part in parts {
            mods |= match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ModifiersState::CONTROL,
                "shift" => ModifiersState::SHIFT,
                "alt" => ModifiersState::ALT,
                "super" | "logo" | "meta" => ModifiersState::SUPER,
                _ => return ModifierSnafu { name: part }.fail(),
            };
        }
        Ok(Self::new(code, mods))
    }
}

impl Serialize for Chord {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The chord bound to each action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keymap {
    pub toggle_menu: Chord,
    pub pause: Chord,
    pub reset_view: Chord,
    pub cycle_debug: Chord,
    pub toggle_temporal: Chord,
    pub quit: Chord,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            toggle_menu: Chord::plain(KeyCode::Escape),
            pause: Chord::plain(KeyCode::Space),
            reset_view: Chord::plain(KeyCode::KeyR),
            cycle_debug: Chord::plain(KeyCode::KeyD),
            toggle_temporal: Chord::plain(KeyCode::KeyT),
            quit: Chord::ctrl(KeyCode::KeyQ),
        }
    }
}

impl Keymap {
    pub const fn get(&self, action: Action) -> &Chord {
        match action {
            Action::ToggleMenu => &self.toggle_menu,
            Action::Pause => &self.pause,
            Action::ResetView => &self.reset_view,
            Action::CycleDebug => &self.cycle_debug,
            Action::ToggleTemporal => &self.toggle_temporal,
            Action::Quit => &self.quit,
        }
    }

    pub const fn get_mut(&mut self, action: Action) -> &mut Chord {
        match action {
            Action::ToggleMenu => &mut self.toggle_menu,
            Action::Pause => &mut self.pause,
            Action::ResetView => &mut self.reset_view,
            Action::CycleDebug => &mut self.cycle_debug,
            Action::ToggleTemporal => &mut self.toggle_temporal,
            Action::Quit => &mut self.quit,
        }
    }

    /// The first available action bound to `chord`.
    pub fn action(&self, chord: &Chord) -> Option<Action> {
        Action::VARIANTS.iter().copied().find(|&a| a.available() && self.get(a) == chord)
    }

    /// Actions sharing a chord with an earlier one, which therefore never fire.
    pub fn shadowed(&self) -> Vec<Action> {
        let live: Vec<Action> = Action::VARIANTS.iter().copied().filter(|a| a.available()).collect();
        live.iter()
            .enumerate()
            .filter(|&(i, &a)| live[..i].iter().any(|&b| self.get(b) == self.get(a)))
            .map(|(_, &a)| a)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use assert2::{assert, check};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::plain("Escape", Chord::plain(KeyCode::Escape))]
    #[case::ctrl("Ctrl+KeyQ", Chord::ctrl(KeyCode::KeyQ))]
    #[case::spaced_lowercase_mods("shift + alt + F1", Chord::new(KeyCode::F1, ModifiersState::SHIFT | ModifiersState::ALT))]
    fn parses_chords(#[case] text: &str, #[case] chord: Chord) {
        assert!(let Ok(parsed) = text.parse::<Chord>());
        check!(parsed == chord);
    }

    #[rstest]
    #[case::unknown_key("Ctrl+Banana")]
    #[case::unknown_modifier("Hyper+KeyA")]
    #[case::modifier_alone("ShiftLeft")]
    #[case::empty("")]
    fn rejects_bad_chords(#[case] text: &str) {
        check!(text.parse::<Chord>().is_err());
    }

    #[test]
    fn every_default_chord_survives_the_file_spelling() {
        let keys = Keymap::default();
        for &action in Action::VARIANTS {
            let chord = keys.get(action);
            check!(chord.to_string().parse::<Chord>().ok().as_ref() == Some(chord));
        }
    }

    #[test]
    fn labels_read_like_the_keyboard() {
        check!(Chord::plain(KeyCode::Escape).label() == "Esc");
        check!(Chord::ctrl(KeyCode::KeyQ).label() == "Ctrl+Q");
        check!(Chord::plain(KeyCode::Digit3).label() == "3");
    }

    #[test]
    fn a_chord_bound_twice_fires_only_the_first_action() {
        let mut keys = Keymap::default();
        keys.reset_view = keys.pause;
        check!(keys.action(&keys.pause) == Some(Action::Pause));
        check!(keys.shadowed() == vec![Action::ResetView]);
        check!(Keymap::default().shadowed().is_empty());
    }

    #[test]
    fn unbound_and_modifier_differing_chords_match_nothing() {
        let keys = Keymap::default();
        check!(keys.action(&Chord::plain(KeyCode::KeyZ)).is_none());
        check!(keys.action(&Chord::ctrl(KeyCode::Escape)).is_none());
        check!(
            Chord::new(KeyCode::KeyA, ModifiersState::SHIFT | ModifiersState::CONTROL).to_string() == "Ctrl+Shift+KeyA"
        );
    }
}
