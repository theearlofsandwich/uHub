//! Keyboard shortcuts: which keys do what (`shortcut`), and which app a
//! switching shortcut lands on (`target`). The keys are caught natively
//! (platform::watch_shortcuts) because the focused webview, often a service
//! page with no IPC, would otherwise get them; webviews.rs carries them out.
//!
//! | Keys                                  | Action                         |
//! |---------------------------------------|--------------------------------|
//! | Ctrl+1 … Ctrl+8                       | the 1st … 8th app in the bar   |
//! | Ctrl+9                                | the last app in the bar        |
//! | Ctrl+Tab, Ctrl+PageDown               | the next enabled app           |
//! | Ctrl+Shift+Tab, Ctrl+PageUp           | the previous enabled app       |
//! | Ctrl+R, Ctrl+Shift+R, F5              | reload the current app         |
//! | Alt+Left / Alt+Right                  | back / forward                 |
//! | Alt+Home                              | the current app's home page    |
//! | Ctrl+,                                | settings                       |
//! | Ctrl+Q                                | quit                           |
//!
//! The shell lists the same in its settings (`SHORTCUTS` in
//! src/components/SettingsModal.tsx). None of them while the shell shows a
//! modal (`set_paused`): its keys are the modal's.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    /// The app at this index in the bar (from 0).
    Nth(usize),
    Last,
    Next,
    Previous,
    Reload,
    Back,
    Forward,
    Home,
    Settings,
    Quit,
}

/// A key, as far as shortcuts care; platforms translate theirs to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Lowercase, so Caps Lock doesn't matter.
    Char(char),
    Tab,
    PageUp,
    PageDown,
    Left,
    Right,
    Home,
    F5,
}

/// The modifiers held. A key with any other (Super, say) is never a shortcut.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// What `key` with exactly `mods` does, if anything.
pub fn shortcut(key: Key, mods: Mods) -> Option<Shortcut> {
    let Mods { ctrl, shift, alt } = mods;
    Some(match (key, ctrl, shift, alt) {
        (Key::Char(c @ '1'..='8'), true, false, false) => Shortcut::Nth(c as usize - '1' as usize),
        (Key::Char('9'), true, false, false) => Shortcut::Last,
        (Key::Tab, true, false, false) | (Key::PageDown, true, false, false) => Shortcut::Next,
        (Key::Tab, true, true, false) | (Key::PageUp, true, false, false) => Shortcut::Previous,
        (Key::Char('r'), true, _, false) | (Key::F5, false, false, false) => Shortcut::Reload,
        (Key::Left, false, false, true) => Shortcut::Back,
        (Key::Right, false, false, true) => Shortcut::Forward,
        (Key::Home, false, false, true) => Shortcut::Home,
        (Key::Char(','), true, false, false) => Shortcut::Settings,
        (Key::Char('q'), true, false, false) => Shortcut::Quit,
        _ => return None,
    })
}

/// The app a switching shortcut goes to, from the bar's apps in order as
/// (id, enabled) and the current one. `Nth`/`Last` name a place in the bar
/// and go nowhere if that app is disabled; `Next`/`Previous` skip disabled
/// apps and wrap around. `None` for the other shortcuts, or nowhere to go.
pub fn target<'a>(shortcut: Shortcut, apps: &[(&'a str, bool)], current: Option<&str>) -> Option<&'a str> {
    let at = |i: usize| apps.get(i).filter(|&&(_, enabled)| enabled).map(|&(id, _)| id);
    let step = |forward: bool| {
        let n = apps.len();
        let from = current.and_then(|c| apps.iter().position(|&(id, _)| id == c));
        // From nowhere, Next starts at the first app and Previous at the last.
        let start = from.unwrap_or(if forward { n.wrapping_sub(1) } else { 0 });
        (1..=n).map(|k| if forward { (start + k) % n } else { (start + n - k % n) % n }).find_map(at)
    };
    match shortcut {
        Shortcut::Nth(i) => at(i),
        Shortcut::Last => apps.len().checked_sub(1).and_then(at),
        Shortcut::Next => step(true),
        Shortcut::Previous => step(false),
        _ => None,
    }
    .filter(|&id| Some(id) != current)
}

/// Set while the shell shows a modal (`webviews::set_overlay`).
static PAUSED: AtomicBool = AtomicBool::new(false);

pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::Relaxed);
}

pub fn paused() -> bool {
    PAUSED.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: Mods = Mods { ctrl: true, shift: false, alt: false };
    const CTRL_SHIFT: Mods = Mods { ctrl: true, shift: true, alt: false };
    const ALT: Mods = Mods { ctrl: false, shift: false, alt: true };
    const NONE: Mods = Mods { ctrl: false, shift: false, alt: false };

    #[test]
    fn keys() {
        assert_eq!(shortcut(Key::Char('1'), CTRL), Some(Shortcut::Nth(0)));
        assert_eq!(shortcut(Key::Char('8'), CTRL), Some(Shortcut::Nth(7)));
        assert_eq!(shortcut(Key::Char('9'), CTRL), Some(Shortcut::Last));
        assert_eq!(shortcut(Key::Tab, CTRL), Some(Shortcut::Next));
        assert_eq!(shortcut(Key::Tab, CTRL_SHIFT), Some(Shortcut::Previous));
        assert_eq!(shortcut(Key::PageDown, CTRL), Some(Shortcut::Next));
        assert_eq!(shortcut(Key::PageUp, CTRL), Some(Shortcut::Previous));
        assert_eq!(shortcut(Key::Char('r'), CTRL), Some(Shortcut::Reload));
        assert_eq!(shortcut(Key::Char('r'), CTRL_SHIFT), Some(Shortcut::Reload));
        assert_eq!(shortcut(Key::F5, NONE), Some(Shortcut::Reload));
        assert_eq!(shortcut(Key::Left, ALT), Some(Shortcut::Back));
        assert_eq!(shortcut(Key::Right, ALT), Some(Shortcut::Forward));
        assert_eq!(shortcut(Key::Home, ALT), Some(Shortcut::Home));
        assert_eq!(shortcut(Key::Char(','), CTRL), Some(Shortcut::Settings));
        assert_eq!(shortcut(Key::Char('q'), CTRL), Some(Shortcut::Quit));
    }

    #[test]
    fn other_keys_go_to_the_page() {
        assert_eq!(shortcut(Key::Char('0'), CTRL), None);
        assert_eq!(shortcut(Key::Char('1'), NONE), None);
        assert_eq!(shortcut(Key::Char('1'), CTRL_SHIFT), None);
        assert_eq!(shortcut(Key::Char('f'), CTRL), None, "find belongs to the page (Discord, Slack)");
        assert_eq!(shortcut(Key::Char('r'), NONE), None);
        assert_eq!(shortcut(Key::Tab, NONE), None);
        assert_eq!(shortcut(Key::Left, NONE), None);
        assert_eq!(shortcut(Key::Left, CTRL), None);
        assert_eq!(shortcut(Key::Char('q'), Mods { ctrl: true, shift: false, alt: true }), None);
    }

    const APPS: &[(&str, bool)] = &[("a", true), ("b", false), ("c", true), ("d", true)];

    #[test]
    fn nth_and_last() {
        assert_eq!(target(Shortcut::Nth(0), APPS, None), Some("a"));
        assert_eq!(target(Shortcut::Nth(1), APPS, None), None, "disabled");
        assert_eq!(target(Shortcut::Nth(2), APPS, Some("a")), Some("c"));
        assert_eq!(target(Shortcut::Nth(7), APPS, None), None, "no 8th app");
        assert_eq!(target(Shortcut::Last, APPS, None), Some("d"));
        assert_eq!(target(Shortcut::Last, &[], None), None);
        assert_eq!(target(Shortcut::Nth(0), APPS, Some("a")), None, "already there");
    }

    #[test]
    fn next_and_previous_skip_disabled_and_wrap() {
        assert_eq!(target(Shortcut::Next, APPS, Some("a")), Some("c"));
        assert_eq!(target(Shortcut::Next, APPS, Some("d")), Some("a"));
        assert_eq!(target(Shortcut::Previous, APPS, Some("c")), Some("a"));
        assert_eq!(target(Shortcut::Previous, APPS, Some("a")), Some("d"));
        assert_eq!(target(Shortcut::Next, APPS, None), Some("a"));
        assert_eq!(target(Shortcut::Previous, APPS, None), Some("d"));
        assert_eq!(target(Shortcut::Next, APPS, Some("gone")), Some("a"));
        assert_eq!(target(Shortcut::Next, &[("a", true), ("b", false)], Some("a")), None, "the only one");
        assert_eq!(target(Shortcut::Next, &[], None), None);
        assert_eq!(target(Shortcut::Reload, APPS, Some("a")), None);
    }
}
