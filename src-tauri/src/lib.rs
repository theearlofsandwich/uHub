mod badge;
mod error;
mod favicon;
mod lifecycle;
mod navigation;
mod performance;
mod platform;
mod plugins;
mod services;
mod shortcuts;
mod tray;
mod webviews;

use tauri::{webview::WebviewBuilder, window::WindowBuilder, LogicalPosition, LogicalSize, Manager, WebviewUrl};
use tauri_plugin_window_state::StateFlags;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,tauri_app_lib=debug"))
        .init();

    tauri::Builder::default()
        // First, so a second launch exits before touching services.json or
        // the webview profiles; it brings up the running instance instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_window(app)))
        // The main window's size, position and maximized state. Not whether
        // it is visible: quitting from the tray while hidden must not start
        // the next run hidden.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::all() - StateFlags::VISIBLE)
                .with_filter(|label| label == "main")
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        // Used from Rust only (get_autostart/set_autostart), like notifications.
        .plugin(plugins::without_init_script(tauri_plugin_autostart::Builder::new().build()))
        // Used from Rust only (webviews.rs); no capability grants it to any webview.
        .plugin(plugins::without_init_script(tauri_plugin_notification::init()))
        .invoke_handler(tauri::generate_handler![
            webviews::list_services,
            webviews::activate_service,
            webviews::wake_service,
            webviews::set_hibernated,
            webviews::list_recipes,
            webviews::add_service,
            webviews::inspect_site,
            webviews::set_overlay,
            webviews::remove_service,
            webviews::rename_service,
            webviews::reorder_services,
            webviews::set_keep_alive,
            webviews::set_enabled,
            webviews::set_audio,
            webviews::set_notifications,
            webviews::update_service,
            webviews::go_home,
            webviews::reload_service,
            webviews::get_settings,
            webviews::set_settings,
            webviews::get_autostart,
            webviews::set_autostart,
            webviews::open_devtools,
            performance::get_performance,
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
            platform::set_title_bar_color(app.handle(), &services.theme_color())?;
            platform::set_bar::<tauri::Wry>(services.bar(), None)?;
            app.manage(services);
            webviews::activate_on_notification_click(app.handle())?;
            // Without a tray (e.g. no appindicator library), closing the window quits.
            if let Err(e) = tray::setup(app.handle(), close_to_tray) {
                log::error!("tray unavailable: {e}");
            }

            // The default size; tauri_plugin_window_state restores the last one.
            let window = WindowBuilder::new(app, "main")
                .title("")
                .inner_size(1280., 800.)
                .min_inner_size(640., 400.)
                .build()?;
            let handle = app.handle().clone();
            platform::watch_shortcuts(&window, move |shortcut| webviews::on_shortcut(&handle, shortcut))?;

            // The trusted React shell: the only webview that gets IPC (see capabilities/default.json).
            // Transparent, so a service shows through while the shell covers it
            // with a modal (platform::set_overlay); the bar paints its own
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
            // Placed by platform::place_shell.
            let (x, y, width, height) = platform::bar().rect(1280., 800.);
            let shell =
                window.add_child(shell, LogicalPosition::new(x, y), LogicalSize::new(width, height))?;
            platform::place_shell(&shell)?;
            webviews::spawn_hibernation(app.handle());
            webviews::start_services(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
