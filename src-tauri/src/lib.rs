mod badge;
mod error;
mod favicon;
mod lifecycle;
mod navigation;
mod platform;
mod plugins;
mod services;
mod tray;
mod webviews;

use tauri::{webview::WebviewBuilder, window::WindowBuilder, LogicalPosition, LogicalSize, Manager, WebviewUrl};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,tauri_app_lib=debug"))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Used from Rust only (webviews.rs); no capability grants it to any webview.
        .plugin(plugins::without_init_script(tauri_plugin_notification::init()))
        .invoke_handler(tauri::generate_handler![
            webviews::list_services,
            webviews::activate_service,
            webviews::unload_service,
            webviews::list_recipes,
            webviews::add_service,
            webviews::inspect_site,
            webviews::set_overlay,
            webviews::remove_service,
            webviews::rename_service,
            webviews::reorder_services,
            webviews::set_keep_alive,
            webviews::get_settings,
            webviews::set_settings,
            webviews::open_devtools,
        ])
        .on_window_event(|window, event| {
            tray::hide_on_close(window, event);
            platform::on_window_event(window, event);
        })
        .setup(|app| {
            // Before the shell exists, so no command can run without it.
            let config_path = app.path().app_config_dir()?.join("services.json");
            let mut services = webviews::Services::load(config_path)?;
            let close_to_tray = services.close_to_tray();
            app.manage(services);
            webviews::activate_on_notification_click(app.handle())?;
            // Without a tray (e.g. no appindicator library), closing the window quits.
            if let Err(e) = tray::setup(app.handle(), close_to_tray) {
                log::error!("tray unavailable: {e}");
            }

            let window = WindowBuilder::new(app, "main")
                .title("uHub")
                .inner_size(1280., 800.)
                .min_inner_size(640., 400.)
                .build()?;

            // The trusted React shell: the only webview that gets IPC (see capabilities/default.json).
            // Transparent, so a service shows through while the shell covers it
            // with a modal (platform::set_overlay); the sidebar paints its own
            // background. No native drag-and-drop handler, so the page gets
            // HTML5 drops (custom app icons).
            let shell = WebviewBuilder::new("shell", WebviewUrl::App("index.html".into()))
                .transparent(true)
                .disable_drag_drop_handler()
                .on_navigation(|url| {
                    let ok = navigation::shell_navigation_allowed(url);
                    if !ok {
                        log::warn!("blocked shell navigation to {url}");
                    }
                    ok
                })
                .devtools(cfg!(debug_assertions));
            let shell = window.add_child(
                shell,
                LogicalPosition::new(0., 0.),
                LogicalSize::new(f64::from(platform::SIDEBAR_WIDTH), 800.),
            )?;
            platform::place_shell(&shell)?;
            webviews::spawn_hibernation(app.handle());
            webviews::restore_last_active(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
