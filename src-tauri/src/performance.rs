//! Memory and CPU use, overall and per service profile, for the shell's
//! Performance panel.
//!
//! Each profile is its own WebKit data directory, so its pages run in web
//! processes of their own, served by a network process of its own. On Linux
//! (WebKitGTK) these are all children of uHub, and which profile each belongs
//! to can be read off the system:
//!
//! - a network process has files open in its profile's directory
//!   (`profiles/<profile>/…`: cookies, HSTS, local storage);
//! - a web process is connected over a unix socket to its context's network
//!   process (the socket's peer, from netlink `sock_diag`, as `ss -x` shows).
//!
//! Everything else (uHub itself, the shell's own web and network processes,
//! helpers such as image loaders) counts as uHub's. A web process isn't
//! connected to its network process until it has loaded something, so for a
//! moment after a page starts it counts as uHub's too.
//!
//! Memory is PSS (`smaps_rollup`): shared pages are split between the
//! processes sharing them, so the parts add up to the total, where RSS would
//! count every shared library once per process. GPU memory isn't included.
//! CPU is the time used since the last sample, as a percentage of one core
//! (like `top`).
//!
//! Elsewhere `supported` is false. Windows (WebView2) could use
//! `ICoreWebView2::BrowserProcessId` and the environment's
//! `GetProcessExtendedInfos`, with `GetProcessMemoryInfo`/`GetProcessTimes`;
//! macOS (WKWebView) the private `_webProcessIdentifier` with
//! `proc_pid_rusage`, its network and GPU processes shared by all profiles.

use crate::error::Result;
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf};

/// What a group of processes uses.
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// PSS in bytes.
    pub memory: u64,
    /// Percent of one core since the last sample; `None` without one.
    pub cpu: Option<f64>,
}

impl Usage {
    fn add(&mut self, other: Usage) {
        self.memory += other.memory;
        self.cpu = match (self.cpu, other.cpu) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
    }
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Performance {
    /// false: nothing is measured on this platform.
    pub supported: bool,
    /// Everything below added up.
    pub total: Usage,
    /// uHub itself and whatever isn't a profile's.
    pub uhub: Usage,
    /// By profile; one that isn't loaded is missing.
    pub profiles: BTreeMap<String, Usage>,
    /// The machine's RAM in bytes, for scale.
    pub system_memory: Option<u64>,
}

/// Measure now; the shell polls this while its Performance panel is open.
#[tauri::command]
pub async fn get_performance(app: tauri::AppHandle) -> Result<Performance> {
    let root = crate::webviews::profiles_root(&app)?;
    // Reads a few files per process; off the async runtime's threads.
    Ok(tauri::async_runtime::spawn_blocking(move || measure(root)).await?)
}

#[cfg(not(target_os = "linux"))]
fn measure(_profiles_root: PathBuf) -> Performance {
    Performance::default()
}

#[cfg(target_os = "linux")]
fn measure(profiles_root: PathBuf) -> Performance {
    linux::measure(&profiles_root)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Performance, Usage};
    use std::{
        collections::{HashMap, HashSet},
        fs,
        path::Path,
        sync::Mutex,
        thread,
        time::{Duration, Instant},
    };

    /// The previous sample's CPU time per process (ticks), for `cpu`.
    static LAST: Mutex<Option<(Instant, HashMap<i32, u64>)>> = Mutex::new(None);
    /// A previous sample older than this is too stale to measure from: a
    /// short one is taken first instead (`FIRST_SAMPLE` apart).
    const STALE: Duration = Duration::from_secs(10);
    const FIRST_SAMPLE: Duration = Duration::from_millis(400);

    struct Proc {
        pid: i32,
        ppid: i32,
        comm: String,
        /// utime + stime, in clock ticks.
        ticks: u64,
    }

    pub fn measure(profiles_root: &Path) -> Performance {
        let fresh = matches!(&*LAST.lock().unwrap_or_else(|e| e.into_inner()), Some((t, _)) if t.elapsed() < STALE);
        if !fresh {
            cpu_since_last(&ours());
            thread::sleep(FIRST_SAMPLE);
        }
        let procs = ours();
        let cpu = cpu_since_last(&procs);
        let owners = profiles_of(&procs, profiles_root);

        let mut perf = Performance { supported: true, system_memory: system_memory(), ..Default::default() };
        for p in &procs {
            let usage = Usage { memory: pss(p.pid).unwrap_or(0), cpu: cpu.as_ref().map(|c| c.get(&p.pid).copied().unwrap_or(0.)) };
            match owners.get(&p.pid) {
                Some(profile) => perf.profiles.entry(profile.clone()).or_default().add(usage),
                None => perf.uhub.add(usage),
            }
            perf.total.add(usage);
        }
        perf
    }

    /// uHub and all its descendants.
    fn ours() -> Vec<Proc> {
        let me = std::process::id() as i32;
        let all: Vec<Proc> = fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .filter_map(stat)
            .collect();
        let mut keep = HashSet::from([me]);
        // Parents come before their children in pid order, except after pids wrap: repeat until stable.
        loop {
            let before = keep.len();
            let children: Vec<i32> = all.iter().filter(|p| keep.contains(&p.ppid)).map(|p| p.pid).collect();
            keep.extend(children);
            if keep.len() == before {
                break;
            }
        }
        all.into_iter().filter(|p| keep.contains(&p.pid)).collect()
    }

    fn stat(pid: i32) -> Option<Proc> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // "pid (comm) state ppid …": comm may hold spaces and parentheses.
        let open = stat.find('(')?;
        let close = stat.rfind(')')?;
        let comm = stat[open + 1..close].to_owned();
        let fields: Vec<&str> = stat[close + 2..].split(' ').collect();
        // Fields counted from state (3 in proc(5)): ppid is 4, utime 14, stime 15.
        let field = |n: usize| fields.get(n - 3)?.parse::<u64>().ok();
        Some(Proc { pid, ppid: field(4)? as i32, comm, ticks: field(14)? + field(15)? })
    }

    /// Percent of one core per process since the last call, which this
    /// sample replaces; `None` on the first call or after a stale one.
    fn cpu_since_last(procs: &[Proc]) -> Option<HashMap<i32, f64>> {
        let now = Instant::now();
        let ticks: HashMap<i32, u64> = procs.iter().map(|p| (p.pid, p.ticks)).collect();
        let last = LAST.lock().unwrap_or_else(|e| e.into_inner()).replace((now, ticks.clone()))?;
        let (then, before) = last;
        let elapsed = now.duration_since(then).as_secs_f64();
        if elapsed <= 0. || now.duration_since(then) > STALE {
            return None;
        }
        // SAFETY: sysconf has no preconditions.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
        let hz = if hz > 0. { hz } else { 100. };
        Some(
            ticks
                .iter()
                // A process started since counts from zero.
                .map(|(pid, t)| (*pid, t.saturating_sub(*before.get(pid).unwrap_or(&0)) as f64 / hz / elapsed * 100.))
                .collect(),
        )
    }

    fn pss(pid: i32) -> Option<u64> {
        let rollup = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
        kb(&rollup, "Pss:")
    }

    fn system_memory() -> Option<u64> {
        kb(&fs::read_to_string("/proc/meminfo").ok()?, "MemTotal:")
    }

    /// The "<key> <n> kB" line's value, in bytes.
    fn kb(text: &str, key: &str) -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(key))?;
        Some(line[key.len()..].split_whitespace().next()?.parse::<u64>().ok()? * 1024)
    }

    /// The profile each WebKit process belongs to (see the module docs).
    fn profiles_of(procs: &[Proc], profiles_root: &Path) -> HashMap<i32, String> {
        let mut owners = HashMap::new();
        let mut sockets: HashMap<u64, i32> = HashMap::new();
        for p in procs {
            let is_network = p.comm.starts_with("WebKitNetwork");
            let fds = fs::read_dir(format!("/proc/{}/fd", p.pid)).into_iter().flatten().flatten();
            for target in fds.filter_map(|e| fs::read_link(e.path()).ok()) {
                if let Some(inode) = socket_inode(&target) {
                    sockets.insert(inode, p.pid);
                } else if is_network
                    && !owners.contains_key(&p.pid)
                    && let Some(profile) = target.strip_prefix(profiles_root).ok().and_then(|rest| rest.iter().next())
                {
                    owners.insert(p.pid, profile.to_string_lossy().into_owned());
                }
            }
        }
        let web: HashSet<i32> = procs.iter().filter(|p| p.comm.starts_with("WebKitWeb")).map(|p| p.pid).collect();

        let peers = match sock_diag::unix_peers() {
            Ok(peers) => peers,
            Err(e) => {
                log::warn!("performance: no socket peers, web processes count as uHub's: {e}");
                return owners;
            }
        };
        for (inode, peer) in peers {
            let (Some(&pid), Some(&peer_pid)) = (sockets.get(&inode), sockets.get(&peer)) else { continue };
            if web.contains(&pid)
                && let Some(profile) = owners.get(&peer_pid).cloned()
            {
                owners.insert(pid, profile);
            }
        }
        owners
    }

    /// "socket:[123]" -> 123.
    fn socket_inode(target: &Path) -> Option<u64> {
        target.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
    }

    /// The kernel's unix socket table over netlink (`NETLINK_SOCK_DIAG`,
    /// see sock_diag(7)), which needs no privileges.
    mod sock_diag {
        use std::{io, mem};

        const SOCK_DIAG_BY_FAMILY: u16 = 20;
        const UDIAG_SHOW_PEER: u32 = 0x4;
        const UNIX_DIAG_PEER: u16 = 2;
        /// sizeof(struct nlmsghdr), sizeof(struct unix_diag_msg).
        const NLMSG_HDR: usize = 16;
        const DIAG_MSG: usize = 16;

        /// Each connected unix socket's inode and its peer's.
        pub fn unix_peers() -> io::Result<Vec<(u64, u64)>> {
            // SAFETY: plain socket(2); the fd is owned (and closed) by `Fd`.
            let fd = Fd(unsafe {
                libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_SOCK_DIAG)
            });
            if fd.0 < 0 {
                return Err(io::Error::last_os_error());
            }

            // nlmsghdr, then unix_diag_req: family, protocol, pad, states, ino, show, cookie[2].
            let mut req = Vec::with_capacity(NLMSG_HDR + 24);
            req.extend((NLMSG_HDR as u32 + 24).to_ne_bytes());
            req.extend(SOCK_DIAG_BY_FAMILY.to_ne_bytes());
            req.extend(((libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16).to_ne_bytes());
            req.extend(1u32.to_ne_bytes()); // seq
            req.extend(0u32.to_ne_bytes()); // pid: the kernel's
            req.extend([libc::AF_UNIX as u8, 0, 0, 0]);
            req.extend(u32::MAX.to_ne_bytes()); // every state
            req.extend(0u32.to_ne_bytes());
            req.extend(UDIAG_SHOW_PEER.to_ne_bytes());
            req.extend([0xff; 8]); // no cookie
            // SAFETY: `req` is valid for its length.
            if unsafe { libc::send(fd.0, req.as_ptr().cast(), req.len(), 0) } < 0 {
                return Err(io::Error::last_os_error());
            }

            let mut peers = Vec::new();
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                // SAFETY: `buf` is valid for its length.
                let n = unsafe { libc::recv(fd.0, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n < 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut msgs = &buf[..n as usize];
                while msgs.len() >= NLMSG_HDR {
                    let len = u32_at(msgs, 0) as usize;
                    let kind = u16_at(msgs, 4);
                    if len < NLMSG_HDR || len > msgs.len() {
                        return Err(io::Error::other("malformed netlink message"));
                    }
                    match kind as i32 {
                        libc::NLMSG_DONE => return Ok(peers),
                        libc::NLMSG_ERROR => {
                            let errno = msgs.get(NLMSG_HDR..NLMSG_HDR + 4).map_or(0, |b| u32_at(b, 0) as i32);
                            return Err(io::Error::from_raw_os_error(-errno));
                        }
                        _ if kind == SOCK_DIAG_BY_FAMILY => {
                            if let Some(pair) = peer_of(&msgs[NLMSG_HDR..len]) {
                                peers.push(pair);
                            }
                        }
                        _ => {}
                    }
                    msgs = &msgs[align(len).min(msgs.len())..];
                }
            }
        }

        /// A unix_diag_msg's inode and its UNIX_DIAG_PEER attribute, if any.
        fn peer_of(msg: &[u8]) -> Option<(u64, u64)> {
            if msg.len() < DIAG_MSG {
                return None;
            }
            let inode = u32_at(msg, 4);
            let mut attrs = &msg[DIAG_MSG..];
            while attrs.len() >= 4 {
                let len = u16_at(attrs, 0) as usize;
                if len < 4 || len > attrs.len() {
                    return None;
                }
                if u16_at(attrs, 2) == UNIX_DIAG_PEER && len >= 8 {
                    return Some((inode.into(), u32_at(attrs, 4).into()));
                }
                attrs = &attrs[align(len).min(attrs.len())..];
            }
            None
        }

        fn align(len: usize) -> usize {
            (len + 3) & !3
        }

        fn u16_at(b: &[u8], at: usize) -> u16 {
            u16::from_ne_bytes([b[at], b[at + 1]])
        }

        fn u32_at(b: &[u8], at: usize) -> u32 {
            u32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
        }

        struct Fd(i32);

        impl Drop for Fd {
            fn drop(&mut self) {
                if self.0 >= 0 {
                    // SAFETY: an fd this owns.
                    unsafe { libc::close(self.0) };
                }
            }
        }

        const _: () = assert!(mem::size_of::<libc::nlmsghdr>() == NLMSG_HDR);
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::{linux::fs::MetadataExt, unix::net::UnixStream};

        #[test]
        fn finds_a_socket_pairs_peers() {
            let (a, b) = UnixStream::pair().unwrap();
            let inode = |s: &UnixStream| {
                let fd = std::os::fd::AsRawFd::as_raw_fd(s);
                fs::metadata(format!("/proc/self/fd/{fd}")).unwrap().st_ino()
            };
            let (a, b) = (inode(&a), inode(&b));
            let peers = sock_diag::unix_peers().unwrap();
            assert!(peers.contains(&(a, b)), "{a} -> {b} missing");
            assert!(peers.contains(&(b, a)), "{b} -> {a} missing");
        }

        #[test]
        fn reads_kb_values_as_bytes() {
            assert_eq!(kb("Rss:  2048 kB\nPss:   1024 kB\n", "Pss:"), Some(1024 * 1024));
            assert_eq!(kb("Rss: 1 kB\n", "Pss:"), None);
        }

        #[test]
        fn measures_this_process() {
            let perf = measure(Path::new("/nonexistent"));
            assert!(perf.supported);
            assert!(perf.total.memory > 0 && perf.total.cpu.is_some());
            assert_eq!(perf.total, perf.uhub);
        }
    }
}
