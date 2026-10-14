use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ratatui::style::Color;

use crate::types::{SparklineStyle, TableId, Tab};
use crate::ui::color::Theme;

/// The range `tick_ms` is kept in, by `-`/`+` and when loaded.
pub const TICK_MS_MIN: u64 = 500;
pub const TICK_MS_MAX: u64 = 30_000;

/// Where `salt-radar.toml` lives: a `salt-radar.toml` in the current
/// directory wins if present (lets you keep a per-directory config, and
/// preserves behavior for anyone already relying on the old cwd-only
/// lookup); otherwise it resolves to the XDG config dir
/// (`$XDG_CONFIG_HOME/salt-radar/salt-radar.toml`, falling back to
/// `~/.config/salt-radar/salt-radar.toml`) instead of silently writing
/// into whatever directory the binary happened to be launched from.
pub fn resolve_config_path() -> PathBuf {
    let cwd_path = PathBuf::from("salt-radar.toml");
    if cwd_path.is_file() {
        return cwd_path;
    }

    match user_config_dir() {
        Some(dir) => dir.join("salt-radar.toml"),
        None => cwd_path,
    }
}

/// The user who started salt-radar through `sudo`, when it runs as root that
/// way: their config, themes and files are used instead of root's, which
/// every sudo user would otherwise share (sudo resets `HOME` to root's and
/// drops `XDG_*`). Only `SUDO_USER` is followed; no sudo, `sudo -i` or a
/// real root login leave this `None`.
pub struct SudoUser {
    pub home: PathBuf,
    pub uid: u32,
    pub gid: u32,
}

pub fn sudo_user() -> Option<&'static SudoUser> {
    static USER: OnceLock<Option<SudoUser>> = OnceLock::new();
    USER.get_or_init(|| {
        // SAFETY: geteuid can't fail; getpwnam runs once, here, before any
        // other thread could call it, and its result is copied out at once.
        if unsafe { libc::geteuid() } != 0 {
            return None;
        }
        let name = std::env::var("SUDO_USER").ok().filter(|n| !n.is_empty() && n != "root")?;
        let name = std::ffi::CString::new(name).ok()?;
        let pw = unsafe { libc::getpwnam(name.as_ptr()) };
        if pw.is_null() {
            return None;
        }
        let (dir, uid, gid) = unsafe { ((*pw).pw_dir, (*pw).pw_uid, (*pw).pw_gid) };
        if dir.is_null() {
            return None;
        }
        let home = PathBuf::from(unsafe { std::ffi::CStr::from_ptr(dir) }.to_string_lossy().into_owned());
        (home.is_absolute() && home != Path::new("/")).then_some(SudoUser { home, uid, gid })
    }).as_ref()
}

/// The invoking user's home: the sudo user's, or `$HOME`.
fn home_dir() -> Option<PathBuf> {
    match sudo_user() {
        Some(u) => Some(u.home.clone()),
        None => std::env::var_os("HOME").map(PathBuf::from),
    }
}

/// `$XDG_CONFIG_HOME/salt-radar`, falling back to `~/.config/salt-radar` —
/// the sudo user's under sudo (their environment's `XDG_*` is gone).
fn user_config_dir() -> Option<PathBuf> {
    match sudo_user() {
        Some(u) => Some(u.home.join(".config/salt-radar")),
        None => own_config_dir(),
    }
}

/// The config dir of the account salt-radar runs as (root's under sudo).
fn own_config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|dir| dir.join("salt-radar"))
}

/// `$XDG_STATE_HOME/salt-radar/last-version`, falling back to
/// `~/.local/state/salt-radar/last-version` (the sudo user's under sudo).
fn last_version_path() -> Option<PathBuf> {
    let state = match sudo_user() {
        Some(u) => Some(u.home.join(".local/state")),
        None => std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state"))),
    };
    state.map(|dir| dir.join("salt-radar").join("last-version"))
}

/// Hands a file or directory salt-radar created over to the user who ran it
/// through sudo, when it's in their home — so their home doesn't fill up
/// with root-owned files they can't edit.
pub fn give_to_sudo_user(path: &Path) {
    let Some(u) = sudo_user() else { return };
    if !path.starts_with(&u.home) {
        return;
    }
    if let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
        // SAFETY: a valid NUL-terminated path; a failure just leaves root as owner.
        unsafe { libc::chown(c.as_ptr(), u.uid, u.gid) };
    }
}

/// `create_dir_all`, giving the directories it creates to the sudo user
/// (see `give_to_sudo_user`).
pub fn create_dir_all_owned(dir: &Path) -> std::io::Result<()> {
    let missing: Vec<PathBuf> = dir.ancestors().take_while(|a| !a.as_os_str().is_empty() && !a.exists()).map(Path::to_path_buf).collect();
    std::fs::create_dir_all(dir)?;
    for d in missing.iter().rev() {
        give_to_sudo_user(d);
    }
    Ok(())
}

/// Records the running version as the last one seen and returns the version
/// seen before it, if that was a different one. A first run returns `None`:
/// nothing was upgraded.
pub fn note_version_seen() -> Option<String> {
    let path = last_version_path()?;
    let current = env!("CARGO_PKG_VERSION");
    let previous = std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string());
    if previous.as_deref() == Some(current) {
        return None;
    }
    if let Some(dir) = path.parent() {
        let _ = create_dir_all_owned(dir);
    }
    if std::fs::write(&path, format!("{current}\n")).is_ok() {
        give_to_sudo_user(&path);
    }
    previous.filter(|p| !p.is_empty())
}

/// Expands a leading `~` / `~/` against the invoking user's home, as the
/// shell would (the sudo user's under sudo).
pub fn expand_home(raw: &str) -> PathBuf {
    let home = home_dir;
    if raw == "~" {
        return home().unwrap_or_else(|| PathBuf::from("/root"));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(h) = home() {
            return h.join(rest);
        }
    }
    PathBuf::from(raw)
}

pub const DEFAULT_THEME: &str = "Default";

/// Where `*.theme` files are looked up, lowest priority first: the
/// packaged themes, then the user's own (which win on a name clash) — under
/// sudo, root's and then the sudo user's.
fn theme_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/share/salt-radar/themes")];
    for dir in [own_config_dir(), user_config_dir()].into_iter().flatten() {
        let themes = dir.join("themes");
        if !dirs.contains(&themes) {
            dirs.push(themes);
        }
    }
    dirs
}

/// Names selectable as `color_theme`: the built-in `Default`, then every
/// `*.theme` file found in `theme_dirs`, by file stem.
pub fn theme_names() -> Vec<String> {
    let mut names: Vec<String> = theme_dirs().iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "theme" { return None; }
            Some(path.file_stem()?.to_string_lossy().into_owned())
        })
        .filter(|name| name != DEFAULT_THEME)
        .collect();
    names.sort();
    names.dedup();
    names.insert(0, DEFAULT_THEME.to_string());
    names
}

/// Resolves `color_theme` — a theme name from `theme_names`, or a path to a
/// `.theme` file — onto the built-in default; keys the file doesn't set (or
/// sets to something unparseable) keep their default color, and an unknown
/// theme is just the default.
pub fn load_theme(name: &str) -> Theme {
    let mut theme = Theme::DEFAULT;
    if name == DEFAULT_THEME { return theme; }
    let path = if name.contains('/') || name.ends_with(".theme") {
        Some(PathBuf::from(name))
    } else {
        theme_dirs().into_iter().rev()
            .map(|dir| dir.join(format!("{}.theme", name)))
            .find(|p| p.is_file())
    };
    let Some(data) = path.and_then(|p| std::fs::read_to_string(p).ok()) else { return theme };
    for line in data.lines() {
        let Some((key, value)) = parse_theme_line(line) else { continue };
        if let (Some(slot), Some(color)) = (theme.slot(key), parse_theme_color(value)) {
            *slot = color;
        }
    }
    theme
}

/// `theme[key]="value"` (btop's format); anything else is skipped.
fn parse_theme_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("theme[")?;
    let (key, rest) = rest.split_once(']')?;
    let value = rest.trim_start().strip_prefix('=')?.trim();
    Some((key.trim(), value.trim_matches('"')))
}

/// `#RRGGBB`, `#RR` (a gray level), or decimal `R G B`; empty means the
/// terminal's own default color.
fn parse_theme_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if value.is_empty() { return Some(Color::Reset); }
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.is_ascii() { return None; }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            2 => byte(0).map(|v| Color::Rgb(v, v, v)),
            6 => Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?)),
            _ => None,
        };
    }
    let rgb: Vec<u8> = value.split_whitespace().map(|v| v.parse().ok()).collect::<Option<_>>()?;
    match rgb[..] {
        [r, g, b] => Some(Color::Rgb(r, g, b)),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub master_config: String,
    pub python_bin: String,
    pub agent_path: String,
    pub dashboard_secs: u64,
    pub minions_secs: u64,
    pub config_secs: u64,
    /// How often (ms) the screen redraws and timed work runs; `-`/`+`
    /// change it by 500 within 500-30000.
    pub tick_ms: u64,
    /// Cap on what redraws send to the terminal, KB/s (0 = none): over it,
    /// redraws that aren't answering a key wait, so a slow link (a
    /// session-recording SSH proxy) doesn't fall behind a busy master.
    pub max_output_kbs: u64,
    pub event_buffer_size: usize,
    pub sniffer_buffer_size: usize,
    pub stale_minion_secs: u64,
    pub pending_key_secs: u64,
    pub job_straggler_secs: u64,
    pub disk_usage_pct: f64,
    pub auth_storm_rate_per_sec: f64,
    pub worker_saturation_pct: f64,
    pub pending_key_churn_per_min: u64,
    pub service_flap_threshold_24h: u64,
    pub service_mem_growth_mb_per_hour: f64,
    pub service_mem_abs_threshold_mb: f64,
    pub service_mem_pct_threshold: f64,
    /// p95 POST /login processing time (seconds) that triggers the "API auth
    /// latency" alert. Only meaningful once the salt-api panel has real
    /// data — see App::api_login_latency_p50_p95.
    pub api_auth_latency_warn_secs: f64,
    /// Rolling window (in observed requests) backing the salt-api panel's
    /// error-rate and POST /login p50/p95 latency figures — see
    /// App::api_login_latency_p50_p95.
    pub api_stats_window: usize,
    /// Max entries kept in the API tab's live request log before the oldest
    /// are evicted, independent of the age-based prune below.
    pub api_log_cap: usize,
    /// Entries older than this (seconds) are dropped from the API tab's
    /// request log every poll regardless of how far under api_log_cap it is
    /// — a quiet API shouldn't let the log hold onto hours-old requests.
    pub api_log_max_age_secs: u64,
    /// Whether salt-radar's own `GET /stats` polling (the request that
    /// populates this very log) is filtered out of the API tab's request
    /// log. Defaults on since polling at the Dashboard refresh interval
    /// would otherwise drown out real traffic with one entry per tick; set
    /// to false to see it anyway.
    pub api_hide_stats_polls: bool,
    /// How the agent authenticates to salt-api's `/stats` endpoint:
    /// "none" (default) relies on `stats_disable_auth: True` in the master
    /// config, so `/stats` needs no credentials at all — see the README's
    /// security note on that tradeoff. "plaintext" logs in with
    /// `api_stats_auth_username`/`api_stats_auth_password` read straight
    /// from this config file (marked insecure — stored and transmitted to
    /// the agent as plain text). "pillar" resolves those same two values,
    /// once and then cached, from `api_stats_auth_username_pillar_path`/
    /// `api_stats_auth_password_pillar_path` — like two independent
    /// `pillar.get` calls, entirely master-side for this master's own local
    /// minion (no live round-trip, and the credential never passes through
    /// this config file or the Rust process at all) — see the doc comment
    /// on the path fields below.
    pub api_stats_auth_mode: String,
    /// eauth backend the login uses (pam, ldap, file, ...) — see Salt's
    /// `external_auth` docs. Only meaningful when api_stats_auth_mode is
    /// "plaintext" or "pillar".
    pub api_stats_auth_eauth: String,
    /// Plaintext credentials for api_stats_auth_mode = "plaintext". See the
    /// mode's own doc comment above for the security tradeoff.
    pub api_stats_auth_username: String,
    pub api_stats_auth_password: String,
    /// Colon-delimited pillar paths resolving directly to the username/
    /// password strings, for api_stats_auth_mode = "pillar" — e.g.
    /// "salt_radar:api_user" for a pillar shaped `salt_radar: {api_user:
    /// ...}`. Compiled for this master's own local minion ID (the same one
    /// `salt-call pillar.get` would use on this box) — not something you
    /// configure here, and requires a minion co-located with the master. A
    /// `top.sls` entry keyed to that minion's own ID (not `'*'`, which
    /// would hand the credential to every minion) is all that's needed.
    pub api_stats_auth_username_pillar_path: String,
    pub api_stats_auth_password_pillar_path: String,
    /// Per-unit overrides for the standard bundled Salt services — each 0.0
    /// means "no override, fall back to the global service_mem_*_threshold*
    /// above" (the global fields already use 0.0 to mean "check disabled",
    /// so these reuse that same sentinel to mean "not set").
    pub service_mem_abs_threshold_master_mb: f64,
    pub service_mem_pct_threshold_master: f64,
    pub service_mem_abs_threshold_minion_mb: f64,
    pub service_mem_pct_threshold_minion: f64,
    pub service_mem_abs_threshold_api_mb: f64,
    pub service_mem_pct_threshold_api: f64,
    pub service_mem_abs_threshold_syndic_mb: f64,
    pub service_mem_pct_threshold_syndic: f64,
    /// Which mechanism computes the Dashboard ZMQ panel's latency figure:
    /// "agent_ipc" (local Rust<->Python subprocess round-trip — a liveness
    /// check, not real Salt transport), "job_ping" (periodic async test.ping,
    /// timed from dispatch to first real return over the actual event bus),
    /// or "master_stats" (Salt's own master_stats instrumentation, requires
    /// master_stats: True in the master's own config). Unrecognized values
    /// fall back to "agent_ipc" behavior.
    pub zmq_latency_source: String,
    /// Latency (ms) at the top of the ZMQ graph's log scale — drawn red, and
    /// anything slower is clipped there. 0 means the default for the
    /// configured zmq_latency_source (see `zmq_latency_ceiling`).
    pub zmq_latency_ceiling_ms: f64,
    /// Which of the Dashboard's bottom sparkline panels are shown. ZMQ /
    /// Events / Activity default on (today's fixed set); CPU / Memory
    /// default off since enabling them changes the existing layout's panel
    /// count — opt-in rather than a default appearance change.
    pub show_zmq_graph: bool,
    pub show_events_graph: bool,
    pub show_activity_graph: bool,
    pub show_cpu_graph: bool,
    pub show_mem_graph: bool,
    /// Stacking order of the Dashboard's sparkline panels, as a comma-
    /// separated list of panel keys ("cpu", "mem", "zmq", "events",
    /// "activity"). Unrecognized/duplicate tokens are ignored; any panel
    /// missing from the list is appended after the ones named, so a partial
    /// or malformed value degrades gracefully instead of losing a panel.
    pub graph_order: String,
    /// How the Dashboard's sparklines are drawn: "dots" or "bars" (`m`).
    pub graph_style: String,
    /// Which of the Dashboard's fixed (non-sparkline) panels are shown, all
    /// defaulting on so existing configs/behavior are unaffected. salt-api
    /// still only ever renders when the process is actually detected —
    /// this just adds a user opt-out on top of that.
    pub show_host_resources: bool,
    pub show_services: bool,
    pub show_key_status: bool,
    pub show_event_types: bool,
    pub show_job_pulse: bool,
    pub show_salt_api: bool,
    /// Whether the Minions tab's Fleet Overview breakdown (OS/version/arch)
    /// scopes itself to the active list filter, or always aggregates the
    /// whole fleet regardless of what's filtered on the left. Toggled live
    /// with `f` on the Minions tab; this is just the startup default.
    pub minions_follow_filter: bool,
    /// Default directory `s`/`S` ("save to file") writes into, from any
    /// popup that supports it. `~` (or `~/...`) is expanded against $HOME
    /// at save time, not at load time, so changing $HOME between launches
    /// (or editing this value live in the Config Editor) takes effect
    /// immediately.
    pub save_dir: String,
    /// How long (seconds) a Job Detail lookup (`salt-run jobs.lookup_jid`)
    /// may run before it's reported as timed out. Huge jobs (e.g.
    /// `grains.items` across thousands of minions) legitimately take tens of
    /// seconds or more to read back from the job cache.
    pub lookup_timeout_secs: u64,
    /// Which columns each list table shows, and in what order, as comma-
    /// separated column keys (see types::TableId::all_columns). Any valid
    /// key left out is hidden.
    pub jobs_live_columns: String,
    pub jobs_cache_columns: String,
    pub minion_columns: String,
    pub event_columns: String,
    pub alert_columns: String,
    pub api_request_columns: String,
    /// Theme name (see `theme_names`) or path to a `.theme` file.
    pub color_theme: String,
    /// When false, the theme's screen backgrounds are left to the
    /// terminal (transparent); popups and the cursor keep theirs.
    pub theme_background: bool,
    /// How strongly the screen behind a popup is faded toward the theme's
    /// background, 0-100% (0 = no dimming).
    pub popup_dim_pct: u64,
    /// Whether `q` asks before quitting, so extra presses meant for a popup
    /// that lag already closed don't quit.
    pub confirm_quit: bool,
    /// Mouse capture (clicks, wheel), toggled with F2.
    pub mouse: bool,
    /// Width of each split tab's left panel, in percent (`Ctrl+←`/`→`).
    pub minions_split_pct: u64,
    pub events_split_pct: u64,
    pub jobs_split_pct: u64,
    pub api_split_pct: u64,
    pub pulse_split_pct: u64,
}

pub const SPLIT_PCT_MIN: u64 = 20;
pub const SPLIT_PCT_MAX: u64 = 80;

impl AppConfig {
    /// The left panel's width setting of a tab split in two side by side.
    pub fn split_pct_mut(&mut self, tab: Tab) -> Option<&mut u64> {
        match tab {
            Tab::Minions => Some(&mut self.minions_split_pct),
            Tab::Events => Some(&mut self.events_split_pct),
            Tab::Jobs => Some(&mut self.jobs_split_pct),
            Tab::Api => Some(&mut self.api_split_pct),
            Tab::Pulse => Some(&mut self.pulse_split_pct),
            Tab::Dashboard | Tab::Alerts => None,
        }
    }

    pub fn split_pct(&self, tab: Tab) -> u16 {
        let pct = match tab {
            Tab::Minions => self.minions_split_pct,
            Tab::Events => self.events_split_pct,
            Tab::Jobs => self.jobs_split_pct,
            Tab::Api => self.api_split_pct,
            Tab::Pulse => self.pulse_split_pct,
            Tab::Dashboard | Tab::Alerts => 100,
        };
        pct.min(100) as u16
    }

    pub fn sparkline_style(&self) -> SparklineStyle {
        if self.graph_style == "bars" { SparklineStyle::Bars } else { SparklineStyle::Dots }
    }

    /// The ZMQ graph's top-of-scale latency: `zmq_latency_ceiling_ms`, or
    /// when unset, a default sized to what the source measures — a local
    /// agent round trip is a few ms when healthy, a real test.ping over the
    /// bus or Salt's own command timings are hundreds.
    pub fn zmq_latency_ceiling(&self) -> f64 {
        if self.zmq_latency_ceiling_ms > 0.0 {
            return self.zmq_latency_ceiling_ms;
        }
        match self.zmq_latency_source.as_str() {
            "job_ping" => 1000.0,
            "master_stats" => 500.0,
            _ => 100.0,
        }
    }

    pub fn columns_for(&self, table: TableId) -> &str {
        match table {
            TableId::LiveJobs => &self.jobs_live_columns,
            TableId::JobCache => &self.jobs_cache_columns,
            TableId::Minions => &self.minion_columns,
            TableId::Events => &self.event_columns,
            TableId::Alerts => &self.alert_columns,
            TableId::ApiLog => &self.api_request_columns,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            master_config: "/etc/salt/master".into(),
            python_bin: "/opt/saltstack/salt/bin/python3".into(),
            agent_path: "/usr/share/salt-radar/salt_radar_agent.py".into(),
            dashboard_secs: 2,
            minions_secs: 5,
            config_secs: 60,
            tick_ms: 2000,
            max_output_kbs: 0,
            event_buffer_size: 10000,
            sniffer_buffer_size: 1000,
            stale_minion_secs: 3600,
            pending_key_secs: 3600,
            job_straggler_secs: 120,
            disk_usage_pct: 85.0,
            auth_storm_rate_per_sec: 5.0,
            worker_saturation_pct: 85.0,
            pending_key_churn_per_min: 10,
            service_flap_threshold_24h: 3,
            service_mem_growth_mb_per_hour: 50.0,
            service_mem_abs_threshold_mb: 2048.0,
            service_mem_pct_threshold: 8.0,
            api_auth_latency_warn_secs: 3.0,
            api_stats_window: 200,
            api_log_cap: 1000,
            api_log_max_age_secs: 3600,
            api_hide_stats_polls: true,
            api_stats_auth_mode: "none".into(),
            api_stats_auth_eauth: "pam".into(),
            api_stats_auth_username: String::new(),
            api_stats_auth_password: String::new(),
            api_stats_auth_username_pillar_path: String::new(),
            api_stats_auth_password_pillar_path: String::new(),
            service_mem_abs_threshold_master_mb: 0.0,
            service_mem_pct_threshold_master: 0.0,
            service_mem_abs_threshold_minion_mb: 0.0,
            service_mem_pct_threshold_minion: 0.0,
            service_mem_abs_threshold_api_mb: 0.0,
            service_mem_pct_threshold_api: 0.0,
            service_mem_abs_threshold_syndic_mb: 0.0,
            service_mem_pct_threshold_syndic: 0.0,
            zmq_latency_source: "agent_ipc".into(),
            zmq_latency_ceiling_ms: 0.0,
            show_zmq_graph: true,
            show_events_graph: true,
            show_activity_graph: true,
            show_cpu_graph: true,
            show_mem_graph: true,
            graph_order: "cpu,mem,zmq,events,activity".into(),
            graph_style: "dots".into(),
            show_host_resources: true,
            show_services: true,
            show_key_status: true,
            show_event_types: true,
            show_job_pulse: true,
            show_salt_api: true,
            minions_follow_filter: true,
            save_dir: "~".into(),
            lookup_timeout_secs: 300,
            jobs_live_columns: TableId::LiveJobs.default_columns().into(),
            jobs_cache_columns: TableId::JobCache.default_columns().into(),
            minion_columns: TableId::Minions.default_columns().into(),
            event_columns: TableId::Events.default_columns().into(),
            alert_columns: TableId::Alerts.default_columns().into(),
            api_request_columns: TableId::ApiLog.default_columns().into(),
            color_theme: DEFAULT_THEME.into(),
            theme_background: true,
            popup_dim_pct: 70,
            confirm_quit: true,
            mouse: true,
            minions_split_pct: 55,
            events_split_pct: 50,
            jobs_split_pct: 55,
            api_split_pct: 65,
            pulse_split_pct: 65,
        }
    }
}

#[derive(serde::Deserialize)]
struct TomlConfig {
    salt: Option<SaltConfig>,
    refresh: Option<RefreshConfig>,
    events: Option<EventsConfig>,
    alerts: Option<AlertsConfig>,
    dashboard: Option<DashboardConfig>,
    api: Option<ApiConfig>,
    minions: Option<MinionsConfig>,
    export: Option<ExportConfig>,
    jobs: Option<JobsConfig>,
    ui: Option<UiConfig>,
}

#[derive(serde::Deserialize)]
struct SaltConfig {
    master_config: Option<String>,
    python_bin: Option<String>,
    agent_path: Option<String>,
}

#[derive(serde::Deserialize)]
struct RefreshConfig {
    dashboard_secs: Option<u64>,
    minions_secs: Option<u64>,
    config_secs: Option<u64>,
    tick_ms: Option<u64>,
    max_output_kbs: Option<u64>,
}

#[derive(serde::Deserialize)]
struct EventsConfig {
    buffer_size: Option<usize>,
    sniffer_buffer_size: Option<usize>,
    event_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct AlertsConfig {
    stale_minion_secs: Option<u64>,
    pending_key_secs: Option<u64>,
    job_straggler_secs: Option<u64>,
    disk_usage_pct: Option<f64>,
    auth_storm_rate_per_sec: Option<f64>,
    worker_saturation_pct: Option<f64>,
    pending_key_churn_per_min: Option<u64>,
    service_flap_threshold_24h: Option<u64>,
    service_mem_growth_mb_per_hour: Option<f64>,
    service_mem_abs_threshold_mb: Option<f64>,
    service_mem_pct_threshold: Option<f64>,
    api_auth_latency_warn_secs: Option<f64>,
    service_mem_abs_threshold_master_mb: Option<f64>,
    service_mem_pct_threshold_master: Option<f64>,
    service_mem_abs_threshold_minion_mb: Option<f64>,
    service_mem_pct_threshold_minion: Option<f64>,
    service_mem_abs_threshold_api_mb: Option<f64>,
    service_mem_pct_threshold_api: Option<f64>,
    service_mem_abs_threshold_syndic_mb: Option<f64>,
    service_mem_pct_threshold_syndic: Option<f64>,
    alert_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct ApiConfig {
    stats_window: Option<usize>,
    log_cap: Option<usize>,
    log_max_age_secs: Option<u64>,
    hide_stats_polls: Option<bool>,
    stats_auth_mode: Option<String>,
    stats_auth_eauth: Option<String>,
    stats_auth_username: Option<String>,
    stats_auth_password: Option<String>,
    stats_auth_username_pillar_path: Option<String>,
    stats_auth_password_pillar_path: Option<String>,
    request_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct MinionsConfig {
    follow_filter: Option<bool>,
    minion_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct ExportConfig {
    save_dir: Option<String>,
}

#[derive(serde::Deserialize)]
struct JobsConfig {
    lookup_timeout_secs: Option<u64>,
    live_columns: Option<String>,
    cache_columns: Option<String>,
}

#[derive(serde::Deserialize)]
struct UiConfig {
    color_theme: Option<String>,
    theme_background: Option<bool>,
    popup_dim_pct: Option<u64>,
    confirm_quit: Option<bool>,
    mouse: Option<bool>,
    minions_split_pct: Option<u64>,
    events_split_pct: Option<u64>,
    jobs_split_pct: Option<u64>,
    api_split_pct: Option<u64>,
    pulse_split_pct: Option<u64>,
}

#[derive(serde::Deserialize)]
struct DashboardConfig {
    zmq_latency_source: Option<String>,
    zmq_latency_ceiling_ms: Option<f64>,
    show_zmq_graph: Option<bool>,
    show_events_graph: Option<bool>,
    show_activity_graph: Option<bool>,
    show_cpu_graph: Option<bool>,
    show_mem_graph: Option<bool>,
    graph_order: Option<String>,
    graph_style: Option<String>,
    show_host_resources: Option<bool>,
    show_services: Option<bool>,
    show_key_status: Option<bool>,
    show_event_types: Option<bool>,
    show_job_pulse: Option<bool>,
    show_salt_api: Option<bool>,
}

/// Converts a setting's value for the config file.
trait ToToml {
    fn to_toml(&self) -> toml::Value;
}

impl ToToml for String {
    fn to_toml(&self) -> toml::Value { toml::Value::String(self.clone()) }
}
impl ToToml for bool {
    fn to_toml(&self) -> toml::Value { toml::Value::Boolean(*self) }
}
impl ToToml for f64 {
    fn to_toml(&self) -> toml::Value { toml::Value::Float(*self) }
}
impl ToToml for u64 {
    fn to_toml(&self) -> toml::Value { toml::Value::Integer(*self as i64) }
}
impl ToToml for usize {
    fn to_toml(&self) -> toml::Value { toml::Value::Integer(*self as i64) }
}

/// Every setting as (section, key, value), in file order — what
/// `write_config` compares against the defaults.
fn config_entries(c: &AppConfig) -> Vec<(&'static str, &'static str, toml::Value)> {
    vec![
        ("salt", "agent_path", c.agent_path.to_toml()),
        ("salt", "python_bin", c.python_bin.to_toml()),
        ("salt", "master_config", c.master_config.to_toml()),
        ("refresh", "dashboard_secs", c.dashboard_secs.to_toml()),
        ("refresh", "minions_secs", c.minions_secs.to_toml()),
        ("refresh", "config_secs", c.config_secs.to_toml()),
        ("refresh", "tick_ms", c.tick_ms.to_toml()),
        ("refresh", "max_output_kbs", c.max_output_kbs.to_toml()),
        ("events", "buffer_size", c.event_buffer_size.to_toml()),
        ("events", "sniffer_buffer_size", c.sniffer_buffer_size.to_toml()),
        ("events", "event_columns", c.event_columns.to_toml()),
        ("alerts", "stale_minion_secs", c.stale_minion_secs.to_toml()),
        ("alerts", "pending_key_secs", c.pending_key_secs.to_toml()),
        ("alerts", "job_straggler_secs", c.job_straggler_secs.to_toml()),
        ("alerts", "disk_usage_pct", c.disk_usage_pct.to_toml()),
        ("alerts", "auth_storm_rate_per_sec", c.auth_storm_rate_per_sec.to_toml()),
        ("alerts", "worker_saturation_pct", c.worker_saturation_pct.to_toml()),
        ("alerts", "pending_key_churn_per_min", c.pending_key_churn_per_min.to_toml()),
        ("alerts", "service_flap_threshold_24h", c.service_flap_threshold_24h.to_toml()),
        ("alerts", "service_mem_growth_mb_per_hour", c.service_mem_growth_mb_per_hour.to_toml()),
        ("alerts", "service_mem_abs_threshold_mb", c.service_mem_abs_threshold_mb.to_toml()),
        ("alerts", "service_mem_pct_threshold", c.service_mem_pct_threshold.to_toml()),
        ("alerts", "api_auth_latency_warn_secs", c.api_auth_latency_warn_secs.to_toml()),
        ("alerts", "service_mem_abs_threshold_master_mb", c.service_mem_abs_threshold_master_mb.to_toml()),
        ("alerts", "service_mem_pct_threshold_master", c.service_mem_pct_threshold_master.to_toml()),
        ("alerts", "service_mem_abs_threshold_minion_mb", c.service_mem_abs_threshold_minion_mb.to_toml()),
        ("alerts", "service_mem_pct_threshold_minion", c.service_mem_pct_threshold_minion.to_toml()),
        ("alerts", "service_mem_abs_threshold_api_mb", c.service_mem_abs_threshold_api_mb.to_toml()),
        ("alerts", "service_mem_pct_threshold_api", c.service_mem_pct_threshold_api.to_toml()),
        ("alerts", "service_mem_abs_threshold_syndic_mb", c.service_mem_abs_threshold_syndic_mb.to_toml()),
        ("alerts", "service_mem_pct_threshold_syndic", c.service_mem_pct_threshold_syndic.to_toml()),
        ("alerts", "alert_columns", c.alert_columns.to_toml()),
        ("api", "stats_window", c.api_stats_window.to_toml()),
        ("api", "log_cap", c.api_log_cap.to_toml()),
        ("api", "log_max_age_secs", c.api_log_max_age_secs.to_toml()),
        ("api", "hide_stats_polls", c.api_hide_stats_polls.to_toml()),
        ("api", "stats_auth_mode", c.api_stats_auth_mode.to_toml()),
        ("api", "stats_auth_eauth", c.api_stats_auth_eauth.to_toml()),
        ("api", "stats_auth_username", c.api_stats_auth_username.to_toml()),
        ("api", "stats_auth_password", c.api_stats_auth_password.to_toml()),
        ("api", "stats_auth_username_pillar_path", c.api_stats_auth_username_pillar_path.to_toml()),
        ("api", "stats_auth_password_pillar_path", c.api_stats_auth_password_pillar_path.to_toml()),
        ("api", "request_columns", c.api_request_columns.to_toml()),
        ("dashboard", "zmq_latency_source", c.zmq_latency_source.to_toml()),
        ("dashboard", "zmq_latency_ceiling_ms", c.zmq_latency_ceiling_ms.to_toml()),
        ("dashboard", "show_zmq_graph", c.show_zmq_graph.to_toml()),
        ("dashboard", "show_events_graph", c.show_events_graph.to_toml()),
        ("dashboard", "show_activity_graph", c.show_activity_graph.to_toml()),
        ("dashboard", "show_cpu_graph", c.show_cpu_graph.to_toml()),
        ("dashboard", "show_mem_graph", c.show_mem_graph.to_toml()),
        ("dashboard", "graph_order", c.graph_order.to_toml()),
        ("dashboard", "graph_style", c.graph_style.to_toml()),
        ("dashboard", "show_host_resources", c.show_host_resources.to_toml()),
        ("dashboard", "show_services", c.show_services.to_toml()),
        ("dashboard", "show_key_status", c.show_key_status.to_toml()),
        ("dashboard", "show_event_types", c.show_event_types.to_toml()),
        ("dashboard", "show_job_pulse", c.show_job_pulse.to_toml()),
        ("dashboard", "show_salt_api", c.show_salt_api.to_toml()),
        ("minions", "follow_filter", c.minions_follow_filter.to_toml()),
        ("minions", "minion_columns", c.minion_columns.to_toml()),
        ("export", "save_dir", c.save_dir.to_toml()),
        ("jobs", "lookup_timeout_secs", c.lookup_timeout_secs.to_toml()),
        ("jobs", "live_columns", c.jobs_live_columns.to_toml()),
        ("jobs", "cache_columns", c.jobs_cache_columns.to_toml()),
        ("ui", "color_theme", c.color_theme.to_toml()),
        ("ui", "theme_background", c.theme_background.to_toml()),
        ("ui", "popup_dim_pct", c.popup_dim_pct.to_toml()),
        ("ui", "confirm_quit", c.confirm_quit.to_toml()),
        ("ui", "mouse", c.mouse.to_toml()),
        ("ui", "minions_split_pct", c.minions_split_pct.to_toml()),
        ("ui", "events_split_pct", c.events_split_pct.to_toml()),
        ("ui", "jobs_split_pct", c.jobs_split_pct.to_toml()),
        ("ui", "api_split_pct", c.api_split_pct.to_toml()),
        ("ui", "pulse_split_pct", c.pulse_split_pct.to_toml()),
    ]
}

/// Writes only the settings that differ from the built-in defaults, so the
/// rest keep following future defaults instead of being pinned to whatever
/// they were when the file was saved. (Files written by older versions held
/// every setting; saving once slims them down to the real changes.)
/// Writes the settings that differ from the defaults, atomically (a temp
/// file renamed over the old one), so two instances saving at once can't
/// leave a torn file. Under sudo, a config in the user's home stays theirs.
pub fn write_config(path: &Path, config: &AppConfig) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            create_dir_all_owned(parent)?;
        }
    }
    let mut out = toml::Table::new();
    for ((section, key, value), (_, _, default)) in config_entries(config).into_iter().zip(config_entries(&AppConfig::default())) {
        if value == default {
            continue;
        }
        if let toml::Value::Table(t) = out.entry(section).or_insert_with(|| toml::Value::Table(toml::Table::new())) {
            t.insert(key.to_string(), value);
        }
    }
    let body = toml::to_string(&out).map_err(std::io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, format!(
        "# salt-radar settings: only values that differ from the defaults are stored.\n\
         # See salt-radar.toml.example for every setting.\n\n{}",
        body,
    ))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    give_to_sudo_user(path);
    Ok(())
}

/// `load_config`, but under sudo a user without a config of their own yet
/// starts from root's — their first save then writes their own file.
pub fn load_config_or_fallback(path: &Path) -> AppConfig {
    if !path.exists() && sudo_user().is_some() {
        if let Some(own) = own_config_dir().map(|d| d.join("salt-radar.toml")).filter(|p| p.is_file()) {
            return load_config(&own);
        }
    }
    load_config(path)
}

pub fn load_config(path: &Path) -> AppConfig {
    let mut cfg = AppConfig::default();
    let data = std::fs::read_to_string(path).ok();
    if let Some(data) = data {
        if let Ok(toml_cfg) = toml::from_str::<TomlConfig>(&data) {
            if let Some(s) = toml_cfg.salt {
                if let Some(v) = s.master_config { cfg.master_config = v; }
                if let Some(v) = s.python_bin { cfg.python_bin = v; }
                if let Some(v) = s.agent_path { cfg.agent_path = v; }
            }
            if let Some(r) = toml_cfg.refresh {
                if let Some(v) = r.dashboard_secs { cfg.dashboard_secs = v; }
                if let Some(v) = r.minions_secs { cfg.minions_secs = v; }
                if let Some(v) = r.config_secs { cfg.config_secs = v; }
                if let Some(v) = r.tick_ms { cfg.tick_ms = v.clamp(TICK_MS_MIN, TICK_MS_MAX); }
                if let Some(v) = r.max_output_kbs { cfg.max_output_kbs = v; }
            }
            if let Some(e) = toml_cfg.events {
                if let Some(v) = e.buffer_size { cfg.event_buffer_size = v; }
                if let Some(v) = e.sniffer_buffer_size { cfg.sniffer_buffer_size = v; }
                if let Some(v) = e.event_columns { cfg.event_columns = v; }
            }
            if let Some(a) = toml_cfg.alerts {
                if let Some(v) = a.stale_minion_secs { cfg.stale_minion_secs = v; }
                if let Some(v) = a.pending_key_secs { cfg.pending_key_secs = v; }
                if let Some(v) = a.job_straggler_secs { cfg.job_straggler_secs = v; }
                if let Some(v) = a.disk_usage_pct { cfg.disk_usage_pct = v; }
                if let Some(v) = a.auth_storm_rate_per_sec { cfg.auth_storm_rate_per_sec = v; }
                if let Some(v) = a.worker_saturation_pct { cfg.worker_saturation_pct = v; }
                if let Some(v) = a.pending_key_churn_per_min { cfg.pending_key_churn_per_min = v; }
                if let Some(v) = a.service_flap_threshold_24h { cfg.service_flap_threshold_24h = v; }
                if let Some(v) = a.service_mem_growth_mb_per_hour { cfg.service_mem_growth_mb_per_hour = v; }
                if let Some(v) = a.service_mem_abs_threshold_mb { cfg.service_mem_abs_threshold_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold { cfg.service_mem_pct_threshold = v; }
                if let Some(v) = a.api_auth_latency_warn_secs { cfg.api_auth_latency_warn_secs = v; }
                if let Some(v) = a.service_mem_abs_threshold_master_mb { cfg.service_mem_abs_threshold_master_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold_master { cfg.service_mem_pct_threshold_master = v; }
                if let Some(v) = a.service_mem_abs_threshold_minion_mb { cfg.service_mem_abs_threshold_minion_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold_minion { cfg.service_mem_pct_threshold_minion = v; }
                if let Some(v) = a.service_mem_abs_threshold_api_mb { cfg.service_mem_abs_threshold_api_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold_api { cfg.service_mem_pct_threshold_api = v; }
                if let Some(v) = a.service_mem_abs_threshold_syndic_mb { cfg.service_mem_abs_threshold_syndic_mb = v; }
                if let Some(v) = a.service_mem_pct_threshold_syndic { cfg.service_mem_pct_threshold_syndic = v; }
                if let Some(v) = a.alert_columns { cfg.alert_columns = v; }
            }
            if let Some(a) = toml_cfg.api {
                if let Some(v) = a.stats_window { cfg.api_stats_window = v; }
                if let Some(v) = a.log_cap { cfg.api_log_cap = v; }
                if let Some(v) = a.log_max_age_secs { cfg.api_log_max_age_secs = v; }
                if let Some(v) = a.hide_stats_polls { cfg.api_hide_stats_polls = v; }
                if let Some(v) = a.stats_auth_mode { cfg.api_stats_auth_mode = v; }
                if let Some(v) = a.stats_auth_eauth { cfg.api_stats_auth_eauth = v; }
                if let Some(v) = a.stats_auth_username { cfg.api_stats_auth_username = v; }
                if let Some(v) = a.stats_auth_password { cfg.api_stats_auth_password = v; }
                if let Some(v) = a.stats_auth_username_pillar_path { cfg.api_stats_auth_username_pillar_path = v; }
                if let Some(v) = a.stats_auth_password_pillar_path { cfg.api_stats_auth_password_pillar_path = v; }
                if let Some(v) = a.request_columns { cfg.api_request_columns = v; }
            }
            if let Some(d) = toml_cfg.dashboard {
                if let Some(v) = d.zmq_latency_source { cfg.zmq_latency_source = v; }
                if let Some(v) = d.zmq_latency_ceiling_ms { cfg.zmq_latency_ceiling_ms = v.max(0.0); }
                if let Some(v) = d.show_zmq_graph { cfg.show_zmq_graph = v; }
                if let Some(v) = d.show_events_graph { cfg.show_events_graph = v; }
                if let Some(v) = d.show_activity_graph { cfg.show_activity_graph = v; }
                if let Some(v) = d.show_cpu_graph { cfg.show_cpu_graph = v; }
                if let Some(v) = d.show_mem_graph { cfg.show_mem_graph = v; }
                if let Some(v) = d.graph_order { cfg.graph_order = v; }
                if let Some(v) = d.graph_style { cfg.graph_style = v; }
                if let Some(v) = d.show_host_resources { cfg.show_host_resources = v; }
                if let Some(v) = d.show_services { cfg.show_services = v; }
                if let Some(v) = d.show_key_status { cfg.show_key_status = v; }
                if let Some(v) = d.show_event_types { cfg.show_event_types = v; }
                if let Some(v) = d.show_job_pulse { cfg.show_job_pulse = v; }
                if let Some(v) = d.show_salt_api { cfg.show_salt_api = v; }
            }
            if let Some(m) = toml_cfg.minions {
                if let Some(v) = m.follow_filter { cfg.minions_follow_filter = v; }
                if let Some(v) = m.minion_columns { cfg.minion_columns = v; }
            }
            if let Some(e) = toml_cfg.export {
                if let Some(v) = e.save_dir { cfg.save_dir = v; }
            }
            if let Some(j) = toml_cfg.jobs {
                if let Some(v) = j.lookup_timeout_secs { cfg.lookup_timeout_secs = v.max(1); }
                if let Some(v) = j.live_columns { cfg.jobs_live_columns = v; }
                if let Some(v) = j.cache_columns { cfg.jobs_cache_columns = v; }
            }
            if let Some(u) = toml_cfg.ui {
                if let Some(v) = u.color_theme { cfg.color_theme = v; }
                if let Some(v) = u.theme_background { cfg.theme_background = v; }
                if let Some(v) = u.popup_dim_pct { cfg.popup_dim_pct = v.min(100); }
                if let Some(v) = u.confirm_quit { cfg.confirm_quit = v; }
                if let Some(v) = u.mouse { cfg.mouse = v; }
                let split = |v: u64| v.clamp(SPLIT_PCT_MIN, SPLIT_PCT_MAX);
                if let Some(v) = u.minions_split_pct { cfg.minions_split_pct = split(v); }
                if let Some(v) = u.events_split_pct { cfg.events_split_pct = split(v); }
                if let Some(v) = u.jobs_split_pct { cfg.jobs_split_pct = split(v); }
                if let Some(v) = u.api_split_pct { cfg.api_split_pct = split(v); }
                if let Some(v) = u.pulse_split_pct { cfg.pulse_split_pct = split(v); }
            }
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty scratch directory under the system temp dir.
    #[test]
    fn config_save_is_atomic_and_creates_its_directories() {
        let dir = scratch("atomic");
        let path = dir.join("a/b/salt-radar.toml");
        let cfg = AppConfig { popup_dim_pct: 40, ..AppConfig::default() };
        write_config(&path, &cfg).unwrap();
        write_config(&path, &cfg).unwrap();
        assert_eq!(load_config(&path).popup_dim_pct, 40);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap()
            .filter_map(|e| e.ok()).map(|e| e.file_name()).filter(|n| n != "salt-radar.toml").collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("salt-radar-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn theme_line_parses_btop_format() {
        assert_eq!(parse_theme_line(r##"theme[main_bg]="#101010""##), Some(("main_bg", "#101010")));
        assert_eq!(parse_theme_line(r##"  theme[ title ] = "#ff00ff" "##), Some(("title", "#ff00ff")));
        assert_eq!(parse_theme_line(r##"theme[main_bg]="""##), Some(("main_bg", "")));
        assert_eq!(parse_theme_line("# a comment"), None);
        assert_eq!(parse_theme_line("main_bg=#101010"), None);
    }

    #[test]
    fn theme_color_accepts_all_btop_value_forms() {
        assert_eq!(parse_theme_color("#ff8000"), Some(Color::Rgb(255, 128, 0)));
        assert_eq!(parse_theme_color("#80"), Some(Color::Rgb(128, 128, 128)));
        assert_eq!(parse_theme_color("10 20 30"), Some(Color::Rgb(10, 20, 30)));
        assert_eq!(parse_theme_color(""), Some(Color::Reset));
        assert_eq!(parse_theme_color("#12345"), None);
        assert_eq!(parse_theme_color("#gggggg"), None);
        assert_eq!(parse_theme_color("10 20"), None);
        assert_eq!(parse_theme_color("300 0 0"), None);
        assert_eq!(parse_theme_color("red"), None);
    }

    #[test]
    fn theme_file_overrides_keys_and_keeps_defaults_for_the_rest() {
        let dir = scratch("theme");
        let path = dir.join("test.theme");
        std::fs::write(&path, concat!(
            "# a partial theme\n",
            "theme[main_bg]=\"#10\"\n",
            "theme[main_fg]=\"200 201 202\"\n",
            "theme[no_such_key]=\"#ffffff\"\n",
            "theme[title]=\"not a color\"\n",
        )).unwrap();
        let theme = load_theme(path.to_str().unwrap());
        assert_eq!(theme.main_bg, Color::Rgb(16, 16, 16));
        assert_eq!(theme.main_fg, Color::Rgb(200, 201, 202));
        assert_eq!(theme.title, Theme::DEFAULT.title, "an unparseable value keeps the default");
        assert_eq!(theme.red, Theme::DEFAULT.red, "keys the file doesn't set keep the default");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Every bundled theme keeps job outcomes and highlighted terms apart
    /// and its colored text and search highlights readable.
    #[test]
    fn bundled_themes_keep_status_colors_apart_and_text_readable() {
        // A transparent background is the terminal's own; assume a dark one.
        let rgb = |c: Color| match c {
            Color::Rgb(r, g, b) => (r as f64, g as f64, b as f64),
            _ => (24.0, 24.0, 24.0),
        };
        // "redmean" weighted RGB distance, a cheap perceptual approximation
        let dist = |a: Color, b: Color| {
            let ((r1, g1, b1), (r2, g2, b2)) = (rgb(a), rgb(b));
            let r = (r1 + r2) / 2.0;
            ((2.0 + r / 256.0) * (r1 - r2).powi(2) + 4.0 * (g1 - g2).powi(2) + (2.0 + (255.0 - r) / 256.0) * (b1 - b2).powi(2)).sqrt()
        };
        let lum = |c: Color| {
            let ch = |v: f64| { let v = v / 255.0; if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
            let (r, g, b) = rgb(c);
            0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
        };
        let contrast = |a: Color, b: Color| {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        };
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/themes");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let t = load_theme(path.to_str().unwrap());
            let status = [("green", t.green), ("cyan", t.cyan), ("yellow", t.yellow), ("red", t.red), ("orange", t.orange)];
            let highlights = [("highlight_1", t.highlight_1), ("highlight_2", t.highlight_2), ("highlight_3", t.highlight_3), ("highlight_4", t.highlight_4)];
            // Greyscale tells its states apart by brightness alone, by design.
            if name != "greyscale" {
                for (i, (a, ca)) in status.iter().enumerate() {
                    for (b, cb) in &status[i + 1..] {
                        assert!(dist(*ca, *cb) >= 50.0, "{name}: {a} and {b} are too close");
                    }
                }
                for (i, (a, ca)) in highlights.iter().enumerate() {
                    for (b, cb) in &highlights[i + 1..] {
                        assert!(dist(*ca, *cb) >= 50.0, "{name}: {a} and {b} are too close");
                    }
                    for (b, cb) in status.iter().filter(|(n, _)| *n != "cyan") {
                        assert!(dist(*ca, *cb) >= 50.0, "{name}: {a} looks like the {b} status");
                    }
                }
            }
            for (a, c) in status.iter().chain(&highlights).chain(&[("purple", t.purple)]) {
                assert!(contrast(*c, t.popup_bg) >= 2.5, "{name}: {a} is hard to read on popup_bg");
            }
            let current_fg = if t.popup_bg == Color::Reset { Color::Black } else { t.popup_bg };
            assert!(contrast(current_fg, t.yellow) >= 3.0, "{name}: the current search match is hard to read");
            assert!(contrast(t.hi_fg, t.badge_bg) >= 3.0, "{name}: search matches are hard to read");
            checked += 1;
        }
        assert!(checked > 40, "only {checked} themes found");
    }

    #[test]
    fn greyscale_theme_sets_every_role_to_a_grey() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/themes/greyscale.theme");
        let text = std::fs::read_to_string(path).unwrap();
        let theme = load_theme(path);
        for key in Theme::KEYS {
            assert!(text.contains(&format!("theme[{key}]=")), "greyscale.theme doesn't set {key}");
            let mut t = theme;
            match t.slot(key) {
                Some(Color::Rgb(r, g, b)) => assert!(r == g && g == b, "{key} is not grey"),
                other => panic!("{key}: {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_theme_name_falls_back_to_default() {
        assert_eq!(load_theme("no-such-theme-anywhere"), Theme::DEFAULT);
        assert_eq!(load_theme(DEFAULT_THEME), Theme::DEFAULT);
    }

    #[test]
    fn config_file_values_are_read_and_clamped() {
        let dir = scratch("config");
        let path = dir.join("salt-radar.toml");
        std::fs::write(&path, concat!(
            "[ui]\n",
            "color_theme = \"nord\"\n",
            "theme_background = false\n",
            "popup_dim_pct = 250\n",
            "[dashboard]\n",
            "zmq_latency_ceiling_ms = -5.0\n",
        )).unwrap();
        let cfg = load_config(&path);
        assert_eq!(cfg.color_theme, "nord");
        assert!(!cfg.theme_background);
        assert_eq!(cfg.popup_dim_pct, 100);
        assert_eq!(cfg.zmq_latency_ceiling_ms, 0.0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_config_file_gives_defaults() {
        let cfg = load_config(Path::new("/nonexistent/salt-radar.toml"));
        assert_eq!(cfg.color_theme, DEFAULT_THEME);
        assert!(cfg.theme_background);
        assert_eq!(cfg.popup_dim_pct, 70);
    }

    #[test]
    fn zmq_ceiling_defaults_per_source_unless_set() {
        let mut cfg = AppConfig::default();
        assert_eq!(cfg.zmq_latency_ceiling(), 100.0);
        cfg.zmq_latency_source = "job_ping".into();
        assert_eq!(cfg.zmq_latency_ceiling(), 1000.0);
        cfg.zmq_latency_source = "master_stats".into();
        assert_eq!(cfg.zmq_latency_ceiling(), 500.0);
        cfg.zmq_latency_ceiling_ms = 42.0;
        assert_eq!(cfg.zmq_latency_ceiling(), 42.0);
    }

    #[test]
    fn saving_defaults_writes_no_settings() {
        let dir = scratch("write-defaults");
        let path = dir.join("salt-radar.toml");
        write_config(&path, &AppConfig::default()).unwrap();
        let table: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert!(table.is_empty(), "{:?}", table);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_writes_only_changed_settings() {
        let dir = scratch("write-changed");
        let path = dir.join("salt-radar.toml");
        let cfg = AppConfig { popup_dim_pct: 40, show_cpu_graph: false, ..AppConfig::default() };
        write_config(&path, &cfg).unwrap();
        let table: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(table.len(), 2, "{:?}", table);
        assert_eq!(table["ui"].as_table().unwrap().len(), 1);
        assert_eq!(table["ui"]["popup_dim_pct"].as_integer(), Some(40));
        assert_eq!(table["dashboard"]["show_cpu_graph"].as_bool(), Some(false));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_slims_an_old_full_file_down_to_its_changes() {
        let dir = scratch("write-slim");
        let path = dir.join("salt-radar.toml");
        std::fs::write(&path, "[ui]\ncolor_theme = \"nord\"\npopup_dim_pct = 70\n[refresh]\ndashboard_secs = 2\n").unwrap();
        let cfg = load_config(&path);
        write_config(&path, &cfg).unwrap();
        let table: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(table.len(), 1, "{:?}", table);
        assert_eq!(table["ui"].as_table().unwrap().len(), 1);
        assert_eq!(table["ui"]["color_theme"].as_str(), Some("nord"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_setting_round_trips_through_the_file() {
        let dir = scratch("write-roundtrip");
        let path = dir.join("salt-radar.toml");
        let cfg = AppConfig {
            agent_path: "/opt/agent.py".into(),
            dashboard_secs: 7,
            tick_ms: 3500,
            max_output_kbs: 6,
            event_buffer_size: 1234,
            disk_usage_pct: 91.5,
            api_hide_stats_polls: false,
            api_stats_auth_mode: "pillar".into(),
            zmq_latency_ceiling_ms: 250.0,
            graph_order: "zmq,cpu".into(),
            graph_style: "bars".into(),
            mouse: false,
            minions_follow_filter: false,
            save_dir: "~/exports".into(),
            jobs_cache_columns: "jid,fun".into(),
            color_theme: "nord".into(),
            theme_background: false,
            popup_dim_pct: 33,
            confirm_quit: false,
            events_split_pct: 70,
            pulse_split_pct: 40,
            ..AppConfig::default()
        };
        write_config(&path, &cfg).unwrap();
        let loaded = load_config(&path);
        assert_eq!(config_entries(&loaded), config_entries(&cfg));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn panel_splits_stay_in_range() {
        let dir = scratch("split-range");
        let path = dir.join("salt-radar.toml");
        std::fs::write(&path, "[ui]\nevents_split_pct = 95\napi_split_pct = 3\njobs_split_pct = 40\n").unwrap();
        let cfg = load_config(&path);
        assert_eq!((cfg.events_split_pct, cfg.api_split_pct, cfg.jobs_split_pct), (SPLIT_PCT_MAX, SPLIT_PCT_MIN, 40));
        assert_eq!((cfg.split_pct(Tab::Jobs), cfg.split_pct(Tab::Alerts)), (40, 100));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn expand_home_only_touches_a_leading_tilde() {
        assert_eq!(expand_home("/etc/salt/master"), PathBuf::from("/etc/salt/master"));
        assert_eq!(expand_home("relative/~/x"), PathBuf::from("relative/~/x"));
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(expand_home("~"), PathBuf::from(&home));
            assert_eq!(expand_home("~/exports"), PathBuf::from(home).join("exports"));
        }
    }
}
