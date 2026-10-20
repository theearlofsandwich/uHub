# uHub (POC)

Feasibility test: Tauri 2 + WebKitGTK as a lightweight Ferdium/Rambox on Linux.
Also compiles for macOS and Windows; what's missing there is listed in
`src-tauri/src/platform/mod.rs`.

    npm install
    npm run tauri dev          # RUST_LOG=tauri_app_lib=trace for more logs
    npm run check              # tsc + cargo check + cargo test + clippy

Layout: `src-tauri/src/{services,webviews,navigation}.rs` and
`src-tauri/src/platform/` (per-OS webview layout and native events); per-recipe
injection in `services/<recipe>/inject.{js,css}` (embedded at compile time);
React shell in `src/`. Configured services (instances of a recipe: name,
profile, optional URL override, order) live in
`~/.config/com.iflo.tauri-app/services.json`, seeded on first run.
"Add app" in the sidebar adds any http(s) site: one that a recipe covers
(e.g. web.whatsapp.com) becomes an instance of it, anything else a `custom`
instance allowed to navigate within its URL's site (`services::site_of`). The
form pre-fills the site's name and favicon (fetched from Rust, `favicon.rs`),
and stores per-app `audio`, `media_indicator`, `unread_badge`,
`hibernate_after_minutes`, `open_links_inside` and an `icon` data: URL; see
`Instance` in services.rs.
Each instance's `"permissions": {"notifications": true, "user_media": true}`
(the defaults) is its web permission policy; camera/microphone also need the
recipe to declare `user_media`, geolocation and everything else are denied.
