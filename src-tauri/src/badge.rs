//! Unread counts read from page titles (the webview's title-change event, see
//! platform/), so service pages need no IPC to report them.
//!
//! A parser returns `Some(count)` when the title says how many unread items
//! there are (a title without a count means 0) and `None` when the title says
//! nothing about it, in which case the last count is kept.

pub type BadgeParser = fn(&str) -> Option<u32>;

/// Shared default: the first "(N)" in the title, as in "(3) WhatsApp" or
/// "Inbox (12) - ...". Thousands separators and a trailing "+" ("(99+)") are
/// accepted. No "(N)" anywhere means nothing is unread.
pub fn from_title(title: &str) -> Option<u32> {
    Some(title.match_indices('(').find_map(|(i, _)| count_at(&title[i..])).unwrap_or(0))
}

/// Gmail: the default fails while a message is open, because the title is then
/// its subject (which can itself contain "(2)") and carries no inbox count.
/// Only the inbox view's "Inbox (N) - ..." / "Inbox - ..." is trusted.
/// English UI only.
pub fn gmail(title: &str) -> Option<u32> {
    let rest = title.strip_prefix("Inbox")?;
    if rest.starts_with(" - ") {
        Some(0)
    } else {
        count_at(rest.strip_prefix(' ')?)
    }
}

/// The count in a "(N)" at the very start of `s`.
fn count_at(s: &str) -> Option<u32> {
    let inner = &s[..s.find(')')?];
    let digits = inner.strip_prefix('(')?;
    let digits = digits.strip_suffix('+').unwrap_or(digits);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit() || b == b',') || digits.starts_with(',') {
        return None;
    }
    digits.replace(',', "").parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_parser() {
        let cases = [
            ("(3) WhatsApp", 3),
            ("WhatsApp", 0),
            ("", 0),
            ("Inbox (12) - me@example.com - Gmail", 12),
            ("(1) Discord | #general | Server", 1),
            ("(99+) Discord", 99),
            ("(1,234) Inbox", 1234),
            ("(0) WhatsApp", 0),
            // Parentheses that hold no count are skipped, not read as 0 and stopped at.
            ("Discord (beta) (5)", 5),
            ("Chat (x) WhatsApp", 0),
            ("(3 WhatsApp", 0),
            ("() WhatsApp", 0),
            ("(,) WhatsApp", 0),
            ("(-3) WhatsApp", 0),
            ("(99999999999) overflow", 0),
            ("• Discord | #general", 0),
        ];
        for (title, want) in cases {
            assert_eq!(from_title(title), Some(want), "{title:?}");
        }
    }

    #[test]
    fn gmail_parser() {
        assert_eq!(gmail("Inbox (12) - me@example.com - Gmail"), Some(12));
        assert_eq!(gmail("Inbox (1,024) - me@example.com - Gmail"), Some(1024));
        assert_eq!(gmail("Inbox - me@example.com - Gmail"), Some(0));
        // An open message or another label: no information, keep the last count.
        assert_eq!(gmail("Meeting notes (2) - me@example.com - Gmail"), None);
        assert_eq!(gmail("(5) things - me@example.com - Gmail"), None);
        assert_eq!(gmail("Starred - me@example.com - Gmail"), None);
        assert_eq!(gmail("Inboxes (3) - me@example.com - Gmail"), None);
        assert_eq!(gmail("Gmail"), None);
        assert_eq!(gmail(""), None);
    }
}
