//! Service registry, Ferdium-style: compiled-in *recipes* (what a site is: URL,
//! allowed domains, injected JS/CSS) and user-configured *instances* (an
//! account of a recipe: name, storage profile, team, optional URL), saved as
//! JSON in `app_config_dir()/services.json`.
//!
//! Rust is the source of truth for URLs, allowed domains and profiles: the
//! (trusted) React shell only ever sends an instance *id*, so a compromised or
//! buggy frontend cannot point a service at an arbitrary origin. Everything
//! read from the config file is validated here (profile names become paths, URL
//! overrides must stay within the recipe's domains) before it is used.
//!
//! "Add app" in the shell picks a recipe from [`RECIPES`]. The [`CUSTOM`]
//! recipe is any site: like a self-hosted one (see [`Hosting`]), it may
//! navigate within its URL's site (see [`site_of`]). Recipe icons and
//! addresses come from Ferdium's recipes (github.com/ferdium/ferdium-recipes,
//! MIT); allowed domains are uHub's own.

use crate::{
    badge::{self, BadgeParser},
    error::{Error, Result},
    navigation,
};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Url;
use url::Host;

/// A site uHub knows how to host. Compiled in, keyed by `id`.
pub struct Recipe {
    pub id: &'static str,
    pub name: &'static str,
    /// The page a new instance loads. With `team`, "{team}" stands for the
    /// instance's team; empty for a [`Hosting::SelfHosted`] recipe.
    pub url: &'static str,
    /// Host suffixes the webview may navigate within ("a.com" matches a.com and *.a.com).
    pub allowed_domains: &'static [&'static str],
    pub inject_js: &'static str,
    pub inject_css: &'static str,
    pub user_agent: Option<&'static str>,
    /// SVG source.
    pub icon: Option<&'static str>,
    /// Reads the unread count from the page title; `None`: the site shows none.
    pub badge: Option<BadgeParser>,
    /// The site needs camera/microphone (calls, voice messages). Without this,
    /// user-media requests are denied whatever the instance allows.
    pub user_media: bool,
    /// Default for `Instance::keep_alive`: messengers stay loaded in the
    /// background so their notifications keep arriving.
    pub keep_alive: bool,
    /// Each instance names its team, which `url` is built from.
    pub team: Option<TeamField>,
    pub hosting: Hosting,
    /// A note for the add/edit form.
    pub message: Option<&'static str>,
}

/// How the add/edit form asks for a team (Ferdium's `hasTeamId`): Slack's
/// workspace, shown as `prefix` [input] `suffix`.
#[derive(Serialize, Clone, Copy, Debug)]
pub struct TeamField {
    pub label: &'static str,
    pub prefix: &'static str,
    pub suffix: &'static str,
}

/// Where a recipe's site runs.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Hosting {
    /// Only at `url`; an instance's URL must stay within the recipe's domains.
    Cloud,
    /// At `url`, or on the user's own server: an instance URL outside the
    /// recipe's domains is a server of its own, allowed only its own site.
    Both,
    /// Only on the user's own server: every instance has a URL, any http(s) one.
    SelfHosted,
}

/// Recipe id of an app added by its URL: a self-hosted recipe without
/// injections, for any site.
pub const CUSTOM: &str = "custom";

/// What every recipe has unless it says otherwise.
const RECIPE: Recipe = Recipe {
    id: "",
    name: "",
    url: "",
    allowed_domains: &[],
    inject_js: "",
    inject_css: "",
    user_agent: None,
    icon: None,
    badge: Some(badge::from_title),
    user_media: false,
    keep_alive: false,
    team: None,
    hosting: Hosting::Cloud,
    message: None,
};

const GOOGLE: &[&str] = &["google.com", "gstatic.com", "googleusercontent.com"];
/// Microsoft 365 apps and the Microsoft account / Entra ID sign-in they share.
const MICROSOFT: &[&str] = &[
    "cloud.microsoft",
    "microsoft.com",
    "office.com",
    "office.net",
    "office365.com",
    "outlook.com",
    "live.com",
    "microsoftonline.com",
    "msauth.net",
    "msftauth.net",
];

/// In the order "Add app" lists them, [`CUSTOM`] last.
pub const RECIPES: &[Recipe] = &[
    Recipe {
        id: "whatsapp",
        name: "WhatsApp",
        url: "https://web.whatsapp.com/",
        allowed_domains: &["whatsapp.com", "whatsapp.net"],
        inject_js: include_str!("../../services/whatsapp/inject.js"),
        inject_css: include_str!("../../services/whatsapp/inject.css"),
        icon: Some(include_str!("../../services/whatsapp/icon.svg")),
        user_media: true,
        keep_alive: true,
        ..RECIPE
    },
    Recipe {
        id: "gmail",
        name: "Gmail",
        url: "https://mail.google.com/",
        allowed_domains: GOOGLE,
        inject_js: include_str!("../../services/gmail/inject.js"),
        inject_css: include_str!("../../services/gmail/inject.css"),
        icon: Some(include_str!("../../services/gmail/icon.svg")),
        badge: Some(badge::gmail),
        ..RECIPE
    },
    Recipe {
        id: "outlook",
        name: "Outlook",
        url: "https://outlook.cloud.microsoft/mail/",
        allowed_domains: MICROSOFT,
        icon: Some(include_str!("../../services/outlook/icon.svg")),
        // The title carries no count.
        badge: None,
        hosting: Hosting::Both,
        ..RECIPE
    },
    Recipe {
        id: "discord",
        name: "Discord",
        url: "https://discord.com/app",
        allowed_domains: &["discord.com", "discordapp.com", "discordapp.net", "hcaptcha.com"],
        inject_js: include_str!("../../services/discord/inject.js"),
        inject_css: include_str!("../../services/discord/inject.css"),
        icon: Some(include_str!("../../services/discord/icon.svg")),
        user_media: true,
        keep_alive: true,
        ..RECIPE
    },
    Recipe {
        id: "slack",
        name: "Slack",
        url: "https://{team}.slack.com/",
        allowed_domains: &["slack.com", "slack-edge.com", "slack-imgs.com", "slack-files.com", "slackb.com"],
        icon: Some(include_str!("../../services/slack/icon.svg")),
        // Marks unread channels in the title, but without a count.
        badge: None,
        user_media: true,
        keep_alive: true,
        team: Some(TeamField { label: "Workspace", prefix: "", suffix: ".slack.com" }),
        message: Some("The workspace's name from its Slack address, as in acme.slack.com"),
        ..RECIPE
    },
    Recipe {
        id: "teams",
        name: "Microsoft Teams",
        url: "https://teams.cloud.microsoft/",
        allowed_domains: MICROSOFT,
        icon: Some(include_str!("../../services/teams/icon.svg")),
        user_media: true,
        keep_alive: true,
        ..RECIPE
    },
    Recipe {
        id: "telegram",
        name: "Telegram",
        url: "https://web.telegram.org/",
        allowed_domains: &["telegram.org", "t.me"],
        icon: Some(include_str!("../../services/telegram/icon.svg")),
        user_media: true,
        keep_alive: true,
        ..RECIPE
    },
    Recipe {
        id: "messenger",
        name: "Messenger",
        url: "https://www.messenger.com/",
        allowed_domains: &["messenger.com", "facebook.com", "fbcdn.net", "fbsbx.com"],
        icon: Some(include_str!("../../services/messenger/icon.svg")),
        user_media: true,
        keep_alive: true,
        ..RECIPE
    },
    Recipe {
        id: "google-chat",
        name: "Google Chat",
        url: "https://chat.google.com/",
        allowed_domains: GOOGLE,
        icon: Some(include_str!("../../services/google-chat/icon.svg")),
        // The title is the open room's name, which may hold "(N)" itself.
        badge: None,
        user_media: true,
        keep_alive: true,
        ..RECIPE
    },
    Recipe {
        id: "google-calendar",
        name: "Google Calendar",
        url: "https://calendar.google.com/",
        allowed_domains: GOOGLE,
        icon: Some(include_str!("../../services/google-calendar/icon.svg")),
        badge: None,
        ..RECIPE
    },
    Recipe {
        id: "proton-mail",
        name: "Proton Mail",
        url: "https://mail.proton.me/",
        allowed_domains: &["proton.me", "protonmail.com"],
        icon: Some(include_str!("../../services/proton-mail/icon.svg")),
        ..RECIPE
    },
    Recipe {
        id: "element",
        name: "Element",
        url: "https://app.element.io/",
        allowed_domains: &["element.io", "matrix.org"],
        icon: Some(include_str!("../../services/element/icon.svg")),
        badge: Some(badge::brackets),
        user_media: true,
        keep_alive: true,
        hosting: Hosting::Both,
        ..RECIPE
    },
    Recipe {
        id: "mattermost",
        name: "Mattermost",
        icon: Some(include_str!("../../services/mattermost/icon.svg")),
        user_media: true,
        keep_alive: true,
        hosting: Hosting::SelfHosted,
        ..RECIPE
    },
    Recipe {
        id: "linkedin",
        name: "LinkedIn",
        url: "https://www.linkedin.com/messaging/",
        allowed_domains: &["linkedin.com", "licdn.com"],
        icon: Some(include_str!("../../services/linkedin/icon.svg")),
        ..RECIPE
    },
    Recipe {
        id: "notion",
        name: "Notion",
        url: "https://www.notion.so/",
        allowed_domains: &["notion.so", "notion.com"],
        icon: Some(include_str!("../../services/notion/icon.svg")),
        badge: None,
        ..RECIPE
    },
    Recipe {
        id: "trello",
        name: "Trello",
        url: "https://trello.com/",
        allowed_domains: &["trello.com", "atlassian.com", "atl-paas.net"],
        icon: Some(include_str!("../../services/trello/icon.svg")),
        badge: None,
        ..RECIPE
    },
    Recipe {
        id: "github",
        name: "GitHub",
        url: "https://github.com/",
        allowed_domains: &["github.com", "githubusercontent.com", "githubassets.com"],
        icon: Some(include_str!("../../services/github/icon.svg")),
        badge: None,
        hosting: Hosting::Both,
        ..RECIPE
    },
    Recipe {
        id: "chatgpt",
        name: "ChatGPT",
        url: "https://chatgpt.com/",
        // The Cloudflare host serves the bot check, in a frame.
        allowed_domains: &["chatgpt.com", "openai.com", "oaistatic.com", "oaiusercontent.com", "challenges.cloudflare.com"],
        icon: Some(include_str!("../../services/chatgpt/icon.svg")),
        badge: None,
        user_media: true,
        ..RECIPE
    },
    Recipe {
        id: "claude",
        name: "Claude",
        url: "https://claude.ai/",
        allowed_domains: &["claude.ai", "claude.com", "anthropic.com", "challenges.cloudflare.com"],
        icon: Some(include_str!("../../services/claude/icon.svg")),
        badge: None,
        user_media: true,
        ..RECIPE
    },
    Recipe { id: CUSTOM, name: "Custom website", user_media: true, hosting: Hosting::SelfHosted, ..RECIPE },
];

pub fn recipe(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.id == id)
}

impl Recipe {
    /// `url` for `team` (which only a recipe with a `team` field takes).
    pub fn url_for(&self, team: Option<&str>) -> Result<Url> {
        let url = match (self.team, team) {
            (Some(_), Some(team)) => {
                validate_team(team)?;
                self.url.replace("{team}", team)
            }
            (Some(field), None) => return Err(Error::invalid(format!("{} needs a {}", self.name, field.label.to_lowercase()))),
            (None, Some(_)) => return Err(Error::invalid(format!("{} has no team", self.name))),
            (None, None) => self.url.to_owned(),
        };
        parse_app_url(&url)
    }

    /// `icon` as a data: URL.
    pub fn icon_url(&self) -> Option<String> {
        self.icon.map(|svg| format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg)))
    }

    /// Whether `url` is on the recipe's own site (over https).
    fn covers(&self, url: &Url) -> bool {
        url.scheme() == "https" && url.host_str().is_some_and(|h| navigation::host_allowed(h, self.allowed_domains))
    }
}

/// One configured service, as stored in services.json.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Instance {
    /// Generated once, never changes; also the webview label suffix.
    pub id: String,
    pub name: String,
    pub recipe: String,
    /// Storage profile. Two services sharing a profile share cookies/storage;
    /// distinct profiles get distinct WebKit data directories.
    pub profile: String,
    /// The team the recipe's URL is built from, for a recipe with a `team` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// Replaces the recipe's URL: https within its allowed domains, or (as
    /// its [`Hosting`] allows) the user's own server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub order: u32,
    #[serde(default)]
    pub permissions: Permissions,
    /// Never hibernate this service; `None` means the recipe's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<bool>,
    /// Hibernate after this many seconds in the background instead of the
    /// app-wide `Settings::hibernate_after_minutes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hibernate_after_seconds: Option<u32>,
    /// While hibernated, load in the background every this many seconds, so
    /// it can sync and notify. `None`: stays hibernated until switched to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_wake_seconds: Option<u32>,
    /// Hibernate this many seconds after an auto-wake (unless switched to
    /// meanwhile); `None`: as after any other time in the background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hibernate_after_wake_seconds: Option<u32>,
    /// A base64 `data:image/...` URL; `None` means the recipe's icon, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Sound from the page (media and its own notification sounds); `false`
    /// mutes the webview.
    #[serde(default = "yes")]
    pub audio: bool,
    /// Show in the sidebar while the page plays audio.
    #[serde(default = "yes")]
    pub media_indicator: bool,
    /// Show the unread count in the sidebar and include it in the tray total.
    #[serde(default = "yes")]
    pub unread_badge: bool,
    /// Off-site links open in a uHub window (with this service's profile)
    /// instead of the system browser.
    #[serde(default)]
    pub open_links_inside: bool,
    /// Stay unloaded when uHub starts, until switched to; otherwise (if
    /// enabled) it is loaded in the background at startup.
    #[serde(default)]
    pub start_hibernated: bool,
}

impl Instance {
    /// Take the form's recipe, team, URL and options; [`Service::new`]
    /// validates the result. Id, profile and order stay.
    fn apply(&mut self, new: &NewService) -> Result<()> {
        let recipe = recipe(&new.recipe).ok_or_else(|| Error::invalid(format!("unknown recipe {:?}", new.recipe)))?;
        let filled = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        self.name = validate_name(&new.name)?;
        self.recipe = recipe.id.into();
        self.team = filled(&new.team).map(|t| t.to_ascii_lowercase());
        self.url = match filled(&new.url) {
            // The recipe's own URL isn't stored, so the instance follows it.
            Some(url) => {
                let url = parse_app_url(&url)?;
                (recipe.url_for(self.team.as_deref()).ok() != Some(url.clone())).then(|| url.to_string())
            }
            None => None,
        };
        self.enabled = new.enabled;
        self.permissions.notifications = new.notifications;
        self.keep_alive = Some(!new.hibernation);
        self.hibernate_after_seconds = Some(new.hibernate_after_seconds);
        self.auto_wake_seconds = new.auto_wake_seconds;
        self.hibernate_after_wake_seconds = Some(new.hibernate_after_wake_seconds);
        self.icon = new.icon.clone();
        self.audio = new.audio;
        self.media_indicator = new.media_indicator;
        self.unread_badge = new.unread_badge;
        self.open_links_inside = new.open_links_inside;
        self.start_hibernated = new.start_hibernated;
        Ok(())
    }

    /// An instance with every option at its default, in its own profile named after `id`.
    fn new(id: String, name: String, recipe: &str, order: u32) -> Self {
        Self {
            profile: id.clone(),
            id,
            name,
            recipe: recipe.into(),
            team: None,
            url: None,
            enabled: true,
            order,
            permissions: Permissions::default(),
            keep_alive: None,
            hibernate_after_seconds: None,
            auto_wake_seconds: None,
            hibernate_after_wake_seconds: None,
            icon: None,
            audio: true,
            media_indicator: true,
            unread_badge: true,
            open_links_inside: false,
            start_hibernated: false,
        }
    }
}

/// What the shell's "Add app" and "Edit app" forms send.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NewService {
    pub name: String,
    pub recipe: String,
    /// For a recipe with a `team` field.
    #[serde(default)]
    pub team: Option<String>,
    /// `None` (or blank) for the recipe's own URL; required by a self-hosted recipe.
    #[serde(default)]
    pub url: Option<String>,
    pub icon: Option<String>,
    pub notifications: bool,
    pub audio: bool,
    pub media_indicator: bool,
    pub unread_badge: bool,
    pub enabled: bool,
    /// `false`: kept alive (see `Instance::keep_alive`).
    pub hibernation: bool,
    pub hibernate_after_seconds: u32,
    /// `None`: no auto-wake (see `Instance::auto_wake_seconds`).
    pub auto_wake_seconds: Option<u32>,
    pub hibernate_after_wake_seconds: u32,
    pub open_links_inside: bool,
    pub start_hibernated: bool,
}

fn yes() -> bool {
    true
}

/// Longest hibernation delay, app-wide or per service: a week.
const MAX_HIBERNATE_MINUTES: u32 = 7 * 24 * 60;

/// App-wide settings, the `settings` object of services.json.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Settings {
    /// Unload a background service (unless kept alive) after this many
    /// minutes; 0 = never.
    #[serde(default = "default_hibernate_after")]
    pub hibernate_after_minutes: u32,
    /// Keep at most this many background services (not counting kept-alive
    /// ones) loaded, unloading the oldest; `None` = no limit.
    #[serde(default)]
    pub max_suspended: Option<u32>,
    /// Closing the window hides it to the tray instead of quitting.
    #[serde(default = "yes")]
    pub close_to_tray: bool,
    /// The window edge the app bar sits on.
    #[serde(default, deserialize_with = "or_default")]
    pub bar_position: BarPosition,
    /// Show app names next to their icons; only a top or bottom bar has room.
    #[serde(default)]
    pub bar_labels: bool,
    #[serde(default, deserialize_with = "or_default")]
    pub icon_size: IconSize,
    /// The gap between the bar's icons, in logical pixels.
    #[serde(default = "default_icon_spacing")]
    pub icon_spacing: u32,
    /// `#rrggbb`: the window's title bar and the shell's accents (the
    /// selected app, buttons, switches).
    #[serde(default = "default_theme_color")]
    pub theme_color: String,
    /// Desktop notifications from any service; `false` drops them all, whatever
    /// each service's own setting (the bar's bell).
    #[serde(default = "yes")]
    pub notifications: bool,
    /// Sound from any service; `false` mutes every webview, whatever each
    /// service's own setting (the bar's bell).
    #[serde(default = "yes")]
    pub audio: bool,
}

/// Largest `Settings::icon_spacing`; the shell's slider stops here too.
const MAX_ICON_SPACING: u32 = 16;

fn default_icon_spacing() -> u32 {
    2
}

fn default_theme_color() -> String {
    "#5b8cff".to_owned()
}

/// `#rrggbb`, the form `<input type="color">` gives.
fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    #[default]
    Left,
    Top,
    Bottom,
    Right,
}

impl BarPosition {
    /// Top or bottom: the bar spans the window's width.
    pub fn is_horizontal(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// The size of the app icons in the bar, which sets its thickness (see
/// `platform::bar_thickness`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IconSize {
    ExtraSmall,
    Small,
    #[default]
    Medium,
    Large,
    ExtraLarge,
}

/// An unknown value (a hand-edited file, or one from a newer version) is
/// the default rather than an unreadable services.json.
fn or_default<'de, D: serde::Deserializer<'de>, T: Default + serde::de::DeserializeOwned>(
    d: D,
) -> std::result::Result<T, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(T::deserialize(value).unwrap_or_else(|e| {
        log::warn!("services.json: using the default for a setting: {e}");
        T::default()
    }))
}

fn default_hibernate_after() -> u32 {
    15
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hibernate_after_minutes: default_hibernate_after(),
            max_suspended: None,
            close_to_tray: true,
            bar_position: BarPosition::default(),
            bar_labels: false,
            icon_size: IconSize::default(),
            icon_spacing: default_icon_spacing(),
            theme_color: default_theme_color(),
            notifications: true,
            audio: true,
        }
    }
}

impl Settings {
    fn validate(&self) -> Result<()> {
        if self.hibernate_after_minutes > MAX_HIBERNATE_MINUTES {
            return Err(Error::invalid("hibernate after must be at most a week"));
        }
        if self.max_suspended.is_some_and(|n| n > 100) {
            return Err(Error::invalid("background limit must be at most 100"));
        }
        if self.icon_spacing > MAX_ICON_SPACING {
            return Err(Error::invalid(format!("icon spacing must be at most {MAX_ICON_SPACING}px")));
        }
        if !is_hex_color(&self.theme_color) {
            return Err(Error::invalid(format!("theme colour {:?} must be #rrggbb", self.theme_color)));
        }
        Ok(())
    }
}

/// Which web permission requests an instance grants. Geolocation and every
/// other kind are always denied; they are not configurable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Permissions {
    #[serde(default = "yes")]
    pub notifications: bool,
    /// Camera/microphone; only takes effect if the recipe declares `user_media`.
    #[serde(default = "yes")]
    pub user_media: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self { notifications: true, user_media: true }
    }
}

/// A WebKit permission request, as far as the policy cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionKind {
    Notifications,
    /// Camera and/or microphone.
    UserMedia,
    Geolocation,
    /// Anything else, by WebKit type name (for the log).
    Other(&'static str),
}

/// An instance that passed validation, joined with its recipe.
#[derive(Clone, Debug)]
pub struct Service {
    pub instance: Instance,
    pub recipe: &'static Recipe,
    /// The instance's URL, else the recipe's (for its team).
    url: String,
    /// The recipe's domains, or for a self-hosted app the site of its URL.
    allowed: Vec<String>,
}

impl std::fmt::Debug for Recipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id)
    }
}

/// Ids and profiles become webview labels and directory names: one path
/// component of `[a-z0-9_-]`, so no separators, no "." or "..", nothing hidden.
pub fn validate_token(kind: &str, s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'));
    if ok {
        Ok(())
    } else {
        Err(Error::invalid(format!("invalid {kind} {s:?}: use 1-64 of a-z, 0-9, '-', '_'")))
    }
}

pub fn validate_profile(profile: &str) -> Result<()> {
    validate_token("profile", profile)
}

/// Trimmed display name, non-empty and at most 64 characters.
pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(Error::invalid("name must be 1-64 printable characters"));
    }
    Ok(name.to_owned())
}

/// A URL override of a hosted recipe may change the path or subdomain, never
/// the site.
pub fn validate_url_override(recipe: &Recipe, url: &str) -> Result<Url> {
    let parsed = parse_app_url(url)?;
    if parsed.scheme() != "https" {
        return Err(Error::invalid(format!("url {url:?} must use https")));
    }
    if recipe.covers(&parsed) {
        Ok(parsed)
    } else {
        Err(Error::invalid(format!("url {url:?} is outside {}'s domains {:?}", recipe.id, recipe.allowed_domains)))
    }
}

/// A team becomes a subdomain: one DNS label, lowercase.
pub fn validate_team(team: &str) -> Result<()> {
    let ok = !team.is_empty()
        && team.len() <= 63
        && !team.starts_with('-')
        && !team.ends_with('-')
        && team.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(Error::invalid(format!("invalid team {team:?}: use 1-63 of a-z, 0-9 and inner '-'")))
    }
}

/// An http(s) URL with a host and no credentials.
pub fn parse_app_url(url: &str) -> Result<Url> {
    let parsed: Url = url.parse().map_err(|e| Error::invalid(format!("invalid url {url:?}: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none_or(str::is_empty) {
        return Err(Error::invalid(format!("url {url:?} must be an http(s) address")));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Error::invalid(format!("url {url:?} must not contain credentials")));
    }
    Ok(parsed)
}

/// The URL `instance` of `recipe` loads and the domains it may navigate
/// within, as its [`Hosting`] has them.
fn resolve(recipe: &Recipe, instance: &Instance) -> Result<(Url, Vec<String>)> {
    if let (None, Some(team)) = (recipe.team, &instance.team) {
        return Err(Error::invalid(format!("{} has no team, got {team:?}", recipe.name)));
    }
    let own = || recipe.allowed_domains.iter().map(|d| (*d).to_owned()).collect();
    let Some(url) = &instance.url else {
        if recipe.hosting == Hosting::SelfHosted {
            return Err(Error::invalid(format!("{} needs its address", recipe.name)));
        }
        return Ok((recipe.url_for(instance.team.as_deref())?, own()));
    };
    let parsed = parse_app_url(url)?;
    let own_server = match recipe.hosting {
        Hosting::SelfHosted => true,
        Hosting::Both => !recipe.covers(&parsed),
        Hosting::Cloud => false,
    };
    if own_server {
        let site = site_of(&parsed);
        Ok((parsed, vec![site]))
    } else {
        Ok((validate_url_override(recipe, url)?, own()))
    }
}

/// The site a self-hosted app may navigate within: its URL's host without
/// subdomains ("app.slack.com" -> "slack.com"), so logins elsewhere on the
/// site work. Without the public suffix list this is a guess: a short label
/// under a two-letter TLD counts as part of the suffix ("bbc.co.uk", not
/// "co.uk"). IP addresses and single-label hosts are kept whole.
pub fn site_of(url: &Url) -> String {
    let domain = match url.host() {
        Some(Host::Domain(d)) => d.trim_end_matches('.').to_ascii_lowercase(),
        Some(ip) => return ip.to_string(),
        None => return String::new(),
    };
    let labels: Vec<&str> = domain.split('.').collect();
    let n = labels.len();
    let keep = if n >= 3 && labels[n - 1].len() == 2 && labels[n - 2].len() <= 3 { 3 } else { 2 };
    labels[n.saturating_sub(keep)..].join(".")
}

/// Largest accepted icon, as a data: URL (the whole config is rewritten on every change).
pub const MAX_ICON_LEN: usize = 256 * 1024;

/// Icons are shown by the shell as `<img src>`, so only base64 `data:image/`
/// URLs are accepted: a remote URL would have the trusted shell fetch it.
pub fn validate_icon(icon: &str) -> Result<()> {
    let ok = icon.len() <= MAX_ICON_LEN
        && icon.strip_prefix("data:image/").and_then(|r| r.split_once(";base64,")).is_some_and(|(ty, data)| {
            !ty.is_empty()
                && ty.bytes().all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
                && data.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
        });
    if ok {
        Ok(())
    } else {
        Err(Error::invalid(format!("icon must be a base64 image data: URL of at most {} KiB", MAX_ICON_LEN / 1024)))
    }
}

impl Service {
    pub fn new(instance: Instance) -> Result<Self> {
        validate_token("id", &instance.id)?;
        validate_profile(&instance.profile)?;
        validate_name(&instance.name)?;
        let recipe = recipe(&instance.recipe).ok_or_else(|| Error::invalid(format!("unknown recipe {:?}", instance.recipe)))?;
        let (url, allowed) = resolve(recipe, &instance)?;
        if let Some(icon) = &instance.icon {
            validate_icon(icon)?;
        }
        let out_of_range = |s: Option<u32>| s.is_some_and(|s| s == 0 || s > MAX_HIBERNATE_MINUTES * 60);
        if out_of_range(instance.hibernate_after_seconds) {
            return Err(Error::invalid("hibernate after must be between a second and a week"));
        }
        if out_of_range(instance.auto_wake_seconds) {
            return Err(Error::invalid("auto-wake must be between a second and a week"));
        }
        if out_of_range(instance.hibernate_after_wake_seconds) {
            return Err(Error::invalid("hibernate after auto-wake must be between a second and a week"));
        }
        Ok(Self { instance, recipe, url: url.into(), allowed })
    }

    pub fn id(&self) -> &str {
        &self.instance.id
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn allowed_domains(&self) -> &[String] {
        &self.allowed
    }

    /// The instance's own icon, else the recipe's.
    pub fn icon(&self) -> Option<String> {
        self.instance.icon.clone().or_else(|| self.recipe.icon_url())
    }

    pub fn keep_alive(&self) -> bool {
        self.instance.keep_alive.unwrap_or(self.recipe.keep_alive)
    }

    /// The permission policy: deny unless the instance (and, for camera and
    /// microphone, the recipe) allows it.
    pub fn allows(&self, kind: PermissionKind) -> bool {
        let p = &self.instance.permissions;
        match kind {
            PermissionKind::Notifications => p.notifications,
            PermissionKind::UserMedia => self.recipe.user_media && p.user_media,
            PermissionKind::Geolocation | PermissionKind::Other(_) => false,
        }
    }

    /// Script run via `initialization_script`: at document-start on every
    /// top-level navigation, before any page script. Guarded to the service's own
    /// domains because it re-runs on every navigation (e.g. login redirects).
    /// Unless `follow_links` (the platform lets the user's off-site link clicks
    /// through, `platform::FOLLOWS_LINKS`), it makes them `target=_blank`.
    pub fn init_script(&self, follow_links: bool) -> String {
        let hosts = serde_json::to_string(self.allowed_domains()).unwrap();
        let css = serde_json::to_string(self.recipe.inject_css).unwrap();
        let links = if follow_links { "" } else { OFFSITE_LINKS_TO_BLANK };
        format!(
            r#"(function () {{
  var hosts = {hosts};
  var h = location.hostname;
  var own = function (host) {{
    return hosts.some(function (d) {{ return host === d || host.endsWith("." + d); }});
  }};
  if (!own(h)) return;
  {links}
  {js}
  var css = {css};
  if (css.trim()) {{
    var add = function () {{
      var s = document.createElement("style");
      s.setAttribute("data-uhub", "");
      s.textContent = css;
      (document.head || document.documentElement).appendChild(s);
    }};
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", add);
    else add();
  }}
}})();"#,
            js = self.recipe.inject_js
        )
    }
}

/// Part of `init_script`: off-site link clicks become target=_blank so they
/// reach on_new_window (the system browser, or a uHub window); the URL-only
/// navigation policy would just block them. Bubble phase on window, so page
/// handlers run first and can preventDefault.
const OFFSITE_LINKS_TO_BLANK: &str = r#"window.addEventListener("click", function (e) {
    if (e.defaultPrevented || e.button !== 0) return;
    if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
    var a = e.target && e.target.closest && e.target.closest("a[href]");
    if (!a || a.hasAttribute("download")) return;
    var u;
    try { u = new URL(a.href, location.href); } catch (_) { return; }
    if ((u.protocol === "http:" || u.protocol === "https:") && !own(u.hostname)) {
      a.target = "_blank";
    }
  });"#;

/// On-disk format of services.json.
#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    services: Vec<Instance>,
    #[serde(default)]
    settings: Settings,
    /// The service that was on screen last, activated again on launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_active: Option<String>,
}

const FILE_VERSION: u32 = 1;

/// What services.json holds, validated.
#[derive(Clone)]
struct Saved {
    services: Vec<Service>,
    settings: Settings,
    last_active: Option<String>,
}

/// The pre-config hard-coded services, with their original ids and profiles so
/// existing profile directories (and logins) are reused.
pub fn seed() -> Vec<Service> {
    [
        ("whatsapp", "WhatsApp", "whatsapp"),
        ("gmail-personal", "Gmail (Personal)", "gmail"),
        ("gmail-work", "Gmail (Work)", "gmail"),
        ("discord", "Discord", "discord"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (id, name, recipe))| {
        Service::new(Instance::new(id.into(), name.into(), recipe, i as u32)).expect("seed services are valid")
    })
    .collect()
}

/// Parse services.json. Invalid or duplicate entries are skipped (and logged)
/// rather than failing the whole file; the result is sorted by `order`.
/// Invalid settings fall back to the defaults, and a `last_active` naming no
/// service is dropped.
fn parse(json: &str) -> Result<Saved> {
    let file: File = serde_json::from_str(json).map_err(|e| Error::invalid(e.to_string()))?;
    if file.version != FILE_VERSION {
        return Err(Error::invalid(format!("unsupported version {}", file.version)));
    }
    let mut out: Vec<Service> = Vec::new();
    for inst in file.services {
        if out.iter().any(|s| s.instance.id == inst.id) {
            log::warn!("services.json: skipping duplicate id {:?}", inst.id);
            continue;
        }
        match Service::new(inst) {
            Ok(s) => out.push(s),
            Err(e) => log::warn!("services.json: skipping entry: {e}"),
        }
    }
    out.sort_by_key(|s| s.instance.order);
    renumber(&mut out);
    let settings = match file.settings.validate() {
        Ok(()) => file.settings,
        Err(e) => {
            log::warn!("services.json: using default settings: {e}");
            Settings::default()
        }
    };
    let last_active = file.last_active.filter(|id| out.iter().any(|s| s.id() == id));
    Ok(Saved { services: out, settings, last_active })
}

fn renumber(services: &mut [Service]) {
    for (i, s) in services.iter_mut().enumerate() {
        s.instance.order = i as u32;
    }
}

fn unix_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The configured services, settings, and where they are saved. Mutations go
/// through methods that write the file first and only then update memory, so
/// memory never holds a state that isn't on disk.
pub struct Config {
    path: PathBuf,
    saved: Saved,
}

impl Config {
    /// Load `path`, seeding it with [`seed`] if it doesn't exist. An unreadable
    /// file is moved aside (not overwritten) and the seed used instead.
    pub fn load(path: PathBuf) -> Result<Self> {
        let saved = match fs::read_to_string(&path) {
            Ok(json) => match parse(&json) {
                Ok(s) => s,
                Err(e) => {
                    let aside = path.with_extension(format!("json.corrupt-{}", unix_millis()));
                    log::error!("{}: {e}; moving it to {} and starting over", path.display(), aside.display());
                    fs::rename(&path, &aside).map_err(|e| Error::io(format!("moving aside {}", path.display()), e))?;
                    return Self::seeded(path);
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Self::seeded(path),
            Err(e) => return Err(Error::io(format!("reading {}", path.display()), e)),
        };
        Ok(Self { path, saved })
    }

    fn seeded(path: PathBuf) -> Result<Self> {
        log::info!("seeding {}", path.display());
        let saved = Saved { services: seed(), settings: Settings::default(), last_active: None };
        write_atomic(&path, &saved)?;
        Ok(Self { path, saved })
    }

    pub fn services(&self) -> &[Service] {
        &self.saved.services
    }

    pub fn find(&self, id: &str) -> Option<&Service> {
        self.saved.services.iter().find(|s| s.id() == id)
    }

    pub fn settings(&self) -> &Settings {
        &self.saved.settings
    }

    pub fn last_active(&self) -> Option<&str> {
        self.saved.last_active.as_deref()
    }

    fn commit(&mut self, mut next: Saved) -> Result<()> {
        renumber(&mut next.services);
        write_atomic(&self.path, &next)?;
        self.saved = next;
        Ok(())
    }

    /// A copy of the saved state with `id`'s service changed by `f`.
    fn with_service(&self, id: &str, f: impl FnOnce(&mut Instance)) -> Result<Saved> {
        let mut next = self.saved.clone();
        f(&mut next.services.iter_mut().find(|s| s.id() == id).ok_or_else(|| unknown(id))?.instance);
        Ok(next)
    }

    /// Add the app at `new.url` (an instance of the recipe covering it, see
    /// [`recipe_for`]) with a fresh id and its own profile. `profile_taken`
    /// reports leftover profile directories, which are never reused: a new
    /// account must not inherit an old login. Returns the new id.
    pub fn add(&mut self, new: &NewService, profile_taken: impl Fn(&str) -> bool) -> Result<String> {
        let recipe = recipe(&new.recipe).ok_or_else(|| Error::invalid(format!("unknown recipe {:?}", new.recipe)))?;
        let id = fresh_id(recipe.id, unix_millis(), |c| {
            self.services().iter().any(|s| s.instance.id == c || s.instance.profile == c) || profile_taken(c)
        });
        let mut inst = Instance::new(id.clone(), String::new(), recipe.id, self.services().len() as u32);
        inst.apply(new)?;
        let svc = Service::new(inst)?;
        let mut next = self.saved.clone();
        next.services.push(svc);
        self.commit(next)?;
        Ok(id)
    }

    /// Replace `id`'s settings with the "Edit app" form's. It may become an
    /// instance of another recipe; it keeps its id and profile (logins).
    pub fn update(&mut self, id: &str, new: &NewService) -> Result<()> {
        let mut next = self.saved.clone();
        let svc = next.services.iter_mut().find(|s| s.id() == id).ok_or_else(|| unknown(id))?;
        let mut inst = svc.instance.clone();
        inst.apply(new)?;
        *svc = Service::new(inst)?;
        self.commit(next)
    }

    /// Remove `id`, returning it. Its webview must already be gone.
    pub fn remove(&mut self, id: &str) -> Result<Service> {
        let i = self.services().iter().position(|s| s.id() == id).ok_or_else(|| unknown(id))?;
        let mut next = self.saved.clone();
        let removed = next.services.remove(i);
        if next.last_active.as_deref() == Some(id) {
            next.last_active = None;
        }
        self.commit(next)?;
        Ok(removed)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<()> {
        let name = validate_name(name)?;
        let next = self.with_service(id, |inst| inst.name = name)?;
        self.commit(next)
    }

    pub fn set_keep_alive(&mut self, id: &str, keep_alive: bool) -> Result<()> {
        let next = self.with_service(id, |inst| inst.keep_alive = Some(keep_alive))?;
        self.commit(next)
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<()> {
        let next = self.with_service(id, |inst| inst.enabled = enabled)?;
        self.commit(next)
    }

    pub fn set_audio(&mut self, id: &str, audio: bool) -> Result<()> {
        let next = self.with_service(id, |inst| inst.audio = audio)?;
        self.commit(next)
    }

    pub fn set_notifications(&mut self, id: &str, notifications: bool) -> Result<()> {
        let next = self.with_service(id, |inst| inst.permissions.notifications = notifications)?;
        self.commit(next)
    }

    pub fn set_settings(&mut self, settings: Settings) -> Result<()> {
        settings.validate()?;
        let mut next = self.saved.clone();
        next.settings = settings;
        self.commit(next)
    }

    /// Remember `id` as the service to restore on launch. Only writes the file
    /// when it changes.
    pub fn set_last_active(&mut self, id: &str) -> Result<()> {
        if self.last_active() == Some(id) {
            return Ok(());
        }
        let mut next = self.with_service(id, |_| ())?;
        next.last_active = Some(id.to_owned());
        self.commit(next)
    }

    /// `ids` must name every service exactly once.
    pub fn reorder(&mut self, ids: &[String]) -> Result<()> {
        let mut services = Vec::with_capacity(ids.len());
        for id in ids {
            if services.iter().any(|s: &Service| s.id() == id) {
                return Err(Error::invalid(format!("{id:?} listed twice")));
            }
            services.push(self.find(id).ok_or_else(|| unknown(id))?.clone());
        }
        if services.len() != self.services().len() {
            return Err(Error::invalid("reorder must list every service"));
        }
        let next = Saved { services, ..self.saved.clone() };
        self.commit(next)
    }
}

pub fn unknown(id: &str) -> Error {
    Error::UnknownService(id.to_owned())
}

/// `<recipe>-<base36 millis>`, bumped until `taken` says no.
fn fresh_id(recipe: &str, mut n: u64, taken: impl Fn(&str) -> bool) -> String {
    loop {
        let id = format!("{recipe}-{}", base36(n));
        if !taken(&id) {
            return id;
        }
        n += 1;
    }
}

fn base36(mut n: u64) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Write to a sibling temp file, fsync, then rename over `path`, so a crash
/// leaves either the old or the new file, never a torn one.
fn write_atomic(path: &Path, saved: &Saved) -> Result<()> {
    let file = File {
        version: FILE_VERSION,
        services: saved.services.iter().map(|s| s.instance.clone()).collect(),
        settings: saved.settings.clone(),
        last_active: saved.last_active.clone(),
    };
    let err = |e: io::Error| Error::io(format!("saving {}", path.display()), e);
    let json = serde_json::to_string_pretty(&file).map_err(|e| err(e.into()))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(err)?;
    }
    let tmp = path.with_extension("json.tmp");
    let result = (|| {
        use io::Write;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(err)
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    /// Webview exists and is visible.
    Active,
    /// Webview exists but hidden (hide()/show()); page state is kept.
    Suspended,
    /// Webview destroyed; only the on-disk profile remains.
    Unloaded,
    /// Webview exists but its web process died; activating recreates it.
    Crashed,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub id: String,
    pub name: String,
    pub recipe: &'static str,
    pub team: Option<String>,
    pub url: String,
    pub allowed_domains: Vec<String>,
    pub profile: String,
    pub enabled: bool,
    pub icon: Option<String>,
    pub state: Lifecycle,
    /// On screen: active (or crashed there), or hibernated there with the
    /// shell's placeholder in its place (`Registry::hibernate_active`).
    pub shown: bool,
    /// From the page title (see badge.rs); 0 while unloaded.
    pub unread: u32,
    /// Never hibernated (the instance's setting or the recipe's default).
    pub keep_alive: bool,
    /// `None`: the app-wide `Settings::hibernate_after_minutes`.
    pub hibernate_after_seconds: Option<u32>,
    /// `None`: no auto-wake.
    pub auto_wake_seconds: Option<u32>,
    /// `None`: as after any other time in the background.
    pub hibernate_after_wake_seconds: Option<u32>,
    pub notifications: bool,
    pub audio: bool,
    pub media_indicator: bool,
    pub unread_badge: bool,
    pub open_links_inside: bool,
    pub start_hibernated: bool,
    /// The page is playing audio right now (Linux only, see platform/mod.rs).
    pub playing_audio: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecipeInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub allowed_domains: &'static [&'static str],
    /// A data: URL.
    pub icon: Option<String>,
    pub team: Option<TeamField>,
    pub hosting: Hosting,
    pub message: Option<&'static str>,
}

/// [`RECIPES`], in order.
pub fn recipe_infos() -> Vec<RecipeInfo> {
    RECIPES
        .iter()
        .map(|r| RecipeInfo {
            id: r.id,
            name: r.name,
            url: r.url,
            allowed_domains: r.allowed_domains,
            icon: r.icon_url(),
            team: r.team,
            hosting: r.hosting,
            message: r.message,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory under the system temp dir.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uhub-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ids(c: &Config) -> Vec<&str> {
        c.services().iter().map(Service::id).collect()
    }

    #[test]
    fn first_run_seeds_legacy_ids_and_profiles() {
        let path = temp_dir("seed").join("sub/services.json");
        let c = Config::load(path.clone()).unwrap();
        assert_eq!(ids(&c), ["whatsapp", "gmail-personal", "gmail-work", "discord"]);
        for s in c.services() {
            assert_eq!(s.instance.profile, s.instance.id, "existing profile dirs must be reused");
        }
        assert_eq!(c.find("gmail-work").unwrap().url(), "https://mail.google.com/");
        assert!(path.exists(), "seed is written so ids stay stable");
        assert!(!path.with_extension("json.tmp").exists());

        // Second load reads the file instead of re-seeding.
        let mut c = Config::load(path.clone()).unwrap();
        c.rename("discord", "  Chat ").unwrap();
        let c = Config::load(path).unwrap();
        assert_eq!(c.find("discord").unwrap().instance.name, "Chat");
    }

    #[test]
    fn corrupt_file_is_moved_aside_not_overwritten() {
        let dir = temp_dir("corrupt");
        let path = dir.join("services.json");
        fs::write(&path, "{ not json").unwrap();
        let c = Config::load(path.clone()).unwrap();
        assert_eq!(c.services().len(), 4);
        let aside: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.starts_with("services.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(fs::read_to_string(dir.join(&aside[0])).unwrap(), "{ not json");
    }

    #[test]
    fn parse_skips_invalid_entries_and_sorts() {
        let json = r#"{"version":1,"services":[
            {"id":"b","name":"B","recipe":"gmail","profile":"b","order":5},
            {"id":"a","name":"A","recipe":"discord","profile":"a","order":1,"enabled":false},
            {"id":"a","name":"Dup","recipe":"discord","profile":"x","order":0},
            {"id":"c","name":"C","recipe":"nope","profile":"c"},
            {"id":"d","name":"D","recipe":"gmail","profile":"../../etc"},
            {"id":"e","name":"E","recipe":"gmail","profile":"e","url":"https://evil.example/"},
            {"id":"f","name":"F","recipe":"gmail","profile":"f","url":"https://inbox.google.com/u/1/"}
        ]}"#;
        let s = parse(json).unwrap().services;
        let got: Vec<_> = s.iter().map(|s| (s.id(), s.instance.order)).collect();
        assert_eq!(got, [("f", 0), ("a", 1), ("b", 2)]);
        assert!(!s[1].instance.enabled);
        assert!(s[2].instance.enabled, "enabled defaults to true");
        assert_eq!(s[0].url(), "https://inbox.google.com/u/1/");
        assert!(parse(r#"{"version":2,"services":[]}"#).is_err());
    }

    #[test]
    fn profile_validation() {
        for ok in ["whatsapp", "gmail-work", "a_b-9", "x"] {
            assert!(validate_profile(ok).is_ok(), "{ok}");
        }
        let long = "a".repeat(65);
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "/abs", "A", "a b", "é", ".hidden", "a\0", &long] {
            assert!(validate_profile(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn url_override_validation() {
        let gmail = recipe("gmail").unwrap();
        for ok in ["https://mail.google.com/mail/u/1/", "https://google.com", "https://MAIL.Google.com./"] {
            assert!(validate_url_override(gmail, ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://mail.google.com/",
            "https://evil.com/",
            "https://google.com.evil.com/",
            "https://evilgoogle.com/",
            "https://user:pw@mail.google.com/",
            "https://web.whatsapp.com/",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "mail.google.com",
            "https://1.2.3.4/",
        ] {
            assert!(validate_url_override(gmail, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn name_validation() {
        assert_eq!(validate_name("  Work  ").unwrap(), "Work");
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
        assert!(validate_name("a\nb").is_err());
    }

    /// The form for an app of `recipe`; a blank `url` is the recipe's own.
    fn new_app(name: &str, recipe: &str, url: &str) -> NewService {
        NewService {
            name: name.into(),
            recipe: recipe.into(),
            team: None,
            url: Some(url.into()),
            icon: None,
            notifications: true,
            audio: true,
            media_indicator: true,
            unread_badge: true,
            enabled: true,
            hibernation: true,
            hibernate_after_seconds: 15 * 60,
            auto_wake_seconds: None,
            hibernate_after_wake_seconds: 60,
            open_links_inside: false,
            start_hibernated: false,
        }
    }

    #[test]
    fn add_remove_rename_reorder() {
        let path = temp_dir("mutate").join("services.json");
        let mut c = Config::load(path.clone()).unwrap();
        let id = c.add(&new_app("Gmail (Side)", "gmail", "https://mail.google.com/"), |_| false).unwrap();
        assert!(id.starts_with("gmail-"));
        let new = c.find(&id).unwrap();
        assert_eq!(new.instance.profile, id);
        assert_eq!(new.instance.order, 4);
        assert_eq!(new.instance.url, None, "the recipe's own url isn't stored");
        assert!(c.add(&new_app("X", CUSTOM, "ftp://example.com/"), |_| false).is_err());
        assert!(c.add(&new_app(" ", "gmail", ""), |_| false).is_err());
        assert!(c.add(&new_app("X", "nope", ""), |_| false).is_err());

        let mut order: Vec<String> = ids(&c).into_iter().rev().map(String::from).collect();
        c.reorder(&order).unwrap();
        assert_eq!(ids(&c)[0], id);
        assert!(c.reorder(&order[1..]).is_err());
        order[1] = order[0].clone();
        assert!(c.reorder(&order).is_err());

        let removed = c.remove("gmail-work").unwrap();
        assert_eq!(removed.instance.profile, "gmail-work");
        assert!(c.remove("gmail-work").is_err());
        assert!(c.rename("nope", "x").is_err());

        c.set_enabled(&id, false).unwrap();
        c.set_audio(&id, false).unwrap();
        c.set_notifications(&id, false).unwrap();
        let inst = &c.find(&id).unwrap().instance;
        assert!(!inst.enabled && !inst.audio && !inst.permissions.notifications);
        assert!(c.set_audio("nope", false).is_err());

        let reloaded = Config::load(path).unwrap();
        assert_eq!(ids(&reloaded), ids(&c));
        let orders: Vec<_> = reloaded.services().iter().map(|s| s.instance.order).collect();
        assert_eq!(orders, [0, 1, 2, 3]);
    }

    #[test]
    fn permission_policy() {
        use PermissionKind::*;
        let json = r#"{"version":1,"services":[
            {"id":"wa","name":"WA","recipe":"whatsapp","profile":"wa"},
            {"id":"g","name":"G","recipe":"gmail","profile":"g","permissions":{"user_media":true}},
            {"id":"d","name":"D","recipe":"discord","profile":"d","permissions":{"notifications":false,"user_media":false}}
        ]}"#;
        let s = parse(json).unwrap().services;
        let (wa, g, d) = (&s[0], &s[1], &s[2]);
        assert_eq!(wa.instance.permissions, Permissions::default(), "missing => defaults");
        assert!(wa.allows(Notifications) && wa.allows(UserMedia));
        assert!(g.allows(Notifications));
        assert!(!g.allows(UserMedia), "recipe doesn't declare user media");
        assert!(!d.allows(Notifications) && !d.allows(UserMedia), "instance opted out");
        for svc in [wa, g, d] {
            assert!(!svc.allows(Geolocation));
            assert!(!svc.allows(Other("WebKitPointerLockPermissionRequest")));
        }
    }

    #[test]
    fn keep_alive_defaults_by_recipe_and_can_be_overridden() {
        let path = temp_dir("keepalive").join("services.json");
        let mut c = Config::load(path.clone()).unwrap();
        let keep = |c: &Config, id| c.find(id).unwrap().keep_alive();
        assert!(keep(&c, "whatsapp") && keep(&c, "discord"));
        assert!(!keep(&c, "gmail-work"));

        c.set_keep_alive("whatsapp", false).unwrap();
        c.set_keep_alive("gmail-work", true).unwrap();
        assert!(c.set_keep_alive("nope", true).is_err());
        let c = Config::load(path).unwrap();
        assert!(!keep(&c, "whatsapp") && keep(&c, "gmail-work"));
        assert!(keep(&c, "discord"), "untouched: still the recipe default");
    }

    #[test]
    fn settings_and_last_active_persist() {
        let path = temp_dir("settings").join("services.json");
        let mut c = Config::load(path.clone()).unwrap();
        assert_eq!(*c.settings(), Settings::default());
        assert_eq!(c.settings().hibernate_after_minutes, 15);
        assert_eq!(c.last_active(), None);

        let s = Settings {
            hibernate_after_minutes: 30,
            max_suspended: Some(2),
            close_to_tray: false,
            bar_position: BarPosition::Bottom,
            bar_labels: true,
            icon_size: IconSize::Large,
            icon_spacing: 8,
            theme_color: "#22c55e".to_owned(),
            notifications: false,
            audio: false,
        };
        c.set_settings(s.clone()).unwrap();
        assert!(c.set_settings(Settings { max_suspended: Some(1000), ..s.clone() }).is_err());
        assert!(c.set_settings(Settings { hibernate_after_minutes: u32::MAX, ..s.clone() }).is_err());
        assert!(c.set_settings(Settings { icon_spacing: 100, ..s.clone() }).is_err());
        for bad in ["22c55e", "#22c55", "#22c55eff", "#ggg000", "red"] {
            assert!(c.set_settings(Settings { theme_color: bad.to_owned(), ..s.clone() }).is_err(), "{bad}");
        }
        c.set_last_active("gmail-work").unwrap();
        assert!(c.set_last_active("nope").is_err());

        let mut c = Config::load(path.clone()).unwrap();
        assert_eq!(*c.settings(), s);
        assert_eq!(c.last_active(), Some("gmail-work"));

        // Removing the remembered service forgets it.
        c.remove("gmail-work").unwrap();
        assert_eq!(c.last_active(), None);
        assert_eq!(Config::load(path).unwrap().last_active(), None);
    }

    #[test]
    fn parse_tolerates_missing_or_bad_settings() {
        let old = r#"{"version":1,"services":[{"id":"a","name":"A","recipe":"gmail","profile":"a"}]}"#;
        let saved = parse(old).unwrap();
        assert_eq!(saved.settings, Settings::default(), "files from before settings existed");
        assert_eq!(saved.last_active, None);

        let json = r#"{"version":1,"services":[{"id":"a","name":"A","recipe":"gmail","profile":"a"}],
            "settings":{"hibernate_after_minutes":0,"max_suspended":100000},"last_active":"gone"}"#;
        let saved = parse(json).unwrap();
        assert_eq!(saved.settings, Settings::default(), "out of range: defaults");
        assert_eq!(saved.last_active, None, "names no service");

        let json = r#"{"version":1,"services":[{"id":"a","name":"A","recipe":"gmail","profile":"a"}],
            "settings":{"hibernate_after_minutes":0},"last_active":"a"}"#;
        let saved = parse(json).unwrap();
        assert_eq!(saved.settings.hibernate_after_minutes, 0);
        assert!(saved.settings.close_to_tray, "missing field: its default");
        assert_eq!(saved.settings.bar_position, BarPosition::Left);
        assert_eq!(saved.last_active.as_deref(), Some("a"));

        let json = r#"{"version":1,"services":[{"id":"a","name":"A","recipe":"gmail","profile":"a"}],
            "settings":{"bar_position":"diagonal","icon_size":"extra-large"}}"#;
        let saved = parse(json).unwrap();
        assert_eq!(saved.settings.bar_position, BarPosition::Left, "unknown value: its default");
        assert_eq!(saved.settings.icon_size, IconSize::ExtraLarge);
    }

    #[test]
    fn add_custom_keeps_options() {
        let path = temp_dir("add-url").join("services.json");
        let mut c = Config::load(path.clone()).unwrap();

        let wa = c.add(&new_app("WA 2", "whatsapp", ""), |_| false).unwrap();
        let wa = c.find(&wa).unwrap();
        assert_eq!((wa.recipe.id, wa.url()), ("whatsapp", "https://web.whatsapp.com/"));
        // A custom app stays custom, whatever its site.
        let plain = c.add(&new_app("WA http", CUSTOM, "http://web.whatsapp.com/"), |_| false).unwrap();
        let plain = c.find(&plain).unwrap();
        assert_eq!((plain.recipe.id, plain.allowed_domains()), (CUSTOM, &["whatsapp.com".to_owned()][..]));
        assert!(c.add(&new_app("X", CUSTOM, " "), |_| false).is_err(), "custom needs a url");

        let mut opts = new_app("Slack", CUSTOM, " https://app.slack.com/client ");
        opts.icon = Some("data:image/png;base64,iVBORw0KGgo=".into());
        opts.audio = false;
        opts.hibernation = false;
        opts.hibernate_after_seconds = 30;
        opts.auto_wake_seconds = Some(600);
        opts.hibernate_after_wake_seconds = 90;
        opts.open_links_inside = true;
        opts.start_hibernated = true;
        opts.enabled = false;
        let id = c.add(&opts, |_| false).unwrap();
        assert!(id.starts_with("custom-"));
        let slack = c.find(&id).unwrap();
        assert_eq!(slack.url(), "https://app.slack.com/client");
        assert_eq!(slack.allowed_domains(), ["slack.com"]);
        assert!(slack.keep_alive(), "hibernation off: kept alive");
        assert!(slack.init_script(false).contains(r#"["slack.com"]"#));
        assert!(slack.init_script(false).contains("a.target = \"_blank\""));
        assert!(!slack.init_script(true).contains("_blank"));

        let reloaded = Config::load(path).unwrap();
        let inst = &reloaded.find(&id).unwrap().instance;
        assert_eq!(inst, &slack.instance);
        assert!(!inst.audio && !inst.enabled && inst.open_links_inside && inst.start_hibernated);
        assert_eq!(inst.hibernate_after_seconds, Some(30));
        assert_eq!((inst.auto_wake_seconds, inst.hibernate_after_wake_seconds), (Some(600), Some(90)));

        let mut bad = new_app("X", CUSTOM, "https://x.example/");
        bad.icon = Some("https://x.example/favicon.ico".into());
        assert!(c.add(&bad, |_| false).is_err(), "remote icon");
        let mut bad = new_app("X", CUSTOM, "https://x.example/");
        bad.hibernate_after_seconds = 0;
        assert!(c.add(&bad, |_| false).is_err());
        let mut bad = new_app("X", CUSTOM, "https://x.example/");
        bad.auto_wake_seconds = Some(0);
        assert!(c.add(&bad, |_| false).is_err());
        let mut bad = new_app("X", CUSTOM, "https://x.example/");
        bad.hibernate_after_wake_seconds = 0;
        assert!(c.add(&bad, |_| false).is_err());
        assert!(c.add(&new_app("X", CUSTOM, "https://user:pw@x.example/"), |_| false).is_err());
    }

    #[test]
    fn update_keeps_id_and_profile_and_may_change_recipe() {
        let path = temp_dir("update").join("services.json");
        let mut c = Config::load(path.clone()).unwrap();
        let mut opts = new_app("Mail", "gmail", "https://mail.google.com/mail/u/1/");
        opts.audio = false;
        opts.hibernation = false;
        c.update("gmail-work", &opts).unwrap();
        let svc = c.find("gmail-work").unwrap();
        assert_eq!((svc.instance.name.as_str(), svc.recipe.id), ("Mail", "gmail"));
        assert_eq!(svc.url(), "https://mail.google.com/mail/u/1/");
        assert!(!svc.instance.audio && svc.keep_alive());

        assert!(c.update("gmail-work", &new_app("Mail", "gmail", "https://app.slack.com/")).is_err(), "off the recipe's site");
        c.update("gmail-work", &new_app("Slack", CUSTOM, "https://app.slack.com/")).unwrap();
        let svc = c.find("gmail-work").unwrap();
        assert_eq!((svc.recipe.id, svc.instance.profile.as_str()), (CUSTOM, "gmail-work"));
        assert_eq!(svc.allowed_domains(), ["slack.com"]);

        assert!(c.update("gmail-work", &new_app(" ", CUSTOM, "https://app.slack.com/")).is_err());
        assert!(c.update("nope", &new_app("X", CUSTOM, "https://x.example/")).is_err());
        let reloaded = Config::load(path).unwrap();
        assert_eq!(reloaded.find("gmail-work").unwrap().instance, c.find("gmail-work").unwrap().instance);
    }

    #[test]
    fn custom_entries_need_a_url() {
        let json = r#"{"version":1,"services":[
            {"id":"a","name":"A","recipe":"custom","profile":"a"},
            {"id":"b","name":"B","recipe":"custom","profile":"b","url":"http://192.168.1.10:8123/"}
        ]}"#;
        let s = parse(json).unwrap().services;
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].allowed_domains(), ["192.168.1.10"]);
    }

    #[test]
    fn team_recipes_build_their_url() {
        let path = temp_dir("team").join("services.json");
        let mut c = Config::load(path.clone()).unwrap();
        let mut opts = new_app("Acme", "slack", "");
        opts.team = Some(" Acme ".into());
        let id = c.add(&opts, |_| false).unwrap();
        let slack = c.find(&id).unwrap();
        assert_eq!(slack.instance.team.as_deref(), Some("acme"));
        assert_eq!(slack.instance.url, None);
        assert_eq!(slack.url(), "https://acme.slack.com/");
        assert!(slack.allowed_domains().iter().any(|d| d == "slack-edge.com"));
        assert_eq!(Config::load(path).unwrap().find(&id).unwrap().url(), "https://acme.slack.com/");

        for bad in [None, Some(" "), Some("a.b"), Some("-acme"), Some("acme-"), Some("évil"), Some("a/b")] {
            let mut opts = new_app("Acme", "slack", "");
            opts.team = bad.map(String::from);
            assert!(c.add(&opts, |_| false).is_err(), "{bad:?}");
        }
        // A team on a recipe without one is refused, not silently dropped from files.
        let json = r#"{"version":1,"services":[{"id":"a","name":"A","recipe":"gmail","profile":"a","team":"x"}]}"#;
        assert!(parse(json).unwrap().services.is_empty());
    }

    #[test]
    fn hosting_decides_where_an_instance_may_point() {
        let path = temp_dir("hosting").join("services.json");
        let mut c = Config::load(path).unwrap();
        let mut add = |recipe: &str, url: &str| c.add(&new_app("X", recipe, url), |_| false).map(|id| c.find(&id).unwrap().clone());

        // The recipe's own URL isn't stored, however it's written.
        let outlook = add("outlook", "https://outlook.cloud.microsoft/mail/").unwrap();
        assert_eq!(outlook.instance.url, None);
        let outlook = add("outlook", "https://outlook.office.com/mail/").unwrap();
        assert_eq!(outlook.url(), "https://outlook.office.com/mail/");
        assert!(outlook.allowed_domains().iter().any(|d| d == "microsoftonline.com"));
        // Hosted or not: an address off the recipe's site is the user's own server.
        let exchange = add("outlook", "https://mail.example.org/owa/").unwrap();
        assert_eq!(exchange.allowed_domains(), ["example.org"]);
        assert!(add("gmail", "https://mail.example.org/").is_err(), "cloud only");

        assert!(add("mattermost", "").is_err(), "self-hosted needs a url");
        let mm = add("mattermost", "http://chat.example.org:8065/").unwrap();
        assert_eq!((mm.url(), mm.allowed_domains()), ("http://chat.example.org:8065/", &["example.org".to_owned()][..]));
    }

    #[test]
    fn recipes_are_well_formed() {
        let infos = recipe_infos();
        assert_eq!(infos.len(), RECIPES.len());
        assert_eq!(infos.last().unwrap().id, CUSTOM, "offered last");
        for (i, r) in RECIPES.iter().enumerate() {
            validate_token("recipe id", r.id).unwrap();
            assert!(RECIPES[..i].iter().all(|o| o.id != r.id), "{} twice", r.id);
            assert_eq!(r.url.contains("{team}"), r.team.is_some(), "{}", r.id);
            if let Some(icon) = r.icon_url() {
                validate_icon(&icon).unwrap_or_else(|e| panic!("{}: {e}", r.id));
            }
            match (r.hosting, r.team) {
                (Hosting::SelfHosted, _) => assert!(r.url.is_empty() && r.allowed_domains.is_empty(), "{}", r.id),
                (_, Some(_)) => assert!(r.covers(&r.url_for(Some("acme")).unwrap()), "{}", r.id),
                (_, None) => assert!(r.covers(&r.url_for(None).unwrap()), "{}: url off its domains", r.id),
            }
        }
    }

    #[test]
    fn site_guess() {
        let site = |u: &str| site_of(&u.parse().unwrap());
        assert_eq!(site("https://app.slack.com/x"), "slack.com");
        assert_eq!(site("https://slack.com/"), "slack.com");
        assert_eq!(site("https://news.bbc.co.uk/"), "bbc.co.uk");
        assert_eq!(site("https://a.b.example.com.au/"), "example.com.au");
        assert_eq!(site("https://www.example.de/"), "example.de");
        assert_eq!(site("https://WWW.Example.COM./"), "example.com");
        assert_eq!(site("http://localhost:3000/"), "localhost");
        assert_eq!(site("http://10.0.0.2/"), "10.0.0.2");
        assert_eq!(site("http://[::1]:8080/"), "[::1]");
    }

    #[test]
    fn icon_validation() {
        for ok in ["data:image/png;base64,iVBORw0KGgo=", "data:image/svg+xml;base64,PHN2Zz4=", "data:image/x-icon;base64,AAAB"] {
            assert!(validate_icon(ok).is_ok(), "{ok}");
        }
        let huge = format!("data:image/png;base64,{}", "A".repeat(MAX_ICON_LEN));
        for bad in ["", "https://x/y.png", "data:text/html;base64,PGI+", "data:image/png,<svg>", "data:image/;base64,AA", "data:image/png;base64,A\"\"", &huge] {
            assert!(validate_icon(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn fresh_ids_skip_taken_and_are_valid() {
        let id = fresh_id("gmail", 35, |c| c == "gmail-z");
        assert_eq!(id, "gmail-10");
        assert!(validate_token("id", &fresh_id("whatsapp", u64::MAX, |_| false)).is_ok());
        assert_eq!(base36(0), "0");
    }
}
