//! Service lifecycle state machine, kept free of Tauri so it can be unit tested.
//!
//! `Registry` is the only record of which services have a webview and which one
//! is on screen. A service's `Lifecycle` is derived, never stored: its web
//! process died => CRASHED, it has a handle and is `active` => ACTIVE, has a
//! handle => SUSPENDED, no handle => UNLOADED. With a single `active:
//! Option<id>` there is no way to end up with two ACTIVE services. A crashed
//! service keeps its (dead) handle, and stays `active` if it was on screen,
//! until it is activated (recreated) or unloaded.
//!
//! Unread counts are reported by a live webview, so they are kept here too and
//! dropped with its handle. So is the time a suspended service was last on
//! screen, which decides when it is hibernated (see `select_hibernation`).
//!
//! Each transition calls into a `Backend` (real webviews in webviews.rs, a fake
//! in the tests) and only updates the registry for the steps that succeeded.

use crate::services::{Lifecycle, Service, ServiceInfo};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

/// The side effects a transition needs. Errors are user-facing (`Display`).
pub trait Backend {
    type Handle;
    type Error: std::fmt::Display;
    /// Create and show the service's webview. Must leave nothing behind on error.
    fn create(&self, svc: &Service) -> Result<Self::Handle, Self::Error>;
    fn hide(&self, id: &str, handle: &Self::Handle) -> Result<(), Self::Error>;
    fn show(&self, id: &str, handle: &Self::Handle) -> Result<(), Self::Error>;
    fn close(&self, id: &str, handle: &Self::Handle) -> Result<(), Self::Error>;
}

pub struct Registry<H> {
    handles: HashMap<String, H>,
    active: Option<String>,
    /// Services whose web process died. Always a subset of `handles`' keys.
    crashed: HashSet<String>,
    /// Non-zero unread counts. Always a subset of `handles`' keys.
    unread: HashMap<String, u32>,
    /// When each service was last hidden by a switch. Always a subset of
    /// `handles`' keys; every SUSPENDED service has one.
    hidden_at: HashMap<String, Instant>,
}

impl<H> Default for Registry<H> {
    fn default() -> Self {
        Self {
            handles: HashMap::new(),
            active: None,
            crashed: HashSet::new(),
            unread: HashMap::new(),
            hidden_at: HashMap::new(),
        }
    }
}

/// When background services are unloaded; a `None` turns that rule off.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hibernation {
    /// Unload a service once it has been in the background this long.
    pub idle: Option<Duration>,
    /// Keep at most this many in the background; the oldest go first.
    pub max_suspended: Option<usize>,
}

/// Of `suspended` (id, when it was last on screen), the ones `policy` says to
/// unload at `now`, oldest first: every one idle for at least `policy.idle`,
/// and the oldest of the rest beyond `policy.max_suspended`. Both rules pick
/// from the oldest end, so the result is the oldest `n` for the larger `n`.
pub fn select_hibernation(mut suspended: Vec<(&str, Instant)>, now: Instant, policy: Hibernation) -> Vec<&str> {
    // By id too, so equal timestamps give a stable answer.
    suspended.sort_by_key(|&(id, t)| (t, id));
    let idle = policy.idle.map_or(0, |limit| {
        suspended.iter().take_while(|&&(_, t)| now.saturating_duration_since(t) >= limit).count()
    });
    let over_cap = policy.max_suspended.map_or(0, |cap| suspended.len().saturating_sub(cap));
    suspended.truncate(idle.max(over_cap));
    suspended.into_iter().map(|(id, _)| id).collect()
}

impl<H> Registry<H> {
    pub fn state(&self, id: &str) -> Lifecycle {
        if self.crashed.contains(id) {
            Lifecycle::Crashed
        } else if self.active.as_deref() == Some(id) {
            Lifecycle::Active
        } else if self.handles.contains_key(id) {
            Lifecycle::Suspended
        } else {
            Lifecycle::Unloaded
        }
    }

    pub fn active_handle(&self) -> Option<&H> {
        self.active.as_ref().and_then(|id| self.handles.get(id))
    }

    /// `services` (the configuration) in display order, with their states.
    pub fn snapshot(&self, services: &[Service]) -> Vec<ServiceInfo> {
        services
            .iter()
            .map(|s| ServiceInfo {
                id: s.instance.id.clone(),
                name: s.instance.name.clone(),
                recipe: s.recipe.id,
                url: s.url().to_owned(),
                allowed_domains: s.allowed_domains(),
                profile: s.instance.profile.clone(),
                enabled: s.instance.enabled,
                icon: s.recipe.icon,
                state: self.state(s.id()),
                unread: self.unread.get(s.id()).copied().unwrap_or(0),
                keep_alive: s.keep_alive(),
            })
            .collect()
    }

    /// Record that `id`'s web process died, if `is_current` confirms the
    /// report is about the handle we still hold (not one already replaced).
    /// Returns whether anything changed.
    pub fn mark_crashed(&mut self, id: &str, is_current: impl FnOnce(&H) -> bool) -> bool {
        match self.handles.get(id) {
            Some(handle) if is_current(handle) => self.crashed.insert(id.to_owned()),
            _ => false,
        }
    }

    /// Record `id`'s unread count, if `is_current` confirms the report is
    /// about the handle we still hold. Returns whether the count changed.
    pub fn set_unread(&mut self, id: &str, count: u32, is_current: impl FnOnce(&H) -> bool) -> bool {
        match self.handles.get(id) {
            Some(handle) if is_current(handle) => {}
            _ => return false,
        }
        let old = if count == 0 { self.unread.remove(id) } else { self.unread.insert(id.to_owned(), count) };
        old.unwrap_or(0) != count
    }

    /// The SUSPENDED services `hibernatable` allows that `policy` says to
    /// unload at `now`, oldest first. Crashed ones are left alone: their web
    /// process is already gone.
    pub fn hibernation_candidates(
        &self,
        now: Instant,
        policy: Hibernation,
        hibernatable: impl Fn(&str) -> bool,
    ) -> Vec<String> {
        let suspended = self
            .hidden_at
            .iter()
            .filter(|&(id, _)| self.state(id) == Lifecycle::Suspended && hibernatable(id))
            .map(|(id, &t)| (id.as_str(), t))
            .collect();
        select_hibernation(suspended, now, policy).into_iter().map(String::from).collect()
    }

    /// Make `svc` the one visible service: hide the current one, then show or
    /// create `svc` (a crashed `svc` is destroyed and created afresh). If that
    /// fails, the previous service is put back on screen.
    pub fn activate<B>(&mut self, backend: &B, svc: &Service) -> Result<(), B::Error>
    where
        B: Backend<Handle = H>,
    {
        let id = svc.id();
        let crashed = self.crashed.contains(id);
        if self.active.as_deref() == Some(id) && !crashed {
            return Ok(());
        }

        // A crashed `svc` that is still on screen is replaced, not hidden.
        let was_active = self.active.as_deref() == Some(id);
        let prev = self.active.clone().filter(|p| p != id);
        if let Some(p) = &prev {
            backend.hide(p, &self.handles[p])?;
            self.hidden_at.insert(p.clone(), Instant::now());
        }
        self.active = None;

        let result = match self.handles.get(id) {
            Some(_) if crashed => self.unload(backend, id).and_then(|()| self.insert_new(backend, svc)),
            Some(handle) => backend.show(id, handle),
            None => self.insert_new(backend, svc),
        };

        match result {
            Ok(()) => {
                self.active = Some(id.to_owned());
                Ok(())
            }
            Err(e) => {
                if was_active && self.handles.contains_key(id) {
                    // Closing the crashed webview failed: it is still on screen.
                    self.active = Some(id.to_owned());
                } else if let Some(p) = prev {
                    match backend.show(&p, &self.handles[&p]) {
                        Ok(()) => self.active = Some(p),
                        Err(e) => log::error!("[{p}] restoring after failed switch: {e}"),
                    }
                }
                Err(e)
            }
        }
    }

    fn insert_new<B>(&mut self, backend: &B, svc: &Service) -> Result<(), B::Error>
    where
        B: Backend<Handle = H>,
    {
        let handle = backend.create(svc)?;
        self.handles.insert(svc.id().to_owned(), handle);
        Ok(())
    }

    /// Destroy `id`'s webview. A no-op if it is already unloaded; if closing
    /// fails the handle is kept so the unload can be retried.
    pub fn unload<B>(&mut self, backend: &B, id: &str) -> Result<(), B::Error>
    where
        B: Backend<Handle = H>,
    {
        let Some(handle) = self.handles.get(id) else {
            return Ok(());
        };
        backend.close(id, handle)?;
        self.handles.remove(id);
        self.crashed.remove(id);
        self.unread.remove(id);
        self.hidden_at.remove(id);
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services;
    use std::{
        cell::{Cell, RefCell},
        collections::HashSet,
        sync::LazyLock,
    };

    /// Mimics Tauri: labels must be unique, a created webview starts visible.
    #[derive(Default)]
    struct Fake {
        live: RefCell<HashSet<String>>,
        visible: RefCell<HashSet<String>>,
        creates: Cell<usize>,
        fail: RefCell<HashSet<(&'static str, String)>>,
    }

    impl Fake {
        fn fail_on(&self, op: &'static str, id: &str) {
            self.fail.borrow_mut().insert((op, id.to_owned()));
        }

        fn check(&self, op: &'static str, id: &str) -> Result<(), String> {
            if self.fail.borrow().contains(&(op, id.to_owned())) {
                Err(format!("{op} {id} failed"))
            } else {
                Ok(())
            }
        }
    }

    impl Backend for Fake {
        type Handle = String;
        type Error = String;

        fn create(&self, svc: &Service) -> Result<String, String> {
            let id = svc.id();
            self.check("create", id)?;
            if !self.live.borrow_mut().insert(id.to_owned()) {
                return Err(format!("duplicate label svc-{id}"));
            }
            self.visible.borrow_mut().insert(id.to_owned());
            self.creates.set(self.creates.get() + 1);
            Ok(id.to_owned())
        }

        fn hide(&self, id: &str, h: &String) -> Result<(), String> {
            assert_eq!(id, h);
            self.check("hide", id)?;
            self.visible.borrow_mut().remove(id);
            Ok(())
        }

        fn show(&self, id: &str, h: &String) -> Result<(), String> {
            assert_eq!(id, h);
            self.check("show", id)?;
            self.visible.borrow_mut().insert(id.to_owned());
            Ok(())
        }

        fn close(&self, id: &str, h: &String) -> Result<(), String> {
            assert_eq!(id, h);
            self.check("close", id)?;
            self.live.borrow_mut().remove(id);
            self.visible.borrow_mut().remove(id);
            Ok(())
        }
    }

    static SVCS: LazyLock<Vec<Service>> = LazyLock::new(services::seed);

    fn svc(i: usize) -> &'static Service {
        &SVCS[i]
    }

    fn active_ids(reg: &Registry<String>) -> Vec<String> {
        reg.snapshot(&SVCS).into_iter().filter(|s| s.state == Lifecycle::Active).map(|s| s.id).collect()
    }

    /// The registry and the (fake) world agree, and at most one service is
    /// active. A crashed service may still be the one on screen.
    fn assert_consistent(reg: &Registry<String>, fake: &Fake) {
        let active = active_ids(reg);
        assert!(active.len() <= 1, "several active: {active:?}");
        if let Some(a) = active.first() {
            assert_eq!(Some(a), reg.active.as_ref());
        }
        let visible: Vec<_> = fake.visible.borrow().iter().cloned().collect();
        assert_eq!(visible, reg.active.iter().cloned().collect::<Vec<_>>(), "visible webviews != active service");
        assert!(reg.crashed.iter().all(|id| reg.handles.contains_key(id)), "crashed without a handle");
        assert!(reg.unread.keys().all(|id| reg.handles.contains_key(id)), "unread without a handle");
        assert!(reg.hidden_at.keys().all(|id| reg.handles.contains_key(id)), "hidden_at without a handle");
        for s in reg.snapshot(&SVCS) {
            if s.state == Lifecycle::Suspended {
                assert!(reg.hidden_at.contains_key(&s.id), "{} suspended without hidden_at", s.id);
            }
        }
        let mut loaded: Vec<_> = reg.handles.keys().cloned().collect();
        let mut live: Vec<_> = fake.live.borrow().iter().cloned().collect();
        loaded.sort();
        live.sort();
        assert_eq!(loaded, live, "registry handles != live webviews");
    }

    #[test]
    fn activate_creates_once() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        reg.activate(&fake, svc(0)).unwrap();
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Active);
        assert_eq!(fake.creates.get(), 1);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn switching_suspends_previous_and_resumes_without_reload() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        reg.activate(&fake, svc(1)).unwrap();
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Suspended);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Active);
        assert_consistent(&reg, &fake);

        reg.activate(&fake, svc(0)).unwrap();
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Active);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Suspended);
        assert_eq!(fake.creates.get(), 2);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn unload_active_and_suspended() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        reg.activate(&fake, svc(1)).unwrap();

        reg.unload(&fake, svc(0).id()).unwrap();
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Unloaded);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Active);
        assert_consistent(&reg, &fake);

        reg.unload(&fake, svc(1).id()).unwrap();
        assert!(reg.active_handle().is_none());
        assert_consistent(&reg, &fake);

        // Unloading twice is a no-op, and re-activating creates afresh.
        reg.unload(&fake, svc(1).id()).unwrap();
        reg.activate(&fake, svc(1)).unwrap();
        assert_eq!(fake.creates.get(), 3);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn failed_create_restores_previous() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        fake.fail_on("create", svc(1).id());
        assert!(reg.activate(&fake, svc(1)).is_err());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Active);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Unloaded);
        assert_consistent(&reg, &fake);

        // Not left half-created: a retry succeeds.
        fake.fail.borrow_mut().clear();
        reg.activate(&fake, svc(1)).unwrap();
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn failed_show_keeps_target_suspended() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        reg.activate(&fake, svc(1)).unwrap();
        fake.fail_on("show", svc(0).id());
        assert!(reg.activate(&fake, svc(0)).is_err());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Suspended);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Active);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn failed_hide_changes_nothing() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        fake.fail_on("hide", svc(0).id());
        assert!(reg.activate(&fake, svc(1)).is_err());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Active);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Unloaded);
        assert_eq!(fake.creates.get(), 1);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn failed_restore_leaves_nothing_active() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        fake.fail_on("create", svc(1).id());
        fake.fail_on("show", svc(0).id());
        assert!(reg.activate(&fake, svc(1)).is_err());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Suspended);
        assert!(reg.active_handle().is_none());
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn failed_close_keeps_handle_for_retry() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        fake.fail_on("close", svc(0).id());
        assert!(reg.unload(&fake, svc(0).id()).is_err());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Active);
        assert_consistent(&reg, &fake);

        fake.fail.borrow_mut().clear();
        reg.unload(&fake, svc(0).id()).unwrap();
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Unloaded);
        assert_consistent(&reg, &fake);
    }

    fn crash(reg: &mut Registry<String>, id: &str) -> bool {
        reg.mark_crashed(id, |_| true)
    }

    #[test]
    fn crashed_active_is_recreated_on_activate() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        assert!(crash(&mut reg, svc(0).id()));
        assert!(!crash(&mut reg, svc(0).id()), "second report changes nothing");
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Crashed);
        assert_consistent(&reg, &fake);

        reg.activate(&fake, svc(0)).unwrap();
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Active);
        assert_eq!(fake.creates.get(), 2);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn crashed_background_is_recreated_on_activate() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        reg.activate(&fake, svc(1)).unwrap();
        crash(&mut reg, svc(0).id());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Crashed);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Active);

        reg.activate(&fake, svc(0)).unwrap();
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Active);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Suspended);
        assert_eq!(fake.creates.get(), 3);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn stale_or_unloaded_crash_reports_are_ignored() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        assert!(!crash(&mut reg, svc(0).id()));
        reg.activate(&fake, svc(0)).unwrap();
        assert!(!reg.mark_crashed(svc(0).id(), |_| false));
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Active);
    }

    #[test]
    fn unload_clears_crash() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        crash(&mut reg, svc(0).id());
        reg.unload(&fake, svc(0).id()).unwrap();
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Unloaded);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn failed_recreate_leaves_service_unloaded() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(1)).unwrap();
        reg.activate(&fake, svc(0)).unwrap();
        crash(&mut reg, svc(0).id());
        fake.fail_on("create", svc(0).id());
        assert!(reg.activate(&fake, svc(0)).is_err());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Unloaded);
        assert!(reg.active_handle().is_none());
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn failed_close_of_crashed_active_keeps_it_on_screen() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        crash(&mut reg, svc(0).id());
        fake.fail_on("close", svc(0).id());
        assert!(reg.activate(&fake, svc(0)).is_err());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Crashed);
        assert_eq!(reg.active.as_deref(), Some(svc(0).id()));
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn failed_close_of_crashed_restores_previous() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        reg.activate(&fake, svc(1)).unwrap();
        crash(&mut reg, svc(0).id());
        fake.fail_on("close", svc(0).id());
        assert!(reg.activate(&fake, svc(0)).is_err());
        assert_eq!(reg.state(svc(0).id()), Lifecycle::Crashed);
        assert_eq!(reg.state(svc(1).id()), Lifecycle::Active);
        assert_consistent(&reg, &fake);
    }

    fn unread(reg: &Registry<String>, id: &str) -> u32 {
        reg.snapshot(&SVCS).into_iter().find(|s| s.id == id).unwrap().unread
    }

    #[test]
    fn unread_reports_change_only_on_new_counts() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        let id = svc(0).id();
        assert!(!reg.set_unread(id, 3, |_| true), "unloaded: ignored");
        reg.activate(&fake, svc(0)).unwrap();
        assert!(!reg.set_unread(id, 0, |_| true), "0 -> 0 is no change");
        assert!(reg.set_unread(id, 3, |_| true));
        assert!(!reg.set_unread(id, 3, |_| true));
        assert!(!reg.set_unread(id, 7, |_| false), "stale webview: ignored");
        assert_eq!(unread(&reg, id), 3);
        assert!(reg.set_unread(id, 0, |_| true));
        assert_eq!(unread(&reg, id), 0);
        assert_consistent(&reg, &fake);
    }

    #[test]
    fn unread_survives_suspend_and_crash_but_not_unload() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        reg.activate(&fake, svc(0)).unwrap();
        reg.set_unread(svc(0).id(), 4, |_| true);
        reg.activate(&fake, svc(1)).unwrap();
        crash(&mut reg, svc(0).id());
        assert_eq!(unread(&reg, svc(0).id()), 4);
        reg.unload(&fake, svc(0).id()).unwrap();
        assert_eq!(unread(&reg, svc(0).id()), 0);
        assert_consistent(&reg, &fake);
    }

    const MIN: Duration = Duration::from_secs(60);

    fn policy(idle_min: Option<u32>, max_suspended: Option<usize>) -> Hibernation {
        Hibernation { idle: idle_min.map(|m| m * MIN), max_suspended }
    }

    #[test]
    fn hibernation_by_idle_time() {
        let t0 = Instant::now();
        let at = |min: u32| t0 + min * MIN;
        let suspended = || vec![("c", at(10)), ("a", at(0)), ("b", at(5))];
        let pick = |now, p| select_hibernation(suspended(), now, p);

        assert_eq!(pick(at(14), policy(Some(15), None)), Vec::<&str>::new());
        assert_eq!(pick(at(15), policy(Some(15), None)), ["a"], "the limit itself counts as idle");
        assert_eq!(pick(at(24), policy(Some(15), None)), ["a", "b"]);
        assert_eq!(pick(at(60), policy(Some(15), None)), ["a", "b", "c"]);
        assert_eq!(pick(at(600), policy(None, None)), Vec::<&str>::new(), "no rule: keep everything");
        // A timestamp after `now` (shouldn't happen) is not idle, not a panic.
        assert_eq!(pick(at(4), policy(Some(1), None)), ["a"]);
    }

    #[test]
    fn hibernation_by_cap() {
        let t0 = Instant::now();
        let at = |min: u32| t0 + min * MIN;
        let suspended = || vec![("c", at(10)), ("a", at(0)), ("b", at(5))];
        let pick = |p| select_hibernation(suspended(), at(11), p);

        assert_eq!(pick(policy(None, Some(3))), Vec::<&str>::new());
        assert_eq!(pick(policy(None, Some(2))), ["a"], "oldest first");
        assert_eq!(pick(policy(None, Some(0))), ["a", "b", "c"]);
        // Both rules: whichever picks more.
        assert_eq!(pick(policy(Some(6), Some(2))), ["a", "b"]);
        assert_eq!(pick(policy(Some(10), Some(1))), ["a", "b"]);
        assert_eq!(select_hibernation(vec![], at(11), policy(Some(0), Some(0))), Vec::<&str>::new());
        // Ties are broken by id, not by input order.
        assert_eq!(select_hibernation(vec![("y", t0), ("x", t0)], t0, policy(None, Some(1))), ["x"]);
    }

    #[test]
    fn hibernation_candidates_are_suspended_and_hibernatable() {
        let (mut reg, fake) = (Registry::default(), Fake::default());
        for i in 0..4 {
            reg.activate(&fake, svc(i)).unwrap();
        }
        // 0, 1, 2 suspended (in that order), 3 active; 1 crashes, 2 is kept alive.
        crash(&mut reg, svc(1).id());
        let later = Instant::now() + 60 * MIN;
        let pick = |reg: &Registry<String>, p| reg.hibernation_candidates(later, p, |id| id != svc(2).id());
        assert_eq!(pick(&reg, policy(Some(15), None)), [svc(0).id()]);
        assert_eq!(pick(&reg, policy(None, Some(0))), [svc(0).id()]);
        assert_eq!(pick(&reg, policy(None, None)), Vec::<String>::new());

        // Coming back to the front resets the clock.
        reg.activate(&fake, svc(0)).unwrap();
        assert_eq!(pick(&reg, policy(Some(15), None)), [svc(3).id()]);
        assert!(reg.hibernation_candidates(Instant::now(), policy(Some(15), None), |_| true).is_empty());

        reg.unload(&fake, svc(3).id()).unwrap();
        assert_eq!(pick(&reg, policy(Some(15), None)), Vec::<String>::new());
        assert_consistent(&reg, &fake);
    }

    /// Long pseudo-random run of activations, unloads, crashes and injected failures.
    #[test]
    fn invariants_hold_under_random_operations() {
        const OPS: [&str; 4] = ["create", "hide", "show", "close"];
        let (mut reg, fake) = (Registry::default(), Fake::default());
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let n = SVCS.len();
        for _ in 0..5000 {
            fake.fail.borrow_mut().clear();
            if next(4) == 0 {
                fake.fail_on(OPS[next(OPS.len())], svc(next(n)).id());
            }
            let s = svc(next(n));
            let _ = match next(7) {
                0 | 1 => reg.unload(&fake, s.id()),
                2 => {
                    crash(&mut reg, s.id());
                    Ok(())
                }
                3 => {
                    reg.set_unread(s.id(), next(3) as u32, |_| true);
                    Ok(())
                }
                _ => reg.activate(&fake, s),
            };
            assert_consistent(&reg, &fake);
        }
    }
}
