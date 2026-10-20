//! Service webview lifecycle: create (ACTIVE), hide on switch (SUSPENDED, page
//! state kept so switching back doesn't reload), destroy on unload (UNLOADED),
//! and CRASHED when WebKit reports the web process died.
//!
//! Service pages have no IPC. What they report reaches us through native
//! webview events instead (platform/): the title (unread badge), whether audio
//! is playing (media indicator), web notifications (sent on as desktop
//! notifications) and permission requests (decided by the instance's policy,
//! see `Service::allows`). Not every platform reports all of them, see
//! platform/mod.rs. Sound and notifications can be switched while the page is
//! loaded; any other change to how a service's webview is built (its URL,
//! say) recreates it.
//!
//! Hibernation: a background task unloads SUSPENDED services that aren't kept
//! alive once they have been idle longer than their own or the app-wide limit,
//! or exceed the background cap (`Settings`), taking the same lock as every
//! switch so it can't race one.
//!
//! Also hosts the commands that edit the service configuration and settings
//! (services.rs), the ones behind the shell's "Add app" and "Edit app" modals,
//! and those of a service's context menu (go home, reload).
//!
//! Every change is pushed to the shell as a "services-changed" event carrying
//! the full snapshot, emitted while the registry lock is held so events arrive
//! in transition order.

use crate::{
    error::{Error, Result},
    favicon::{self, SiteInfo},
    lifecycle::{Backend, Hibernate, Registry},
    navigation,
    platform::{self, ServiceEvents},
    services::{self, Config, Lifecycle, NewService, PermissionKind, RecipeInfo, Service, ServiceInfo, Settings},
    tray,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{
    async_runtime::{self, Mutex},
    webview::{PageLoadEvent, WebviewBuilder},
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, Runtime, State, Webview,
    WebviewUrl, Wry,
};
use tauri_plugin_notification::NotificationExt;

pub const SERVICES_CHANGED: &str = "services-changed";
/// Key of the desktop notification's `extra` payload naming the service it came from.
const NOTIFICATION_SERVICE: &str = "service";

/// The configured services and their webviews (see lifecycle.rs). An async
/// mutex, held for a whole transition, so overlapping commands (quick clicks)
/// run one after another. It is never locked from a sync command: those run on
/// the main thread, which the webview calls inside a transition wait on.
/// Concrete over Wry because `generate_handler!` can't infer a runtime from
/// `State` alone.
pub struct Services(Mutex<Inner>);

struct Inner {
    reg: Registry<SvcWebview>,
    config: Config,
}

impl Services {
    pub fn load(config_path: PathBuf) -> Result<Self> {
        let config = Config::load(config_path)?;
        Ok(Self(Mutex::new(Inner { reg: Registry::default(), config })))
    }

    /// Before it is shared: the setting the tray starts with.
    pub fn close_to_tray(&mut self) -> bool {
        self.0.get_mut().config.settings().close_to_tray
    }

    /// Before it is shared: where the shell is placed.
    pub fn bar(&mut self) -> platform::Bar {
        let settings = self.0.get_mut().config.settings();
        platform::Bar::new(settings.bar_position, settings.icon_size)
    }
}

/// A service webview plus a per-creation number, so a crash report about a
/// webview that has since been destroyed (and its label reused) is ignored,
/// and its event receiver, which holds the settings that apply live.
pub struct SvcWebview<R: Runtime = Wry> {
    webview: Webview<R>,
    generation: u64,
    events: Arc<Reporter<R>>,
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

fn label(id: &str) -> String {
    format!("svc-{id}")
}

fn profiles_root<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
    Ok(app.path().app_local_data_dir()?.join("profiles"))
}

/// The profile's data directory. Validated again here, right before it becomes
/// a path, even though the config only ever holds validated profiles.
fn profile_dir<R: Runtime>(app: &AppHandle<R>, profile: &str) -> Result<PathBuf> {
    services::validate_profile(profile)?;
    Ok(profiles_root(app)?.join(profile))
}

/// Push the current state to the shell (and only the shell), and the unread
/// total of the services that show it to the tray.
fn emit_snapshot<R: Runtime>(app: &AppHandle<R>, inner: &Inner) {
    let snapshot = inner.reg.snapshot(inner.config.services());
    tray::show_unread(app, snapshot.iter().filter(|s| s.unread_badge).fold(0, |n: u32, s| n.saturating_add(s.unread)));
    if let Err(e) = app.emit_to(EventTarget::webview("shell"), SERVICES_CHANGED, snapshot) {
        log::error!("emitting {SERVICES_CHANGED} failed: {e}");
    }
}

/// Runs on the main thread, so it must not wait for the registry lock (a
/// transition holding it may be waiting on the main thread): hand off instead.
fn on_web_process_terminated<R: Runtime>(app: &AppHandle<R>, id: String, generation: u64, reason: &str) {
    log::error!("[{id}] web process terminated: {reason}");
    let app = app.clone();
    async_runtime::spawn(async move {
        let state = app.state::<Services>();
        let mut inner = state.0.lock().await;
        if inner.reg.mark_crashed(&id, |h| h.generation == generation) {
            emit_snapshot(&app, &inner);
        }
    });
}

/// Main thread (title change): the parse and the dedup happen here, only a
/// changed count is handed off to the registry.
fn on_unread<R: Runtime>(app: &AppHandle<R>, id: String, generation: u64, count: u32) {
    log::debug!("[{id}] unread: {count}");
    let app = app.clone();
    async_runtime::spawn(async move {
        let state = app.state::<Services>();
        let mut inner = state.0.lock().await;
        if inner.reg.set_unread(&id, count, |h| h.generation == generation) {
            emit_snapshot(&app, &inner);
        }
    });
}

/// Main thread (audio started or stopped): handed off like `on_unread`.
fn on_playing_audio<R: Runtime>(app: &AppHandle<R>, id: String, generation: u64, playing: bool) {
    log::debug!("[{id}] playing audio: {playing}");
    let app = app.clone();
    async_runtime::spawn(async move {
        let state = app.state::<Services>();
        let mut inner = state.0.lock().await;
        if inner.reg.set_playing(&id, playing, |h| h.generation == generation) {
            emit_snapshot(&app, &inner);
        }
    });
}

/// Main thread (web notification): only the strings leave it. The current
/// service name (it may have been renamed) is looked up off the main thread.
fn on_show_notification<R: Runtime>(app: &AppHandle<R>, id: String, title: String, body: String) {
    log::info!("[{id}] notification: {title:?}");
    let app = app.clone();
    async_runtime::spawn(async move {
        let name = {
            let state = app.state::<Services>();
            let inner = state.0.lock().await;
            match inner.config.find(&id) {
                Some(svc) => svc.instance.name.clone(),
                None => return,
            }
        };
        let title = if title.is_empty() { name } else { format!("{name}: {title}") };
        let shown =
            app.notification().builder().title(title).body(body).extra(NOTIFICATION_SERVICE, &id).show();
        if let Err(e) = shown {
            log::error!("[{id}] showing notification failed: {e}");
        }
    });
}

/// Main thread (permission request): decided synchronously from the policy
/// captured at creation, as updated since (see `Reporter::allows`).
fn on_permission_request<R: Runtime>(events: &Reporter<R>, kind: PermissionKind) -> bool {
    let allowed = events.allows(kind);
    log::info!("[{}] permission {kind:?}: {}", events.svc.id(), if allowed { "allowed" } else { "denied" });
    allowed
}

/// Receives one service webview's native events (see platform/).
struct Reporter<R: Runtime> {
    app: AppHandle<R>,
    svc: Service,
    generation: u64,
    /// The unread count last handed off, so only changes are.
    last_unread: std::sync::Mutex<Option<u32>>,
    /// `svc`'s notification permission, switchable while the page is loaded.
    notifications: AtomicBool,
}

impl<R: Runtime> Reporter<R> {
    fn allows(&self, kind: PermissionKind) -> bool {
        match kind {
            PermissionKind::Notifications => self.notifications.load(Ordering::Relaxed),
            _ => self.svc.allows(kind),
        }
    }
}

impl<R: Runtime> ServiceEvents for Reporter<R> {
    fn watches_title(&self) -> bool {
        self.svc.recipe.badge.is_some()
    }

    fn title_changed(&self, title: &str) {
        let Some(parse) = self.svc.recipe.badge else { return };
        if let Some(n) = parse(title)
            && self.last_unread.lock().unwrap_or_else(|e| e.into_inner()).replace(n) != Some(n)
        {
            on_unread(&self.app, self.svc.id().to_owned(), self.generation, n);
        }
    }

    fn crashed(&self, reason: &str) {
        on_web_process_terminated(&self.app, self.svc.id().to_owned(), self.generation, reason);
    }

    fn notification(&self, title: String, body: String) {
        // The permission was denied, so the page shouldn't get here; if it does, drop it.
        if self.allows(PermissionKind::Notifications) {
            on_show_notification(&self.app, self.svc.id().to_owned(), title, body);
        }
    }

    fn permission(&self, kind: PermissionKind) -> bool {
        on_permission_request(self, kind)
    }

    fn watches_audio(&self) -> bool {
        self.svc.instance.media_indicator
    }

    fn playing_audio(&self, playing: bool) {
        on_playing_audio(&self.app, self.svc.id().to_owned(), self.generation, playing);
    }
}

fn create<R: Runtime>(app: &AppHandle<R>, svc: &Service) -> Result<SvcWebview<R>> {
    let id = svc.id().to_owned();
    let window = app.get_window("main").ok_or(Error::WebviewMissing)?;
    let data_dir = profile_dir(app, &svc.instance.profile)?;
    std::fs::create_dir_all(&data_dir).map_err(|e| Error::io(format!("creating {}", data_dir.display()), e))?;
    log::info!("[{id}] creating webview (profile dir {})", data_dir.display());

    let url = svc.url().parse().map_err(|e| Error::CreateFailed(format!("bad url: {e}")))?;
    let log_id = id.clone();
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    let events = Arc::new(Reporter {
        app: app.clone(),
        svc: svc.clone(),
        generation,
        last_unread: Default::default(),
        notifications: AtomicBool::new(svc.allows(PermissionKind::Notifications)),
    });
    let links_inside = svc.instance.open_links_inside.then(|| data_dir.clone());
    let mut builder = WebviewBuilder::new(label(&id), WebviewUrl::External(url))
        // Distinct directory => distinct WebKit WebContext + WebsiteDataManager.
        .data_directory(data_dir)
        .initialization_script(svc.init_script())
        .on_navigation(navigation::on_navigation(id.clone(), svc.allowed_domains().to_vec()))
        .on_new_window(navigation::on_new_window(app.clone(), id.clone(), links_inside))
        .on_page_load(move |_wv, p| match p.event() {
            PageLoadEvent::Started => log::debug!("[{log_id}] load started: {}", p.url()),
            PageLoadEvent::Finished => log::debug!("[{log_id}] load finished: {}", p.url()),
        })
        .devtools(cfg!(debug_assertions));
    if let Some(ua) = svc.recipe.user_agent {
        builder = builder.user_agent(ua);
    }

    let builder = platform::service_builder(builder, &events);

    // Placed by platform::attach_service.
    let webview = window
        .add_child(builder, LogicalPosition::new(0., 0.), LogicalSize::new(1., 1.))
        .map_err(|e| Error::CreateFailed(format!("add_child failed: {e}")))?;
    // From here on the label is taken: don't leak the webview on failure.
    if let Err(e) = platform::attach_service(&webview, events.clone(), !svc.instance.audio) {
        log::error!("[{id}] placing webview failed, closing it: {e}");
        if let Err(e) = webview.close() {
            log::error!("[{id}] close failed: {e}");
        }
        return Err(Error::CreateFailed(e.to_string()));
    }
    Ok(SvcWebview { webview, generation, events })
}

/// Real webviews: suspend = hide(), resume = show(), unload = close().
struct Webviews<'a, R: Runtime>(&'a AppHandle<R>);

impl<R: Runtime> Backend for Webviews<'_, R> {
    type Handle = SvcWebview<R>;
    type Error = Error;

    fn create(&self, svc: &Service) -> Result<SvcWebview<R>> {
        create(self.0, svc)
    }

    fn hide(&self, id: &str, wv: &SvcWebview<R>) -> Result<()> {
        log::info!("[{id}] suspending webview");
        Ok(wv.webview.hide()?)
    }

    fn show(&self, id: &str, wv: &SvcWebview<R>) -> Result<()> {
        log::info!("[{id}] resuming webview");
        Ok(wv.webview.show()?)
    }

    fn close(&self, id: &str, wv: &SvcWebview<R>) -> Result<()> {
        log::info!("[{id}] destroying webview");
        wv.webview.close().map_err(|e| {
            log::error!("[{id}] close failed: {e}");
            e.into()
        })
    }
}

/// Initial load only; afterwards the shell follows SERVICES_CHANGED events.
#[tauri::command]
pub async fn list_services(state: State<'_, Services>) -> Result<Vec<ServiceInfo>> {
    let inner = state.0.lock().await;
    Ok(inner.reg.snapshot(inner.config.services()))
}

#[tauri::command]
pub fn list_recipes() -> Vec<RecipeInfo> {
    services::recipe_infos()
}

#[tauri::command]
pub async fn activate_service(app: AppHandle, id: String) -> Result<()> {
    activate(&app, &id).await
}

async fn activate(app: &AppHandle, id: &str) -> Result<()> {
    let state = app.state::<Services>();
    let mut guard = state.0.lock().await;
    let inner = &mut *guard;
    let svc = inner.config.find(id).ok_or_else(|| services::unknown(id))?;
    if !svc.instance.enabled {
        return Err(Error::ServiceDisabled(svc.instance.name.clone()));
    }
    // A failed transition can still have changed state (e.g. restore failed).
    let result = inner.reg.activate(&Webviews(app), svc);
    if result.is_ok()
        && let Err(e) = inner.config.set_last_active(id)
    {
        log::warn!("[{id}] remembering as last active failed: {e}");
    }
    // The service just sent to the background may be over the limit.
    hibernate(app, inner);
    emit_snapshot(app, inner);
    result
}

/// `act` on `id`'s page where it is, on screen or in the background, without
/// switching to it. A no-op unless the page is loaded (and its process alive):
/// an unloaded service starts at home anyway.
async fn with_page(
    state: State<'_, Services>,
    id: &str,
    act: impl FnOnce(&Webview, &Service) -> Result<()>,
) -> Result<()> {
    let inner = state.0.lock().await;
    let svc = inner.config.find(id).ok_or_else(|| services::unknown(id))?;
    match (inner.reg.state(id), inner.reg.handle(id)) {
        (Lifecycle::Active | Lifecycle::Suspended, Some(wv)) => act(&wv.webview, svc),
        _ => Ok(()),
    }
}

/// Send `id`'s page back to its home URL.
#[tauri::command]
pub async fn go_home(state: State<'_, Services>, id: String) -> Result<()> {
    with_page(state, &id, |webview, svc| {
        let url = svc.url().parse().map_err(|e| Error::invalid(format!("bad url: {e}")))?;
        log::info!("[{id}] going home");
        Ok(webview.navigate(url)?)
    })
    .await
}

#[tauri::command]
pub async fn reload_service(state: State<'_, Services>, id: String) -> Result<()> {
    with_page(state, &id, |webview, _| {
        log::info!("[{id}] reloading");
        Ok(webview.reload()?)
    })
    .await
}

/// How often idle services are looked for.
const HIBERNATE_CHECK: Duration = Duration::from_secs(60);

/// When `svc` is unloaded in the background: never if kept alive, else after
/// its own delay or the app-wide one (0: only to stay within the cap).
fn hibernation(svc: &Service, settings: &Settings) -> Hibernate {
    if svc.keep_alive() {
        return Hibernate::Never;
    }
    match svc.instance.hibernate_after_minutes.unwrap_or(settings.hibernate_after_minutes) {
        0 => Hibernate::OverCap,
        minutes => Hibernate::After(Duration::from_secs(u64::from(minutes) * 60)),
    }
}

/// Unload the background services the settings say to; returns whether any
/// were. The caller holds the lock and emits the snapshot.
fn hibernate(app: &AppHandle, inner: &mut Inner) -> bool {
    let config = &inner.config;
    let cap = config.settings().max_suspended.map(|n| n as usize);
    let ids = inner.reg.hibernation_candidates(Instant::now(), cap, |id| {
        // A webview whose service is gone from the config goes right away.
        config.find(id).map_or(Hibernate::After(Duration::ZERO), |s| hibernation(s, config.settings()))
    });
    let mut changed = false;
    for id in ids {
        log::info!("[{id}] hibernating");
        // A failed close keeps the handle; the next check retries.
        match inner.reg.unload(&Webviews(app), &id) {
            Ok(()) => changed = true,
            Err(e) => log::error!("[{id}] hibernating failed: {e}"),
        }
    }
    changed
}

/// Check for services to hibernate every `HIBERNATE_CHECK`, for the life of the app.
pub fn spawn_hibernation(app: &AppHandle) {
    let app = app.clone();
    async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(HIBERNATE_CHECK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let state = app.state::<Services>();
            let mut inner = state.0.lock().await;
            if hibernate(&app, &mut inner) {
                emit_snapshot(&app, &inner);
            }
        }
    });
}

/// Activate the service that was on screen when the app last ran.
pub fn restore_last_active(app: &AppHandle) {
    let app = app.clone();
    async_runtime::spawn(async move {
        let id = app.state::<Services>().0.lock().await.config.last_active().map(str::to_owned);
        if let Some(id) = id
            && let Err(e) = activate(&app, &id).await
        {
            log::warn!("[{id}] restoring last active service failed: {e}");
        }
    });
}

/// Clicking a service's desktop notification brings the window up on that
/// service. Must be registered before the first notification is shown: the
/// plugin only tracks clicks on notifications shown while a handler exists.
pub fn activate_on_notification_click(app: &AppHandle) -> tauri_plugin_notification::Result<()> {
    let handle = app.clone();
    app.notification().on_action(move |action| {
        if action.action_id() != "tap" {
            return;
        }
        let Some(id) = action
            .notification()
            .and_then(|n| n.extra().get(NOTIFICATION_SERVICE))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            return;
        };
        log::info!("[{id}] notification clicked");
        let app = handle.clone();
        async_runtime::spawn(async move {
            if let Err(e) = activate(&app, &id).await {
                log::error!("[{id}] activating from notification failed: {e}");
            }
            tray::show_window(&app);
        });
    })
}

#[tauri::command]
pub async fn unload_service(
    app: AppHandle,
    state: State<'_, Services>,
    id: String,
) -> Result<()> {
    let mut inner = state.0.lock().await;
    if inner.config.find(&id).is_none() {
        return Err(services::unknown(&id));
    }
    let result = inner.reg.unload(&Webviews(&app), &id);
    emit_snapshot(&app, &inner);
    result
}

/// Add an (unloaded) app with its own fresh profile; returns its id.
#[tauri::command]
pub async fn add_service(app: AppHandle, state: State<'_, Services>, service: NewService) -> Result<String> {
    let root = profiles_root(&app)?;
    let mut inner = state.0.lock().await;
    let id = inner.config.add(&service, |p| root.join(p).exists())?;
    log::info!("[{id}] added ({})", service.url);
    emit_snapshot(&app, &inner);
    Ok(id)
}

/// The title and icon of the site at `url`, for the "Add app" form.
#[tauri::command]
pub async fn inspect_site(url: String) -> Result<SiteInfo> {
    async_runtime::spawn_blocking(move || favicon::inspect(&url))
        .await
        .map_err(|e| Error::Fetch(format!("inspecting the site failed: {e}")))?
}

/// The shell covers the whole window while it shows a modal (`open`), see
/// `platform::set_overlay`.
#[tauri::command]
pub async fn set_overlay(app: AppHandle, open: bool) -> Result<()> {
    let shell = app.get_webview("shell").ok_or(Error::WebviewMissing)?;
    Ok(platform::set_overlay(&shell, open)?)
}

/// Destroy the webview and drop the service from the config. With
/// `delete_profile` its data directory (logins, storage) is deleted too,
/// unless another service still shares that profile.
#[tauri::command]
pub async fn remove_service(
    app: AppHandle,
    state: State<'_, Services>,
    id: String,
    delete_profile: bool,
) -> Result<()> {
    let mut guard = state.0.lock().await;
    let inner = &mut *guard;
    let result = remove(&app, inner, &id, delete_profile);
    emit_snapshot(&app, inner);
    result
}

fn remove(app: &AppHandle, inner: &mut Inner, id: &str, delete_profile: bool) -> Result<()> {
    if inner.config.find(id).is_none() {
        return Err(services::unknown(id));
    }
    inner.reg.unload(&Webviews(app), id)?;
    let removed = inner.config.remove(id)?;
    let profile = &removed.instance.profile;
    log::info!("[{id}] removed");
    if !delete_profile {
        return Ok(());
    }
    if inner.config.services().iter().any(|s| &s.instance.profile == profile) {
        log::info!("[{id}] profile {profile} is shared, keeping it");
        return Ok(());
    }
    let dir = profile_dir(app, profile)?;
    log::info!("[{id}] deleting profile dir {}", dir.display());
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(format!("service removed, but deleting {} failed", dir.display()), e)),
    }
}

#[tauri::command]
pub async fn rename_service(
    app: AppHandle,
    state: State<'_, Services>,
    id: String,
    name: String,
) -> Result<()> {
    let mut inner = state.0.lock().await;
    inner.config.rename(&id, &name)?;
    emit_snapshot(&app, &inner);
    Ok(())
}

/// `ids` must list every service exactly once, in the new order.
#[tauri::command]
pub async fn reorder_services(
    app: AppHandle,
    state: State<'_, Services>,
    ids: Vec<String>,
) -> Result<()> {
    let mut inner = state.0.lock().await;
    inner.config.reorder(&ids)?;
    emit_snapshot(&app, &inner);
    Ok(())
}

#[tauri::command]
pub async fn set_keep_alive(
    app: AppHandle,
    state: State<'_, Services>,
    id: String,
    keep_alive: bool,
) -> Result<()> {
    let mut inner = state.0.lock().await;
    inner.config.set_keep_alive(&id, keep_alive)?;
    emit_snapshot(&app, &inner);
    Ok(())
}

/// Disabling unloads the service; enabling doesn't load it.
#[tauri::command]
pub async fn set_enabled(app: AppHandle, state: State<'_, Services>, id: String, enabled: bool) -> Result<()> {
    let mut guard = state.0.lock().await;
    let inner = &mut *guard;
    let result = set_enabled_in(&app, inner, &id, enabled);
    emit_snapshot(&app, inner);
    result
}

fn set_enabled_in(app: &AppHandle, inner: &mut Inner, id: &str, enabled: bool) -> Result<()> {
    if !enabled {
        inner.reg.unload(&Webviews(app), id)?;
    }
    inner.config.set_enabled(id, enabled)
}

/// Takes effect right away if the service is loaded.
#[tauri::command]
pub async fn set_audio(app: AppHandle, state: State<'_, Services>, id: String, audio: bool) -> Result<()> {
    let mut inner = state.0.lock().await;
    inner.config.set_audio(&id, audio)?;
    emit_snapshot(&app, &inner);
    match inner.reg.handle(&id) {
        Some(wv) => Ok(platform::set_muted(&wv.webview, !audio)?),
        None => Ok(()),
    }
}

/// Takes effect right away if the service is loaded, though a page that
/// already asked for the permission and was denied may need a reload to ask
/// again.
#[tauri::command]
pub async fn set_notifications(
    app: AppHandle,
    state: State<'_, Services>,
    id: String,
    notifications: bool,
) -> Result<()> {
    let mut inner = state.0.lock().await;
    inner.config.set_notifications(&id, notifications)?;
    if let Some(wv) = inner.reg.handle(&id) {
        wv.events.notifications.store(notifications, Ordering::Relaxed);
    }
    emit_snapshot(&app, &inner);
    Ok(())
}

/// Save the "Edit app" form. A loaded service applies sound and notifications
/// in place; if anything else its webview is built from changed, the webview
/// is recreated (or just unloaded, if it was in the background).
#[tauri::command]
pub async fn update_service(
    app: AppHandle,
    state: State<'_, Services>,
    id: String,
    service: NewService,
) -> Result<()> {
    let mut guard = state.0.lock().await;
    let inner = &mut *guard;
    let old = inner.config.find(&id).ok_or_else(|| services::unknown(&id))?.clone();
    inner.config.update(&id, &service)?;
    log::info!("[{id}] updated ({})", service.url);
    let result = refresh(&app, inner, &old);
    emit_snapshot(&app, inner);
    result
}

/// Bring `old`'s webview, if it has one, in line with its updated config.
fn refresh(app: &AppHandle, inner: &mut Inner, old: &Service) -> Result<()> {
    let id = old.id();
    let Some(wv) = inner.reg.handle(id) else { return Ok(()) };
    let new = inner.config.find(id).ok_or_else(|| services::unknown(id))?.clone();
    if !new.instance.enabled {
        return inner.reg.unload(&Webviews(app), id);
    }
    fn rebuilt(s: &Service) -> (&str, &str, &[String], bool, bool) {
        (s.recipe.id, s.url(), s.allowed_domains(), s.instance.media_indicator, s.instance.open_links_inside)
    }
    if rebuilt(old) == rebuilt(&new) {
        wv.events.notifications.store(new.allows(PermissionKind::Notifications), Ordering::Relaxed);
        return Ok(platform::set_muted(&wv.webview, !new.instance.audio)?);
    }
    let was_active = inner.reg.state(id) == Lifecycle::Active;
    log::info!("[{id}] recreating webview for the new settings");
    inner.reg.unload(&Webviews(app), id)?;
    if was_active {
        inner.reg.activate(&Webviews(app), &new)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, Services>) -> Result<Settings> {
    Ok(state.0.lock().await.config.settings().clone())
}

#[tauri::command]
pub async fn set_settings(
    app: AppHandle,
    state: State<'_, Services>,
    settings: Settings,
) -> Result<()> {
    let mut guard = state.0.lock().await;
    let inner = &mut *guard;
    inner.config.set_settings(settings)?;
    let settings = inner.config.settings();
    tray::set_close_to_tray(&app, settings.close_to_tray);
    let bar = platform::Bar::new(settings.bar_position, settings.icon_size);
    if bar != platform::bar() {
        platform::set_bar(bar, app.get_webview("shell").as_ref())?;
    }
    // A shorter limit or a lower cap applies right away.
    if hibernate(&app, inner) {
        emit_snapshot(&app, inner);
    }
    Ok(())
}

#[tauri::command]
pub async fn open_devtools(state: State<'_, Services>) -> Result<()> {
    #[cfg(debug_assertions)]
    if let Some(wv) = state.0.lock().await.reg.active_handle() {
        wv.webview.open_devtools();
    }
    #[cfg(not(debug_assertions))]
    let _ = state;
    Ok(())
}
