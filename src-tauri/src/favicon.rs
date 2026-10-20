//! What the shell's "Add app" form pre-fills from the site itself: a name
//! (from its title) and its icon, as a `data:` URL ready to store (see
//! `Instance::icon`). Fetched here because the shell's CSP lets it load
//! nothing remote.
//!
//! The page's `<link rel="icon">` / `apple-touch-icon` tags are found by a
//! forgiving scan, not an HTML parser; the largest icon that loads (and is an
//! image) wins, then `/favicon.ico`.

use crate::{
    error::{Error, Result},
    services,
};
use base64::Engine as _;
use serde::Serialize;
use std::{io::Read, time::Duration};
use tauri::Url;
use ureq::ResponseExt as _;

#[derive(Serialize, Debug, Default)]
pub struct SiteInfo {
    /// From the page's `<title>`, see [`short_name`].
    pub name: Option<String>,
    pub icon: Option<String>,
}

const TIMEOUT: Duration = Duration::from_secs(10);
/// How much of the page is read looking for its `<head>`.
const MAX_PAGE: u64 = 1024 * 1024;
/// Largest icon file: what still fits `services::MAX_ICON_LEN` once base64-encoded.
const MAX_ICON: u64 = (services::MAX_ICON_LEN as u64 - 64) / 4 * 3;
/// Some sites serve a bare "unsupported browser" page to unknown agents.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// Fetch `url` and its icon. Blocking: run it off the async runtime's workers.
pub fn inspect(url: &str) -> Result<SiteInfo> {
    let url = services::parse_app_url(url)?;
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).user_agent(USER_AGENT).build().into();

    let mut page = agent.get(url.as_str()).call().map_err(|e| fetch_error(&url, e))?;
    // Relative icon links resolve against where redirects ended up.
    let base: Url = page.get_uri().to_string().parse().unwrap_or(url);
    let mut html = Vec::new();
    page.body_mut().as_reader().take(MAX_PAGE).read_to_end(&mut html).map_err(|e| fetch_error(&base, e))?;
    let (title, links) = scan(&String::from_utf8_lossy(&html));
    let name = title.map(|t| short_name(&t, &base));

    let mut candidates: Vec<Url> = links.iter().filter_map(|l| base.join(&l.href).ok()).collect();
    candidates.extend(base.join("/favicon.ico"));
    candidates.dedup();
    let icon = candidates.iter().find_map(|u| match fetch_icon(&agent, u) {
        Ok(icon) => Some(icon),
        Err(e) => {
            log::debug!("no icon at {u}: {e}");
            None
        }
    });
    Ok(SiteInfo { name, icon })
}

fn fetch_error(url: &Url, e: impl std::fmt::Display) -> Error {
    Error::Fetch(format!("couldn't load {url}: {e}"))
}

/// The image at `url` as a base64 `data:` URL.
fn fetch_icon(agent: &ureq::Agent, url: &Url) -> Result<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::Fetch(format!("not fetching {url}")));
    }
    let mut resp = agent.get(url.as_str()).call().map_err(|e| fetch_error(url, e))?;
    let header = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let mut bytes = Vec::new();
    resp.body_mut().as_reader().take(MAX_ICON + 1).read_to_end(&mut bytes).map_err(|e| fetch_error(url, e))?;
    if bytes.len() as u64 > MAX_ICON {
        return Err(Error::Fetch(format!("{url} is too large")));
    }
    let mime = image_type(&bytes, header.as_deref()).ok_or_else(|| Error::Fetch(format!("{url} is not an image")))?;
    let icon = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
    services::validate_icon(&icon)?;
    Ok(icon)
}

/// The image type of `bytes`, by their signature, else by a `Content-Type`
/// that says image (servers often send favicon.ico as text/plain, and an
/// HTML error page as image/x-icon is rarer than the reverse).
fn image_type(bytes: &[u8], content_type: Option<&str>) -> Option<String> {
    let sniffed = match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [0, 0, 1, 0, ..] => Some("image/x-icon"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    };
    if let Some(mime) = sniffed {
        return Some(mime.into());
    }
    let start = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).trim_start().to_ascii_lowercase();
    if start.starts_with("<svg") || (start.starts_with("<?xml") && start.contains("<svg")) {
        return Some("image/svg+xml".into());
    }
    if start.starts_with('<') || bytes.is_empty() {
        return None;
    }
    let mime = content_type?.split(';').next()?.trim().to_ascii_lowercase();
    (mime.starts_with("image/") && mime.len() > "image/".len()).then_some(mime)
}

/// Of the parts of a title ("Slack | AI work platform"), the one that names
/// the site's domain, else the first.
fn short_name(title: &str, url: &Url) -> String {
    let site = services::site_of(url);
    let label = site.split('.').next().unwrap_or("");
    let squash = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let parts: Vec<&str> = [" | ", " - ", " · ", " — ", " – ", ": "]
        .iter()
        .fold(vec![title], |parts, sep| parts.iter().flat_map(|p| p.split(sep)).collect())
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let named = parts.iter().find(|p| !label.is_empty() && squash(p).contains(&squash(label)));
    named.or(parts.first()).map_or(title, |p| p).to_owned()
}

/// An icon `<link>`, in the order [`scan`] prefers.
#[derive(Debug, PartialEq)]
struct IconLink {
    href: String,
    /// Largest side in pixels, guessed when the tag doesn't say.
    size: u32,
}

/// The `<title>` and icon links of the page's head, best icon first.
fn scan(html: &str) -> (Option<String>, Vec<IconLink>) {
    // ASCII lowercasing keeps byte offsets, so matches index into `html` too.
    let lower = html.to_ascii_lowercase();
    let end = lower.find("</head").unwrap_or(lower.len());
    let (html, lower) = (&html[..end], &lower[..end]);

    let title = lower.find("<title").and_then(|i| {
        let open = i + lower[i..].find('>')? + 1;
        let close = open + lower[open..].find("</title")?;
        let text = decode_entities(&html[open..close]).split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then(|| text.chars().take(64).collect())
    });

    let mut links = Vec::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<link") {
        let start = at + i + "<link".len();
        let Some(len) = lower[start..].find('>') else { break };
        at = start + len;
        let attrs = attributes(&html[start..at]);
        let get = |name: &str| attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        let rel = get("rel").unwrap_or("").to_ascii_lowercase();
        let rels: Vec<&str> = rel.split_whitespace().collect();
        let touch = rels.iter().any(|r| r.starts_with("apple-touch-icon"));
        let Some(href) = get("href").filter(|h| !h.is_empty() && (touch || rels.contains(&"icon"))) else {
            continue;
        };
        let href = decode_entities(href);
        let declared = get("sizes").and_then(largest_size);
        let size = declared.unwrap_or(if touch {
            180
        } else if get("type").is_some_and(|t| t.contains("svg")) || href.ends_with(".svg") {
            512
        } else {
            16
        });
        links.push(IconLink { href, size });
    }
    // Stable: equal sizes keep page order.
    links.sort_by_key(|l| std::cmp::Reverse(l.size));
    (title, links)
}

/// "16x16 32x32" -> 32; "any" (scalable) -> 512.
fn largest_size(sizes: &str) -> Option<u32> {
    sizes
        .split_whitespace()
        .filter_map(|s| {
            if s.eq_ignore_ascii_case("any") {
                return Some(512);
            }
            let s = s.to_ascii_lowercase();
            let (w, h) = s.split_once('x')?;
            Some(w.parse::<u32>().ok()?.max(h.parse().ok()?))
        })
        .max()
}

/// `name=value` pairs of a tag (value quoted with ' or ", or bare), names lowercased.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = tag.trim_start_matches('/');
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        let name_len = rest.find(|c: char| c.is_whitespace() || c == '=' || c == '/').unwrap_or(rest.len());
        if name_len == 0 {
            return out;
        }
        let name = rest[..name_len].to_ascii_lowercase();
        rest = rest[name_len..].trim_start();
        let Some(after_eq) = rest.strip_prefix('=') else {
            out.push((name, String::new()));
            continue;
        };
        rest = after_eq.trim_start();
        let value;
        match rest.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let body = &rest[1..];
                let len = body.find(q).unwrap_or(body.len());
                value = &body[..len];
                rest = body.get(len + 1..).unwrap_or("");
            }
            _ => {
                let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
                value = &rest[..len];
                rest = &rest[len..];
            }
        }
        out.push((name, value.to_owned()));
    }
}

/// The few entities titles and URLs actually use.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    [("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&#x27;", "'"), ("&nbsp;", " ")]
        .iter()
        .fold(s.to_owned(), |s, (from, to)| s.replace(from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(href: &str, size: u32) -> IconLink {
        IconLink { href: href.into(), size }
    }

    #[test]
    fn scans_title_and_icons_best_first() {
        let html = r#"<!doctype html><HTML><head>
            <meta charset=utf-8><TITLE>
              Slack &amp; Friends  </title>
            <link rel="stylesheet" href="/a.css">
            <link rel="shortcut icon" href="/favicon.ico">
            <link rel=icon type=image/png sizes=32x32 href=/f-32.png>
            <link rel='apple-touch-icon' href='/touch.png'/>
            <link rel="mask-icon" href="/mask.svg">
            <link rel="icon" sizes="16x16 192x192" href="/f.png?a=1&amp;b=2">
            <link rel="icon" href="">
            </head><body><link rel="icon" href="/body.png"><title>Nope</title>"#;
        let (title, links) = scan(html);
        assert_eq!(title.as_deref(), Some("Slack & Friends"));
        assert_eq!(
            links,
            [link("/f.png?a=1&b=2", 192), link("/touch.png", 180), link("/f-32.png", 32), link("/favicon.ico", 16)]
        );
    }

    #[test]
    fn scan_tolerates_missing_bits() {
        assert_eq!(scan(""), (None, vec![]));
        assert_eq!(scan("<title></title><link rel=icon"), (None, vec![]));
        let (title, links) = scan("<title>x</title><link rel=\"icon\" href=\"/i.svg\">");
        assert_eq!(title.as_deref(), Some("x"));
        assert_eq!(links, [link("/i.svg", 512)]);
        assert_eq!(scan(&format!("<title>{}</title>", "é".repeat(100))).0.unwrap().chars().count(), 64);
    }

    #[test]
    fn short_names() {
        let name = |t, u: &str| short_name(t, &u.parse().unwrap());
        assert_eq!(name("Slack | AI work platform and productivity tools", "https://app.slack.com/"), "Slack");
        assert_eq!(name("The AI workspace that works for you. | Notion", "https://www.notion.so/"), "Notion");
        assert_eq!(name("GitHub · Change is constant. · GitHub · ", "https://github.com/"), "GitHub");
        assert_eq!(name("Gmail", "https://mail.google.com/"), "Gmail");
        assert_eq!(name("Home - Assistant", "http://10.0.0.2:8123/"), "Home");
        assert_eq!(name(" | ", "https://x.example/"), " | ");
    }

    #[test]
    fn attribute_parsing() {
        let attrs = attributes(r#" REL="icon"  sizes=any data-x href='/a b.png' hidden /"#);
        let want = [("rel", "icon"), ("sizes", "any"), ("data-x", ""), ("href", "/a b.png"), ("hidden", "")];
        assert_eq!(attrs, want.map(|(k, v)| (k.to_owned(), v.to_owned())));
        assert_eq!(attributes(r#"href="unterminated"#), [("href".to_owned(), "unterminated".to_owned())]);
    }

    #[test]
    fn sizes() {
        assert_eq!(largest_size("16x16 48x48"), Some(48));
        assert_eq!(largest_size("any"), Some(512));
        assert_eq!(largest_size("32X64"), Some(64));
        assert_eq!(largest_size("big"), None);
    }

    #[test]
    fn image_sniffing() {
        assert_eq!(image_type(b"\x89PNG\r\n", Some("text/plain")).as_deref(), Some("image/png"));
        assert_eq!(image_type(&[0, 0, 1, 0, 1], None).as_deref(), Some("image/x-icon"));
        assert_eq!(image_type(b"  <svg xmlns=...>", None).as_deref(), Some("image/svg+xml"));
        assert_eq!(image_type(b"<?xml version='1.0'?><svg>", None).as_deref(), Some("image/svg+xml"));
        assert_eq!(image_type(b"\x00\x01avif?", Some("image/avif; q=1")).as_deref(), Some("image/avif"));
        assert_eq!(image_type(b"<!doctype html>404", Some("image/x-icon")), None, "error page");
        assert_eq!(image_type(b"\x00\x01", Some("text/html")), None);
        assert_eq!(image_type(b"", Some("image/png")), None);
    }

    #[test]
    fn icon_cap_fits_the_config_limit() {
        let encoded = (MAX_ICON as usize).div_ceil(3) * 4 + "data:image/svg+xml;base64,".len();
        assert!(encoded <= services::MAX_ICON_LEN);
    }
}
