//! LocalSend v2.1 UDP discovery + HTTPS transfer server and session state machine.

use super::protocol::{
    self, InfoDto, MulticastDto, PrepareUploadRequest, PrepareUploadResponse, TlsIdentity,
    MULTICAST_ADDR, PROTOCOL_VERSION,
};
use super::rom::{self, Console, EmulatorIndex, FileRole};
use super::{
    cleanup_staging, free_space_bytes, now_hms, resolve_target, ActiveTransfer, Config, Counts,
    RecentEvent, Status, FREE_SPACE_MARGIN, MAX_RECENT, STAGING_DIR,
};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const PIN_LOCKOUT_THRESHOLD: u32 = 5;
const PIN_LOCKOUT_DURATION: Duration = Duration::from_secs(30);
const MAX_CONCURRENT_CONNS: usize = 6;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Runtime parameters passed from `nibos-drop` CLI or tests.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub port: u16,
    pub state_dir: PathBuf,
    pub status_path: PathBuf,
    pub emulators_dir: PathBuf,
    pub roms_override: Option<PathBuf>,
    pub alias_override: Option<String>,
    pub pin_override: Option<String>,
    pub target_override: Option<super::TargetMode>,
    pub model: String,
    pub enable_multicast: bool,
}

struct SessionFile {
    subdir: Option<String>,
    file_name: String,
    size: u64,
    sha256: Option<String>,
    token: String,
    role: FileRole,
    done: bool,
    staged_path: Option<PathBuf>,
}

struct ActiveSession {
    id: String,
    roms_dir: PathBuf,
    staging_dir: PathBuf,
    files: HashMap<String, SessionFile>,
    last_activity: Instant,
}

impl ActiveSession {
    fn all_done(&self) -> bool {
        self.files.values().all(|f| f.done)
    }
}

struct DropState {
    opts: ServerOptions,
    config: Config,
    info: InfoDto,
    emulators: EmulatorIndex,
    session: Option<ActiveSession>,
    pin_failures: u32,
    pin_locked_until: Option<Instant>,
    counts: Counts,
    recent: Vec<RecentEvent>,
    transfer: Option<ActiveTransfer>,
}

impl DropState {
    fn current_mounts() -> String {
        fs::read_to_string("/proc/mounts").unwrap_or_default()
    }

    fn resolved_target(&self) -> Result<super::ResolvedTarget, String> {
        let mode = self.opts.target_override.unwrap_or(self.config.target);
        resolve_target(
            mode,
            self.opts.roms_override.as_deref(),
            &Self::current_mounts(),
        )
    }

    fn effective_pin(&self) -> &str {
        self.opts
            .pin_override
            .as_deref()
            .unwrap_or(&self.config.pin)
    }

    fn record_event(
        &mut self,
        file: String,
        outcome: &str,
        console: &str,
        path: &str,
        reason: &str,
    ) {
        if outcome == "accepted" {
            self.counts.accepted += 1;
        } else {
            self.counts.rejected += 1;
        }
        self.recent.insert(
            0,
            RecentEvent {
                time: now_hms(),
                file,
                outcome: outcome.to_owned(),
                console: console.to_owned(),
                path: path.to_owned(),
                reason: reason.to_owned(),
            },
        );
        self.recent.truncate(MAX_RECENT);
    }

    fn snapshot_status(&self, active: bool) -> Status {
        let (target, target_label, free_bytes) = match self.resolved_target() {
            Ok(t) => {
                let free = free_space_bytes(&t.roms_dir).unwrap_or(0);
                (t.roms_dir.display().to_string(), t.label, free)
            }
            Err(e) => (String::new(), e, 0),
        };
        Status {
            active,
            alias: self.info.alias.clone(),
            port: self.info.port,
            pin_required: !self.effective_pin().is_empty(),
            target,
            target_label,
            free_bytes,
            transfer: self.transfer.clone(),
            counts: self.counts.clone(),
            recent: self.recent.clone(),
        }
    }

    fn flush_status(&self, active: bool) {
        let status = self.snapshot_status(active);
        if let Err(e) = status.write(&self.opts.status_path) {
            eprintln!(
                "nibos-drop: cannot write {}: {e}",
                self.opts.status_path.display()
            );
        }
    }

    fn reap_stale_session(&mut self) {
        if let Some(s) = &self.session {
            if s.last_activity.elapsed() >= SESSION_IDLE_TIMEOUT {
                let staging = s.staging_dir.clone();
                self.session = None;
                self.transfer = None;
                let _ = fs::remove_dir_all(staging);
                self.flush_status(true);
            }
        }
    }
}

/// Running LocalSend v2.1 server handle.
pub struct DropServer {
    pub info: InfoDto,
    pub bound_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<DropState>>,
    http_thread: Option<thread::JoinHandle<()>>,
    udp_thread: Option<thread::JoinHandle<()>>,
}

impl DropServer {
    pub fn start(opts: ServerOptions) -> io::Result<Self> {
        let config = Config::load(&opts.state_dir);
        let alias = opts
            .alias_override
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| (!config.alias.is_empty()).then(|| config.alias.clone()))
            .unwrap_or_else(|| super::DEFAULT_ALIAS.to_owned());

        let identity = TlsIdentity::load_or_create(&opts.state_dir, &alias)?;
        let tls_cfg = identity.server_config()?;

        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, opts.port)))?;
        listener.set_nonblocking(true)?;
        let bound_addr = listener.local_addr()?;
        let port = bound_addr.port();

        let info = InfoDto {
            alias,
            version: PROTOCOL_VERSION.into(),
            device_model: Some(opts.model.clone()),
            device_type: Some("headless".into()),
            fingerprint: identity.fingerprint,
            port,
            protocol: "https".into(),
            download: false,
        };

        let emulators = EmulatorIndex::load(&opts.emulators_dir);
        let state = Arc::new(Mutex::new(DropState {
            opts: opts.clone(),
            config,
            info: info.clone(),
            emulators,
            session: None,
            pin_failures: 0,
            pin_locked_until: None,
            counts: Counts::default(),
            recent: Vec::new(),
            transfer: None,
        }));

        // Wipe any leftover staging from an unclean shutdown.
        if let Ok(target) = state.lock().unwrap().resolved_target() {
            cleanup_staging(&target.roms_dir);
        }
        state.lock().unwrap().flush_status(true);

        let stop = Arc::new(AtomicBool::new(false));
        let active_conns = Arc::new(AtomicUsize::new(0));

        let http_thread = {
            let stop = Arc::clone(&stop);
            let state = Arc::clone(&state);
            let tls_cfg = Arc::clone(&tls_cfg);
            let active_conns = Arc::clone(&active_conns);
            thread::Builder::new()
                .name("drop-http".into())
                .spawn(move || accept_loop(listener, tls_cfg, state, stop, active_conns))?
        };

        let udp_thread = if opts.enable_multicast {
            let stop = Arc::clone(&stop);
            let info_udp = info.clone();
            Some(
                thread::Builder::new()
                    .name("drop-udp".into())
                    .spawn(move || multicast_loop(port, info_udp, stop))?,
            )
        } else {
            None
        };

        Ok(Self {
            info,
            bound_addr,
            stop,
            state,
            http_thread: Some(http_thread),
            udp_thread,
        })
    }

    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.http_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.udp_thread.take() {
            let _ = h.join();
        }
        let mut st = self.state.lock().unwrap();
        if let Some(s) = st.session.take() {
            let _ = fs::remove_dir_all(s.staging_dir);
        }
        st.transfer = None;
        if let Ok(target) = st.resolved_target() {
            cleanup_staging(&target.roms_dir);
        }
        st.flush_status(false);
    }
}

fn accept_loop(
    listener: TcpListener,
    tls_cfg: Arc<ServerConfig>,
    state: Arc<Mutex<DropState>>,
    stop: Arc<AtomicBool>,
    active_conns: Arc<AtomicUsize>,
) {
    let mut last_reap = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        if last_reap.elapsed() >= Duration::from_secs(5) {
            state.lock().unwrap().reap_stale_session();
            last_reap = Instant::now();
        }
        match listener.accept() {
            Ok((stream, _peer)) => {
                if active_conns.load(Ordering::Relaxed) >= MAX_CONCURRENT_CONNS {
                    drop(stream);
                    continue;
                }
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
                let tls_cfg = Arc::clone(&tls_cfg);
                let state = Arc::clone(&state);
                let counter = Arc::clone(&active_conns);
                counter.fetch_add(1, Ordering::Relaxed);
                let _ = thread::Builder::new()
                    .name("drop-conn".into())
                    .spawn(move || {
                        handle_connection(stream, tls_cfg, state);
                        counter.fetch_sub(1, Ordering::Relaxed);
                    });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_connection(stream: TcpStream, tls_cfg: Arc<ServerConfig>, state: Arc<Mutex<DropState>>) {
    let Ok(conn) = ServerConnection::new(tls_cfg) else {
        return;
    };
    let mut tls = StreamOwned::new(conn, stream);
    let Ok(head) = protocol::read_http_head(&mut tls) else {
        return;
    };

    match (head.method.as_str(), head.path.as_str()) {
        ("GET", "/api/localsend/v2/info") | ("GET", "/api/localsend/v1/info") => {
            let info = state.lock().unwrap().info.clone();
            let _ = protocol::write_json_response(&mut tls, 200, "OK", &info);
        }
        ("POST", "/api/localsend/v2/register") | ("POST", "/api/localsend/v1/register") => {
            let _ = protocol::read_body_bytes(&mut tls, &head, 16 * 1024);
            let info = state.lock().unwrap().info.clone();
            let _ = protocol::write_json_response(&mut tls, 200, "OK", &info);
        }
        ("POST", "/api/localsend/v2/prepare-upload") => {
            handle_prepare_upload(&mut tls, &head, &state);
        }
        ("POST", "/api/localsend/v2/upload") => {
            handle_upload(&mut tls, &head, &state);
        }
        ("POST", "/api/localsend/v2/cancel") => {
            handle_cancel(&mut tls, &head, &state);
        }
        _ => {
            let _ = protocol::write_error_response(&mut tls, 404, "Not Found", "unknown endpoint");
        }
    }
}

fn handle_prepare_upload<S: io::Read + io::Write>(
    stream: &mut S,
    head: &protocol::HttpRequestHead,
    state: &Arc<Mutex<DropState>>,
) {
    let Ok(body) = protocol::read_body_bytes(stream, head, 2 * 1024 * 1024) else {
        let _ = protocol::write_error_response(stream, 400, "Bad Request", "invalid body");
        return;
    };
    let Ok(req) = serde_json::from_slice::<PrepareUploadRequest>(&body) else {
        let _ = protocol::write_error_response(stream, 400, "Bad Request", "invalid json");
        return;
    };

    let mut st = state.lock().unwrap();
    st.reap_stale_session();

    // PIN + rate limiting.
    if let Some(until) = st.pin_locked_until {
        if Instant::now() < until {
            let _ = protocol::write_error_response(
                stream,
                429,
                "Too Many Requests",
                "too many invalid PIN attempts",
            );
            return;
        }
        st.pin_locked_until = None;
        st.pin_failures = 0;
    }
    let expected_pin = st.effective_pin().to_owned();
    if !expected_pin.is_empty() {
        let given = head.query.get("pin").map(String::as_str).unwrap_or("");
        if given != expected_pin {
            st.pin_failures += 1;
            if st.pin_failures >= PIN_LOCKOUT_THRESHOLD {
                st.pin_locked_until = Some(Instant::now() + PIN_LOCKOUT_DURATION);
                let _ = protocol::write_error_response(
                    stream,
                    429,
                    "Too Many Requests",
                    "too many invalid PIN attempts",
                );
            } else {
                let _ = protocol::write_error_response(
                    stream,
                    401,
                    "Unauthorized",
                    "PIN required or invalid",
                );
            }
            return;
        }
        st.pin_failures = 0;
    }

    if st.session.is_some() {
        let _ = protocol::write_error_response(
            stream,
            409,
            "Conflict",
            "another transfer session is active",
        );
        return;
    }

    let target = match st.resolved_target() {
        Ok(t) => t,
        Err(e) => {
            let _ = protocol::write_error_response(stream, 503, "Service Unavailable", &e);
            return;
        }
    };

    // Stage 1: filter non-ROMs before transferring any bytes.
    let candidates: Vec<rom::Candidate<'_>> = req
        .files
        .values()
        .map(|f| rom::Candidate {
            id: &f.id,
            file_name: &f.file_name,
            size: f.size,
        })
        .collect();
    let plan = rom::stage1_plan(&candidates);
    for rej in &plan.rejected {
        eprintln!(
            "nibos-drop: rejected (stage 1) {}: {}",
            rej.file_name, rej.reason
        );
        st.record_event(rej.file_name.clone(), "rejected", "", "", &rej.reason);
    }
    if plan.accepted.is_empty() {
        st.flush_status(true);
        let _ = protocol::write_error_response(
            stream,
            403,
            "Forbidden",
            "only supported ROM files are accepted",
        );
        return;
    }

    // Disk space check.
    let needed = plan.total_bytes().saturating_add(FREE_SPACE_MARGIN);
    if let Some(free) = free_space_bytes(&target.roms_dir) {
        if free < needed {
            let _ = protocol::write_error_response(
                stream,
                507,
                "Insufficient Storage",
                "not enough free space on target card",
            );
            return;
        }
    }

    let session_id = protocol::random_hex(12);
    let staging_dir = target.roms_dir.join(STAGING_DIR).join(&session_id);
    if let Err(e) = fs::create_dir_all(&staging_dir) {
        let _ = protocol::write_error_response(
            stream,
            500,
            "Internal Server Error",
            &format!("cannot create staging dir: {e}"),
        );
        return;
    }

    let mut tokens = BTreeMap::new();
    let mut files = HashMap::new();
    for acc in plan.accepted {
        let token = protocol::random_hex(12);
        let sha256 = req.files.get(&acc.id).and_then(|m| m.sha256.clone());
        tokens.insert(acc.id.clone(), token.clone());
        files.insert(
            acc.id,
            SessionFile {
                subdir: acc.subdir,
                file_name: acc.file_name,
                size: acc.size,
                sha256,
                token,
                role: acc.role,
                done: false,
                staged_path: None,
            },
        );
    }

    st.session = Some(ActiveSession {
        id: session_id.clone(),
        roms_dir: target.roms_dir,
        staging_dir,
        files,
        last_activity: Instant::now(),
    });
    st.flush_status(true);

    let resp = PrepareUploadResponse {
        session_id,
        files: tokens,
    };
    let _ = protocol::write_json_response(stream, 200, "OK", &resp);
}

fn handle_upload<S: io::Read + io::Write>(
    stream: &mut S,
    head: &protocol::HttpRequestHead,
    state: &Arc<Mutex<DropState>>,
) {
    let Some(session_id) = head.query.get("sessionId").cloned() else {
        let _ = protocol::write_error_response(stream, 400, "Bad Request", "missing sessionId");
        return;
    };
    let Some(file_id) = head.query.get("fileId").cloned() else {
        let _ = protocol::write_error_response(stream, 400, "Bad Request", "missing fileId");
        return;
    };
    let Some(token) = head.query.get("token").cloned() else {
        let _ = protocol::write_error_response(stream, 400, "Bad Request", "missing token");
        return;
    };

    // Validate session + token and claim the staging path before releasing the lock during I/O.
    let (staging_path, file_name, expected_size, expected_sha, role, subdir, emulators, roms_dir) = {
        let mut st = state.lock().unwrap();
        let emulators = st.emulators.clone();
        let Some(sess) = st.session.as_mut() else {
            let _ = protocol::write_error_response(stream, 403, "Forbidden", "no active session");
            return;
        };
        if sess.id != session_id {
            let _ = protocol::write_error_response(stream, 403, "Forbidden", "invalid sessionId");
            return;
        }
        let Some(f) = sess.files.get(&file_id) else {
            let _ = protocol::write_error_response(stream, 403, "Forbidden", "unknown fileId");
            return;
        };
        if f.token != token || f.done {
            let _ = protocol::write_error_response(stream, 403, "Forbidden", "invalid token");
            return;
        }
        sess.last_activity = Instant::now();
        let stage = sess
            .staging_dir
            .join(format!("{}.part", protocol::random_hex(8)));
        let info = (
            stage,
            f.file_name.clone(),
            f.size,
            f.sha256.clone(),
            f.role.clone(),
            f.subdir.clone(),
            emulators,
            sess.roms_dir.clone(),
        );
        st.transfer = Some(ActiveTransfer {
            file: info.1.clone(),
            received: 0,
            total: info.2,
        });
        st.flush_status(true);
        info
    };

    let mut staged_file = match File::create(&staging_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = protocol::write_error_response(
                stream,
                500,
                "Internal Server Error",
                &format!("cannot create staged file: {e}"),
            );
            return;
        }
    };

    let mut last_tick = Instant::now();
    // Allow slight slack if client omitted Content-Length or rounded size, capped tightly.
    let max_bytes = expected_size.saturating_add(4096);
    let stream_res =
        protocol::stream_body_to_file(stream, head, &mut staged_file, max_bytes, |received| {
            if last_tick.elapsed() >= PROGRESS_INTERVAL {
                last_tick = Instant::now();
                if let Ok(mut st) = state.lock() {
                    if let Some(sess) = st.session.as_mut() {
                        sess.last_activity = Instant::now();
                    }
                    st.transfer = Some(ActiveTransfer {
                        file: file_name.clone(),
                        received,
                        total: expected_size,
                    });
                    st.flush_status(true);
                }
            }
        });
    drop(staged_file);

    let (_bytes, actual_sha) = match stream_res {
        Ok(r) => r,
        Err(e) => {
            let _ = fs::remove_file(&staging_path);
            let mut st = state.lock().unwrap();
            st.transfer = None;
            st.record_event(
                file_name,
                "rejected",
                "",
                "",
                &format!("upload failed: {e}"),
            );
            mark_done_and_cleanup(&mut st, &session_id, &file_id, None);
            let _ = protocol::write_error_response(
                stream,
                400,
                "Bad Request",
                &format!("upload failed: {e}"),
            );
            return;
        }
    };

    if let Some(want) = expected_sha.filter(|s| !s.is_empty()) {
        if !want.eq_ignore_ascii_case(&actual_sha) {
            let _ = fs::remove_file(&staging_path);
            let mut st = state.lock().unwrap();
            st.transfer = None;
            st.record_event(file_name, "rejected", "", "", "SHA-256 mismatch");
            mark_done_and_cleanup(&mut st, &session_id, &file_id, None);
            let _ = protocol::write_error_response(
                stream,
                422,
                "Unprocessable Entity",
                "checksum mismatch",
            );
            return;
        }
    }

    match role {
        FileRole::Single => match rom::classify_file(&staging_path, &file_name, &emulators) {
            Ok(class) => {
                let dest_dir = if class.preserve_subdir {
                    if let Some(sub) = &subdir {
                        roms_dir.join(class.console.folder()).join(sub)
                    } else {
                        roms_dir.join(class.console.folder())
                    }
                } else {
                    roms_dir.join(class.console.folder())
                };
                let dest = dest_dir.join(&file_name);
                if dest.exists() {
                    let _ = fs::remove_file(&staging_path);
                    let mut st = state.lock().unwrap();
                    st.transfer = None;
                    eprintln!("nibos-drop: collision {} -> {}", file_name, dest.display());
                    st.record_event(file_name, "rejected", "", "", "already exists");
                    mark_done_and_cleanup(&mut st, &session_id, &file_id, None);
                    let _ = protocol::write_error_response(
                        stream,
                        409,
                        "Conflict",
                        "file already exists on device",
                    );
                    return;
                }
                if let Err(e) = place_file(&staging_path, &dest) {
                    let _ = fs::remove_file(&staging_path);
                    let mut st = state.lock().unwrap();
                    st.transfer = None;
                    st.record_event(file_name, "rejected", "", "", &format!("save error: {e}"));
                    mark_done_and_cleanup(&mut st, &session_id, &file_id, None);
                    let _ = protocol::write_error_response(
                        stream,
                        500,
                        "Internal Server Error",
                        &format!("failed to move ROM: {e}"),
                    );
                    return;
                }
                let mut st = state.lock().unwrap();
                st.transfer = None;
                eprintln!("nibos-drop: accepted {} -> {}", file_name, dest.display());
                st.record_event(
                    file_name,
                    "accepted",
                    class.console.folder(),
                    &dest.display().to_string(),
                    "",
                );
                mark_done_and_cleanup(&mut st, &session_id, &file_id, None);
                let _ = protocol::write_http_response(stream, 200, "OK", None, b"");
            }
            Err(reason) => {
                let _ = fs::remove_file(&staging_path);
                let mut st = state.lock().unwrap();
                st.transfer = None;
                eprintln!("nibos-drop: rejected (stage 2) {}: {}", file_name, reason);
                st.record_event(file_name, "rejected", "", "", &reason);
                mark_done_and_cleanup(&mut st, &session_id, &file_id, None);
                let _ = protocol::write_error_response(stream, 403, "Forbidden", &reason);
            }
        },
        FileRole::DiscSetMember => {
            let mut st = state.lock().unwrap();
            st.transfer = None;
            if let Some(sess) = st.session.as_mut().filter(|s| s.id == session_id) {
                if let Some(f) = sess.files.get_mut(&file_id) {
                    f.done = true;
                    f.staged_path = Some(staging_path);
                }
                sess.last_activity = Instant::now();
            }
            // Check if all DiscSetMember files in this `subdir` have now arrived.
            let ready = st.session.as_ref().is_some_and(|sess| {
                sess.id == session_id
                    && sess
                        .files
                        .values()
                        .filter(|f| f.role == FileRole::DiscSetMember && f.subdir == subdir)
                        .all(|f| f.done)
            });
            if !ready {
                st.flush_status(true);
                let _ = protocol::write_http_response(stream, 200, "OK", None, b"");
                return;
            }
            // Commit the whole disc set together.
            let res = commit_disc_set(&mut st, &session_id, &subdir);
            maybe_finish_session(&mut st, &session_id);
            st.flush_status(true);
            match res {
                Ok(console) => {
                    eprintln!(
                        "nibos-drop: accepted disc set ({}) -> {}",
                        file_name,
                        console.folder()
                    );
                    let _ = protocol::write_http_response(stream, 200, "OK", None, b"");
                }
                Err((code, reason_phrase, msg)) => {
                    let _ = protocol::write_error_response(stream, code, reason_phrase, &msg);
                }
            }
        }
    }
}

fn commit_disc_set(
    st: &mut DropState,
    session_id: &str,
    subdir: &Option<String>,
) -> Result<Console, (u16, &'static str, String)> {
    let (roms_dir, entries): (PathBuf, HashMap<String, (String, PathBuf)>) = {
        let Some(sess) = st.session.as_mut().filter(|s| s.id == session_id) else {
            return Err((403, "Forbidden", "session gone".into()));
        };
        let mut map = HashMap::new();
        for f in sess
            .files
            .values_mut()
            .filter(|f| f.role == FileRole::DiscSetMember && &f.subdir == subdir)
        {
            if let Some(p) = f.staged_path.take() {
                map.insert(f.file_name.to_ascii_lowercase(), (f.file_name.clone(), p));
            }
        }
        (sess.roms_dir.clone(), map)
    };

    let cleanup = |entries: &HashMap<String, (String, PathBuf)>| {
        for (_, p) in entries.values() {
            let _ = fs::remove_file(p);
        }
    };

    let console = match rom::classify_disc_set(&entries) {
        Ok(c) => c,
        Err(reason) => {
            cleanup(&entries);
            for (name, _) in entries.values() {
                st.record_event(name.clone(), "rejected", "", "", &reason);
            }
            return Err((403, "Forbidden", reason));
        }
    };

    let dest_dir = roms_dir.join(console.folder());
    for (name, _) in entries.values() {
        if dest_dir.join(name).exists() {
            cleanup(&entries);
            st.record_event(name.clone(), "rejected", "", "", "already exists");
            return Err((409, "Conflict", format!("{name} already exists")));
        }
    }

    for (name, staged) in &entries.values().cloned().collect::<Vec<_>>() {
        let dest = dest_dir.join(name);
        if let Err(e) = place_file(staged, &dest) {
            cleanup(&entries);
            let msg = format!("failed to move disc file: {e}");
            st.record_event(name.clone(), "rejected", "", "", &msg);
            return Err((500, "Internal Server Error", msg));
        }
        st.record_event(
            name.clone(),
            "accepted",
            console.folder(),
            &dest.display().to_string(),
            "",
        );
    }
    Ok(console)
}

fn mark_done_and_cleanup(
    st: &mut DropState,
    session_id: &str,
    file_id: &str,
    staged: Option<PathBuf>,
) {
    if let Some(sess) = st.session.as_mut().filter(|s| s.id == session_id) {
        if let Some(f) = sess.files.get_mut(file_id) {
            f.done = true;
            f.staged_path = staged;
        }
        sess.last_activity = Instant::now();
    }
    maybe_finish_session(st, session_id);
    st.flush_status(true);
}

fn maybe_finish_session(st: &mut DropState, session_id: &str) {
    let finished = st
        .session
        .as_ref()
        .is_some_and(|s| s.id == session_id && s.all_done());
    if finished {
        if let Some(sess) = st.session.take() {
            let _ = fs::remove_dir_all(sess.staging_dir);
        }
    }
}

fn handle_cancel<S: io::Read + io::Write>(
    stream: &mut S,
    head: &protocol::HttpRequestHead,
    state: &Arc<Mutex<DropState>>,
) {
    let sid = head
        .query
        .get("sessionId")
        .map(String::as_str)
        .unwrap_or("");
    let mut st = state.lock().unwrap();
    if st
        .session
        .as_ref()
        .is_some_and(|s| s.id == sid || sid.is_empty())
    {
        if let Some(sess) = st.session.take() {
            let _ = fs::remove_dir_all(sess.staging_dir);
        }
        st.transfer = None;
        st.flush_status(true);
    }
    let _ = protocol::write_http_response(stream, 200, "OK", None, b"");
}

fn place_file(src: &Path, dest: &Path) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            fs::copy(src, dest)?;
            fs::remove_file(src)
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// UDP multicast discovery (`224.0.0.167:53317`)
// ---------------------------------------------------------------------------

fn bind_multicast_socket(port: u16) -> io::Result<UdpSocket> {
    // SAFETY: standard POSIX socket creation with SO_REUSEADDR (+ SO_REUSEPORT) before bind.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let one: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            (&one as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            (&one as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let addr = libc::sockaddr_in {
            #[cfg(target_os = "macos")]
            sin_len: std::mem::size_of::<libc::sockaddr_in>() as u8,
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: port.to_be(),
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        };
        if libc::bind(
            fd,
            (&addr as *const libc::sockaddr_in).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ) < 0
        {
            let err = io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }
        use std::os::unix::io::FromRawFd;
        Ok(UdpSocket::from_raw_fd(fd))
    }
}

fn multicast_loop(port: u16, info: InfoDto, stop: Arc<AtomicBool>) {
    let sock = match bind_multicast_socket(port) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nibos-drop: multicast bind failed on :{port}: {e}");
            return;
        }
    };
    let _ = sock.join_multicast_v4(&MULTICAST_ADDR, &Ipv4Addr::UNSPECIFIED);
    let _ = sock.set_multicast_loop_v4(false);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = sock.as_raw_fd();

    let target = SocketAddrV4::new(MULTICAST_ADDR, port);
    let announce_bytes =
        serde_json::to_vec(&MulticastDto::from_info(&info, true)).unwrap_or_default();
    let reply_bytes =
        serde_json::to_vec(&MulticastDto::from_info(&info, false)).unwrap_or_default();

    // Initial burst to cover Wi-Fi multicast loss on startup.
    let mut next_announce = Instant::now();
    let mut bursts_left = 3u8;

    let mut buf = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        if Instant::now() >= next_announce {
            let _ = sock.send_to(&announce_bytes, target);
            if bursts_left > 1 {
                bursts_left -= 1;
                next_announce = Instant::now() + Duration::from_secs(1);
            } else {
                next_announce = Instant::now() + Duration::from_secs(30);
            }
        }
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                let Ok(peer) = serde_json::from_slice::<MulticastDto>(&buf[..n]) else {
                    continue;
                };
                if peer.fingerprint == info.fingerprint || !peer.is_announce() {
                    continue;
                }
                // Reply over UDP multicast fallback AND direct HTTP(S) `/register`.
                let _ = sock.send_to(&reply_bytes, target);
                let peer_ip = src.ip();
                let peer_port = peer.port;
                let peer_proto = peer.protocol.clone();
                let our = info.clone();
                let _ = thread::Builder::new()
                    .name("drop-reg".into())
                    .spawn(move || {
                        protocol::register_with_peer(peer_ip, peer_port, &peer_proto, &our);
                    });
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme};
    use std::io::{Read, Write};

    #[derive(Debug)]
    struct NoVerify;
    impl ServerCertVerifier for NoVerify {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            rustls_graviola::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    fn https_request(addr: SocketAddr, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
        let provider = Arc::new(rustls_graviola::default_provider());
        let cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify))
            .with_no_client_auth();
        let tcp = TcpStream::connect(addr).unwrap();
        let conn = ClientConnection::new(Arc::new(cfg), ServerName::try_from("localhost").unwrap())
            .unwrap();
        let mut tls = StreamOwned::new(conn, tcp);
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        tls.write_all(req.as_bytes()).unwrap();
        tls.write_all(body).unwrap();
        tls.flush().unwrap();
        let mut raw = Vec::new();
        let _ = tls.read_to_end(&mut raw);
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&raw[..split]);
        let code = head
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u16>()
            .unwrap();
        (code, raw[split + 4..].to_vec())
    }

    #[test]
    fn end_to_end_accepts_rom_and_rejects_non_rom_over_https() {
        let base = std::env::temp_dir().join(format!(
            "nibos-drop-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let roms = base.join("Roms");
        let state_dir = base.join("state");
        let status_path = base.join("status.json");
        fs::create_dir_all(&roms).unwrap();

        let srv = DropServer::start(ServerOptions {
            port: 0,
            state_dir,
            status_path: status_path.clone(),
            emulators_dir: base.join("emulators"),
            roms_override: Some(roms.clone()),
            alias_override: Some("Test DS+".into()),
            pin_override: Some("4242".into()),
            target_override: None,
            model: "RG DS+".into(),
            enable_multicast: false,
        })
        .unwrap();

        // 1. /info returns our alias & headless deviceType
        let (code, body) = https_request(srv.bound_addr, "GET", "/api/localsend/v2/info", b"");
        assert_eq!(code, 200);
        let info: InfoDto = serde_json::from_slice(&body).unwrap();
        assert_eq!(info.alias, "Test DS+");
        assert_eq!(info.device_type.as_deref(), Some("headless"));

        // 2. prepare-upload without PIN -> 401
        let prep_non_rom = br#"{"info":{},"files":{"f1":{"id":"f1","fileName":"photo.jpg","size":100,"fileType":"image/jpeg"}}}"#;
        let (code, _) = https_request(
            srv.bound_addr,
            "POST",
            "/api/localsend/v2/prepare-upload",
            prep_non_rom,
        );
        assert_eq!(code, 401);

        // 3. prepare-upload with PIN but only a non-ROM -> 403
        let (code, _) = https_request(
            srv.bound_addr,
            "POST",
            "/api/localsend/v2/prepare-upload?pin=4242",
            prep_non_rom,
        );
        assert_eq!(code, 403);

        // 4. prepare-upload mixed batch: 1 valid NES, 1 fake .nes, 1 .jpg -> returns 2 tokens
        let mut nes = vec![0u8; 16 + 16384];
        nes[..4].copy_from_slice(b"NES\x1a");
        let fake = b"definitely not a nes rom".to_vec();
        let prep_mixed = format!(
            r#"{{"info":{{}},"files":{{"ok":{{"id":"ok","fileName":"Contra.nes","size":{},"fileType":"application/octet-stream"}},"bad":{{"id":"bad","fileName":"Fake.nes","size":{},"fileType":"application/octet-stream"}},"pic":{{"id":"pic","fileName":"cat.png","size":50,"fileType":"image/png"}}}}}}"#,
            nes.len(),
            fake.len()
        );
        let (code, body) = https_request(
            srv.bound_addr,
            "POST",
            "/api/localsend/v2/prepare-upload?pin=4242",
            prep_mixed.as_bytes(),
        );
        assert_eq!(code, 200);
        let resp: PrepareUploadResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.files.len(), 2);
        assert!(!resp.files.contains_key("pic"));

        // Upload real NES -> 200 and moved to Roms/FC/Contra.nes
        let (code, _) = https_request(
            srv.bound_addr,
            "POST",
            &format!(
                "/api/localsend/v2/upload?sessionId={}&fileId=ok&token={}",
                resp.session_id, resp.files["ok"]
            ),
            &nes,
        );
        assert_eq!(code, 200);
        assert!(roms.join("FC/Contra.nes").is_file());

        // Upload fake .nes -> 403 and NOT placed
        let (code, _) = https_request(
            srv.bound_addr,
            "POST",
            &format!(
                "/api/localsend/v2/upload?sessionId={}&fileId=bad&token={}",
                resp.session_id, resp.files["bad"]
            ),
            &fake,
        );
        assert_eq!(code, 403);
        assert!(!roms.join("FC/Fake.nes").exists());

        // Status JSON reflects accepted=1, rejected=3 (photo.jpg, cat.png, Fake.nes)
        let status = Status::load(&status_path).unwrap();
        assert!(status.active);
        assert_eq!(status.counts.accepted, 1);
        assert_eq!(status.counts.rejected, 3);

        srv.shutdown();
        assert!(!Status::load(&status_path).unwrap().active);
        let _ = fs::remove_dir_all(&base);
    }
}
