//! The Config Editor (F3): its field model, the path browser, opening,
//! editing, saving and live theme preview.

use super::*;

#[derive(Clone, Copy, Debug)]
pub enum ConfigFieldKind {
    Section,
    Str,
    U64,
    F64,
    /// A field with a fixed, small set of valid values — edited via an
    /// inline picker (arrow keys + Enter) instead of free-text entry.
    Choice(&'static [&'static str]),
    /// A comma-separated ordered list (currently just `graph_order`) —
    /// edited by moving a cursor between items (arrow keys) and moving the
    /// selected item up/down in the order (`+`/`-`), instead of hand-typing
    /// the raw comma-separated string where a typo or dropped token is easy
    /// to make and hard to notice.
    Order,
    /// Like `Order`, over a subset of a fixed set of keys: the value lists
    /// the shown items in order, and every key it leaves out is hidden.
    /// `Space` toggles an item between shown and hidden.
    Columns(&'static [&'static str]),
    /// `color_theme`: a `Choice` over the themes found on disk
    /// (`ConfigEditorState::themes`) rather than a fixed list.
    Theme,
    /// A path expected to exist — picked with a file browser
    /// (`PathBrowser`) instead of typed, so it can't be mistyped; `e` in the
    /// browser still allows typing one (e.g. a directory to be created).
    Path(PathKind),
    SaveButton,
}

#[derive(Clone, Debug)]
pub struct ConfigField {
    pub label: &'static str,
    pub label_value: String,
    pub kind: ConfigFieldKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathKind {
    File,
    Dir,
}

impl PathKind {
    /// Whether `raw` (a config value, `~` allowed) names an existing path
    /// of this kind.
    pub fn exists(self, raw: &str) -> bool {
        let p = crate::config::expand_home(raw.trim());
        match self {
            PathKind::File => p.is_file(),
            PathKind::Dir => p.is_dir(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BrowserEntry {
    /// Directory pickers only: choose the directory being shown.
    SelectHere,
    Parent,
    Dir(String),
    File(String),
}

/// File browser for a `ConfigFieldKind::Path` field.
#[derive(Clone, Debug)]
pub struct PathBrowser {
    pub kind: PathKind,
    pub dir: std::path::PathBuf,
    pub entries: Vec<BrowserEntry>,
    pub selected: usize,
    pub show_hidden: bool,
    /// Type-ahead: the name typed so far, and when its last key came in.
    pub typed: String,
    pub typed_at: Option<Instant>,
}

impl PathBrowser {
    /// Opens at the field's current value — its directory, with the file
    /// itself selected — or the nearest existing ancestor if it's missing.
    pub fn open(kind: PathKind, value: &str) -> Self {
        let target = crate::config::expand_home(value.trim());
        let (dir, select) = if target.is_dir() {
            (target.clone(), None)
        } else {
            let name = target.file_name().map(|n| n.to_string_lossy().into_owned());
            let mut dir = target.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| "/".into());
            while !dir.is_dir() {
                dir = dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| "/".into());
            }
            (dir, name)
        };
        let mut b = PathBrowser { kind, dir, entries: Vec::new(), selected: 0, show_hidden: false, typed: String::new(), typed_at: None };
        b.load(select.as_deref());
        b
    }

    /// Lists `dir` (directories first, then files for file pickers),
    /// selecting the entry named `select` if present.
    pub fn load(&mut self, select: Option<&str>) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !self.show_hidden && name.starts_with('.') { continue; }
                if e.path().is_dir() { dirs.push(name); } else { files.push(name); }
            }
        }
        dirs.sort();
        files.sort();
        self.entries.clear();
        if self.kind == PathKind::Dir { self.entries.push(BrowserEntry::SelectHere); }
        if self.dir.parent().is_some() { self.entries.push(BrowserEntry::Parent); }
        self.entries.extend(dirs.into_iter().map(BrowserEntry::Dir));
        if self.kind == PathKind::File { self.entries.extend(files.into_iter().map(BrowserEntry::File)); }
        self.selected = select
            .and_then(|n| self.entries.iter().position(|e| matches!(e, BrowserEntry::Dir(d) | BrowserEntry::File(d) if d == n)))
            .unwrap_or(0);
    }

    /// Type-ahead: extends the typed name with `c` (starting over after a
    /// pause of a second) and selects the first entry starting with it,
    /// case-insensitively; if nothing matches, tries `c` on its own.
    pub fn type_ahead(&mut self, c: char, now: Instant) {
        if self.typed_at.is_none_or(|at| now.duration_since(at) > std::time::Duration::from_secs(1)) {
            self.typed.clear();
        }
        self.typed_at = Some(now);
        self.typed.push(c);
        let find = |prefix: &str| {
            let prefix = prefix.to_lowercase();
            self.entries.iter().position(|e| matches!(e,
                BrowserEntry::Dir(n) | BrowserEntry::File(n) if n.to_lowercase().starts_with(&prefix)))
        };
        if let Some(i) = find(&self.typed) {
            self.selected = i;
        } else if let Some(i) = find(&c.to_string()) {
            self.typed = c.to_string();
            self.selected = i;
        }
    }

    /// Whether a type-ahead name is still being typed (its last key came in
    /// under a second ago).
    pub fn typing(&self, now: Instant) -> bool {
        self.typed_at.is_some_and(|at| now.duration_since(at) <= std::time::Duration::from_secs(1))
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.dir.parent().map(|p| p.to_path_buf()) {
            let from = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
            self.dir = parent;
            self.load(from.as_deref());
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConfigEditorState {
    pub fields: Vec<ConfigField>,
    pub selected: usize,
    pub editing: bool,
    pub input: String,
    /// Cursor position within the option list while editing a `Choice` field
    pub choice_idx: usize,
    pub save_error: Option<String>,
    /// Opened from a table's `v` shortcut straight into one Columns field:
    /// confirming saves the config and closes, cancelling just closes.
    pub quick: bool,
    /// Every field's value as of opening, to tell whether there's anything
    /// unsaved to warn about before closing.
    pub original: Vec<String>,
    /// Set by a first Esc/q with unsaved changes; a second one discards.
    pub confirm_discard: bool,
    /// Theme names offered by the `color_theme` picker.
    pub themes: Vec<String>,
    /// Open file browser for the selected `Path` field.
    pub browser: Option<PathBrowser>,
}

impl ConfigEditorState {
    /// The option list of a picker field (`Choice` or `Theme`).
    pub fn options(&self, kind: ConfigFieldKind) -> Option<Vec<String>> {
        match kind {
            ConfigFieldKind::Choice(options) => Some(options.iter().map(|o| o.to_string()).collect()),
            ConfigFieldKind::Theme => Some(self.themes.clone()),
            _ => None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.fields.iter().zip(&self.original).any(|(f, o)| f.label_value != *o)
    }
}

/// Rows PageUp/PageDown move in the Config Editor's pickers, the same step
/// as in its settings list.
const PICKER_PAGE: usize = 5;

impl App {
    /// Installs the theme to draw with: the configured one, or while the
    /// Config Editor is open, whatever its theme fields currently hold —
    /// including the option highlighted in an open picker, so themes
    /// preview live as you scroll through them (like btop's options menu)
    /// and revert on their own when the edit is cancelled or discarded.
    pub fn sync_theme(&mut self) {
        let mut wanted = (self.config.color_theme.clone(), self.config.theme_background);
        if let Some(ref s) = self.config_editor {
            let current = |label: &str| {
                let i = s.fields.iter().position(|f| f.label == label)?;
                if s.editing && s.selected == i {
                    s.options(s.fields[i].kind)?.get(s.choice_idx).cloned()
                } else {
                    Some(s.fields[i].label_value.clone())
                }
            };
            if let Some(theme) = current("color_theme") { wanted.0 = theme; }
            if let Some(bg) = current("theme_background") { wanted.1 = bg == "on"; }
        }
        if self.applied_theme.as_ref() == Some(&wanted) { return; }
        let mut theme = crate::config::load_theme(&wanted.0);
        if !wanted.1 {
            theme.main_bg = ratatui::style::Color::Reset;
            theme.bar_bg = ratatui::style::Color::Reset;
            theme.meter_bg = ratatui::style::Color::Reset;
        }
        crate::ui::color::set_theme(theme);
        self.applied_theme = Some(wanted);
    }

    /// `popup_dim_pct` to draw with: the configured one, or while the Config
    /// Editor is open the value confirmed in its field, so the dimming behind
    /// the editor previews it (and reverts with a cancelled or discarded edit).
    pub fn popup_dim_pct(&self) -> u64 {
        self.config_editor.as_ref()
            .and_then(|s| s.fields.iter().find(|f| f.label == "popup_dim_pct"))
            .and_then(|f| f.label_value.trim().parse::<u64>().ok())
            .map_or(self.config.popup_dim_pct, |v| v.min(100))
    }

    pub(crate) fn open_columns_picker(&mut self, table: TableId) {
        self.open_config_editor();
        if let Some(ref mut s) = self.config_editor {
            if let Some(idx) = s.fields.iter().position(|f| f.label == table.config_key()) {
                s.selected = idx;
                s.input = s.fields[idx].label_value.clone();
                s.choice_idx = 0;
                s.editing = true;
                self.input_cursor = None;
                s.quick = true;
            }
        }
    }

    pub(crate) fn columns_field(&self, table: TableId) -> ConfigField {
        ConfigField {
            label: table.config_key(),
            label_value: column_keys(&self.columns(table)),
            kind: ConfigFieldKind::Columns(table.all_columns()),
        }
    }

    pub(crate) fn open_config_editor(&mut self) {
        // Clone the current config into an editable state
        let cfg = &self.config;
        self.config_editor = Some(ConfigEditorState {
            fields: vec![
                ConfigField { label: "[salt]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "agent_path", label_value: cfg.agent_path.clone(), kind: ConfigFieldKind::Path(PathKind::File) },
                ConfigField { label: "python_bin", label_value: cfg.python_bin.clone(), kind: ConfigFieldKind::Path(PathKind::File) },
                ConfigField { label: "master_config", label_value: cfg.master_config.clone(), kind: ConfigFieldKind::Path(PathKind::File) },
                ConfigField { label: "[refresh]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "dashboard_secs", label_value: cfg.dashboard_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "minions_secs", label_value: cfg.minions_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "config_secs", label_value: cfg.config_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "tick_ms", label_value: cfg.tick_ms.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "max_output_kbs", label_value: cfg.max_output_kbs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "[events]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "buffer_size", label_value: cfg.event_buffer_size.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "sniffer_buffer_size", label_value: cfg.sniffer_buffer_size.to_string(), kind: ConfigFieldKind::U64 },
                self.columns_field(TableId::Events),
                ConfigField { label: "[alerts]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "stale_minion_secs", label_value: cfg.stale_minion_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "pending_key_secs", label_value: cfg.pending_key_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "job_straggler_secs", label_value: cfg.job_straggler_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "disk_usage_pct", label_value: cfg.disk_usage_pct.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "auth_storm_rate_per_sec", label_value: cfg.auth_storm_rate_per_sec.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "worker_saturation_pct", label_value: cfg.worker_saturation_pct.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "pending_key_churn_per_min", label_value: cfg.pending_key_churn_per_min.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "service_flap_threshold_24h", label_value: cfg.service_flap_threshold_24h.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "service_mem_growth_mb_per_hour", label_value: cfg.service_mem_growth_mb_per_hour.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_mb", label_value: cfg.service_mem_abs_threshold_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold", label_value: cfg.service_mem_pct_threshold.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "api_auth_latency_warn_secs", label_value: cfg.api_auth_latency_warn_secs.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_master_mb", label_value: cfg.service_mem_abs_threshold_master_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold_master", label_value: cfg.service_mem_pct_threshold_master.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_minion_mb", label_value: cfg.service_mem_abs_threshold_minion_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold_minion", label_value: cfg.service_mem_pct_threshold_minion.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_api_mb", label_value: cfg.service_mem_abs_threshold_api_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold_api", label_value: cfg.service_mem_pct_threshold_api.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_abs_threshold_syndic_mb", label_value: cfg.service_mem_abs_threshold_syndic_mb.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "service_mem_pct_threshold_syndic", label_value: cfg.service_mem_pct_threshold_syndic.to_string(), kind: ConfigFieldKind::F64 },
                self.columns_field(TableId::Alerts),
                ConfigField { label: "[api]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "stats_window", label_value: cfg.api_stats_window.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "log_cap", label_value: cfg.api_log_cap.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "log_max_age_secs", label_value: cfg.api_log_max_age_secs.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "hide_stats_polls", label_value: on_off(cfg.api_hide_stats_polls), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "stats_auth_mode", label_value: cfg.api_stats_auth_mode.clone(), kind: ConfigFieldKind::Choice(API_STATS_AUTH_MODES) },
                ConfigField { label: "stats_auth_eauth", label_value: cfg.api_stats_auth_eauth.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "stats_auth_username", label_value: cfg.api_stats_auth_username.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "stats_auth_password", label_value: cfg.api_stats_auth_password.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "stats_auth_username_pillar_path", label_value: cfg.api_stats_auth_username_pillar_path.clone(), kind: ConfigFieldKind::Str },
                ConfigField { label: "stats_auth_password_pillar_path", label_value: cfg.api_stats_auth_password_pillar_path.clone(), kind: ConfigFieldKind::Str },
                self.columns_field(TableId::ApiLog),
                ConfigField { label: "[dashboard]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "zmq_latency_source", label_value: cfg.zmq_latency_source.clone(), kind: ConfigFieldKind::Choice(ZMQ_LATENCY_SOURCES) },
                ConfigField { label: "zmq_latency_ceiling_ms", label_value: cfg.zmq_latency_ceiling_ms.to_string(), kind: ConfigFieldKind::F64 },
                ConfigField { label: "show_zmq_graph", label_value: on_off(cfg.show_zmq_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_events_graph", label_value: on_off(cfg.show_events_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_activity_graph", label_value: on_off(cfg.show_activity_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_cpu_graph", label_value: on_off(cfg.show_cpu_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_mem_graph", label_value: on_off(cfg.show_mem_graph), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_host_resources", label_value: on_off(cfg.show_host_resources), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_services", label_value: on_off(cfg.show_services), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_key_status", label_value: on_off(cfg.show_key_status), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_event_types", label_value: on_off(cfg.show_event_types), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_job_pulse", label_value: on_off(cfg.show_job_pulse), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "show_salt_api", label_value: on_off(cfg.show_salt_api), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "graph_order", label_value: cfg.graph_order.clone(), kind: ConfigFieldKind::Order },
                ConfigField { label: "graph_style", label_value: cfg.graph_style.clone(), kind: ConfigFieldKind::Choice(GRAPH_STYLES) },
                ConfigField { label: "[minions]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "follow_filter", label_value: on_off(cfg.minions_follow_filter), kind: ConfigFieldKind::Choice(ON_OFF) },
                self.columns_field(TableId::Minions),
                ConfigField { label: "[export]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "save_dir", label_value: cfg.save_dir.clone(), kind: ConfigFieldKind::Path(PathKind::Dir) },
                ConfigField { label: "[jobs]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "lookup_timeout_secs", label_value: cfg.lookup_timeout_secs.to_string(), kind: ConfigFieldKind::U64 },
                self.columns_field(TableId::LiveJobs),
                self.columns_field(TableId::JobCache),
                ConfigField { label: "[ui]", label_value: String::new(), kind: ConfigFieldKind::Section },
                ConfigField { label: "color_theme", label_value: cfg.color_theme.clone(), kind: ConfigFieldKind::Theme },
                ConfigField { label: "theme_background", label_value: on_off(cfg.theme_background), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "popup_dim_pct", label_value: cfg.popup_dim_pct.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "confirm_quit", label_value: on_off(cfg.confirm_quit), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "mouse", label_value: on_off(cfg.mouse), kind: ConfigFieldKind::Choice(ON_OFF) },
                ConfigField { label: "minions_split_pct", label_value: cfg.minions_split_pct.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "events_split_pct", label_value: cfg.events_split_pct.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "jobs_split_pct", label_value: cfg.jobs_split_pct.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "api_split_pct", label_value: cfg.api_split_pct.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "pulse_split_pct", label_value: cfg.pulse_split_pct.to_string(), kind: ConfigFieldKind::U64 },
                ConfigField { label: "[Save]", label_value: String::new(), kind: ConfigFieldKind::SaveButton },
            ],
            selected: 1, // first editable field (skip section header)
            editing: false,
            input: String::new(),
            choice_idx: 0,
            save_error: None,
            quick: false,
            original: Vec::new(),
            confirm_discard: false,
            themes: crate::config::theme_names(),
            browser: None,
        });
        if let Some(ref mut s) = self.config_editor {
            s.original = s.fields.iter().map(|f| f.label_value.clone()).collect();
        }
    }

    pub(crate) fn save_config_from_editor(&mut self) -> Result<(), String> {
        let fields = match self.config_editor {
            Some(ref state) => &state.fields,
            _ => return Err("No editor state".into()),
        };
        // Extract values by label
        let mut vals: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for f in fields {
            match f.kind {
                ConfigFieldKind::Str | ConfigFieldKind::U64 | ConfigFieldKind::F64 | ConfigFieldKind::Choice(_) | ConfigFieldKind::Order | ConfigFieldKind::Columns(_) | ConfigFieldKind::Theme | ConfigFieldKind::Path(_) => {
                    vals.insert(f.label, f.label_value.as_str());
                }
                _ => {}
            }
        }
        // Parse and build new config
        let parse_u64 = |key| vals.get(key).and_then(|v| v.parse::<u64>().ok()).ok_or_else(|| format!("Invalid {}: expected integer", key));
        let parse_usize = |key| vals.get(key).and_then(|v| v.parse::<usize>().ok()).ok_or_else(|| format!("Invalid {}: expected integer", key));
        let parse_f64 = |key| vals.get(key).and_then(|v| v.parse::<f64>().ok()).ok_or_else(|| format!("Invalid {}: expected number", key));
        let parse_bool = |key| vals.get(key) == Some(&"on");
        let columns = |key, table: TableId| vals.get(key).copied().unwrap_or(table.default_columns()).to_string();
        let split = |key| parse_u64(key).map(|v| v.clamp(crate::config::SPLIT_PCT_MIN, crate::config::SPLIT_PCT_MAX));

        let new_config = AppConfig {
            agent_path: vals.get("agent_path").unwrap_or(&"").to_string(),
            python_bin: vals.get("python_bin").unwrap_or(&"").to_string(),
            master_config: vals.get("master_config").unwrap_or(&"").to_string(),
            dashboard_secs: parse_u64("dashboard_secs")?,
            minions_secs: parse_u64("minions_secs")?,
            config_secs: parse_u64("config_secs")?,
            tick_ms: parse_u64("tick_ms")?.clamp(crate::config::TICK_MS_MIN, crate::config::TICK_MS_MAX),
            max_output_kbs: parse_u64("max_output_kbs")?,
            event_buffer_size: parse_usize("buffer_size")?,
            sniffer_buffer_size: parse_usize("sniffer_buffer_size")?,
            stale_minion_secs: parse_u64("stale_minion_secs")?,
            pending_key_secs: parse_u64("pending_key_secs")?,
            job_straggler_secs: parse_u64("job_straggler_secs")?,
            disk_usage_pct: parse_f64("disk_usage_pct")?,
            auth_storm_rate_per_sec: parse_f64("auth_storm_rate_per_sec")?,
            worker_saturation_pct: parse_f64("worker_saturation_pct")?,
            pending_key_churn_per_min: parse_u64("pending_key_churn_per_min")?,
            service_flap_threshold_24h: parse_u64("service_flap_threshold_24h")?,
            service_mem_growth_mb_per_hour: parse_f64("service_mem_growth_mb_per_hour")?,
            service_mem_abs_threshold_mb: parse_f64("service_mem_abs_threshold_mb")?,
            service_mem_pct_threshold: parse_f64("service_mem_pct_threshold")?,
            api_auth_latency_warn_secs: parse_f64("api_auth_latency_warn_secs")?,
            service_mem_abs_threshold_master_mb: parse_f64("service_mem_abs_threshold_master_mb")?,
            service_mem_pct_threshold_master: parse_f64("service_mem_pct_threshold_master")?,
            service_mem_abs_threshold_minion_mb: parse_f64("service_mem_abs_threshold_minion_mb")?,
            service_mem_pct_threshold_minion: parse_f64("service_mem_pct_threshold_minion")?,
            service_mem_abs_threshold_api_mb: parse_f64("service_mem_abs_threshold_api_mb")?,
            service_mem_pct_threshold_api: parse_f64("service_mem_pct_threshold_api")?,
            service_mem_abs_threshold_syndic_mb: parse_f64("service_mem_abs_threshold_syndic_mb")?,
            service_mem_pct_threshold_syndic: parse_f64("service_mem_pct_threshold_syndic")?,
            api_stats_window: parse_usize("stats_window")?,
            api_log_cap: parse_usize("log_cap")?,
            api_log_max_age_secs: parse_u64("log_max_age_secs")?,
            api_hide_stats_polls: parse_bool("hide_stats_polls"),
            api_stats_auth_mode: vals.get("stats_auth_mode").unwrap_or(&"none").to_string(),
            api_stats_auth_eauth: vals.get("stats_auth_eauth").unwrap_or(&"pam").to_string(),
            api_stats_auth_username: vals.get("stats_auth_username").unwrap_or(&"").to_string(),
            api_stats_auth_password: vals.get("stats_auth_password").unwrap_or(&"").to_string(),
            api_stats_auth_username_pillar_path: vals.get("stats_auth_username_pillar_path").unwrap_or(&"").to_string(),
            api_stats_auth_password_pillar_path: vals.get("stats_auth_password_pillar_path").unwrap_or(&"").to_string(),
            zmq_latency_source: vals.get("zmq_latency_source").unwrap_or(&"agent_ipc").to_string(),
            zmq_latency_ceiling_ms: parse_f64("zmq_latency_ceiling_ms")?.max(0.0),
            show_zmq_graph: parse_bool("show_zmq_graph"),
            show_events_graph: parse_bool("show_events_graph"),
            show_activity_graph: parse_bool("show_activity_graph"),
            show_cpu_graph: parse_bool("show_cpu_graph"),
            show_mem_graph: parse_bool("show_mem_graph"),
            graph_order: vals.get("graph_order").unwrap_or(&"cpu,mem,zmq,events,activity").to_string(),
            graph_style: vals.get("graph_style").unwrap_or(&"dots").to_string(),
            show_host_resources: parse_bool("show_host_resources"),
            show_services: parse_bool("show_services"),
            show_key_status: parse_bool("show_key_status"),
            show_event_types: parse_bool("show_event_types"),
            show_job_pulse: parse_bool("show_job_pulse"),
            show_salt_api: parse_bool("show_salt_api"),
            minions_follow_filter: parse_bool("follow_filter"),
            save_dir: {
                let v = vals.get("save_dir").unwrap_or(&"~").trim();
                if v.is_empty() { "~".to_string() } else { v.to_string() }
            },
            lookup_timeout_secs: parse_u64("lookup_timeout_secs")?.max(1),
            jobs_live_columns: columns("live_columns", TableId::LiveJobs),
            jobs_cache_columns: columns("cache_columns", TableId::JobCache),
            minion_columns: columns("minion_columns", TableId::Minions),
            event_columns: columns("event_columns", TableId::Events),
            alert_columns: columns("alert_columns", TableId::Alerts),
            api_request_columns: columns("request_columns", TableId::ApiLog),
            color_theme: vals.get("color_theme").unwrap_or(&crate::config::DEFAULT_THEME).to_string(),
            theme_background: parse_bool("theme_background"),
            popup_dim_pct: parse_u64("popup_dim_pct")?.min(100),
            confirm_quit: parse_bool("confirm_quit"),
            mouse: parse_bool("mouse"),
            minions_split_pct: split("minions_split_pct")?,
            events_split_pct: split("events_split_pct")?,
            jobs_split_pct: split("jobs_split_pct")?,
            api_split_pct: split("api_split_pct")?,
            pulse_split_pct: split("pulse_split_pct")?,
        };

        crate::config::write_config(&self.config_path, &new_config)
            .map_err(|e| format!("Write failed: {}", e))?;
        let latency_source_changed = new_config.zmq_latency_source != self.config.zmq_latency_source;
        self.needs_mouse_toggle |= new_config.mouse != self.config.mouse;
        self.persisted_config = new_config.clone();
        self.config = new_config;
        // A sort on a column that was just hidden would keep reordering the
        // table with no header showing why.
        let stale: Vec<TableId> = self.table_sorts.iter()
            .filter(|(t, (c, _))| !self.columns(**t).contains(c))
            .map(|(t, _)| *t)
            .collect();
        for table in stale {
            self.set_sort(table, None);
        }
        // A changed zmq_latency_source shouldn't leave a stale figure — or
        // sparkline history in different units — from the previous source
        // mislabeled under the new one. Any other save keeps the history.
        if latency_source_changed {
            self.zmq_latency_ms = 0.0;
            self.zmq_latency_seen = false;
            self.zmq_latency_history = [0.0; 2000];
            self.zmq_latency_idx = 0;
            self.zmq_ping_sent_at = None;
            self.job_ping_pending = None;
        }
        Ok(())
    }

    pub(crate) fn handle_config_editor_key(&mut self, code: KeyCode) -> bool {
        // --- Editing mode (borrow state) ---
        if let Some(ref mut s) = self.config_editor {
            if let Some(ref mut b) = s.browser {
                let last = b.entries.len().saturating_sub(1);
                let mut picked: Option<String> = None;
                match code {
                    KeyCode::Up => b.selected = b.selected.saturating_sub(1),
                    KeyCode::Down => b.selected = (b.selected + 1).min(last),
                    KeyCode::PageUp => b.selected = b.selected.saturating_sub(10),
                    KeyCode::PageDown => b.selected = (b.selected + 10).min(last),
                    KeyCode::Home => b.selected = 0,
                    KeyCode::End => b.selected = last,
                    KeyCode::Left | KeyCode::Backspace => b.go_up(),
                    KeyCode::Char('.') if !b.typing(Instant::now()) => {
                        b.show_hidden = !b.show_hidden;
                        let keep = match b.entries.get(b.selected) {
                            Some(BrowserEntry::Dir(n)) | Some(BrowserEntry::File(n)) => Some(n.clone()),
                            _ => None,
                        };
                        b.load(keep.as_deref());
                    }
                    KeyCode::Enter | KeyCode::Right => match b.entries.get(b.selected).cloned() {
                        Some(BrowserEntry::SelectHere) => picked = Some(b.dir.display().to_string()),
                        Some(BrowserEntry::Parent) => b.go_up(),
                        Some(BrowserEntry::Dir(name)) => {
                            b.dir = b.dir.join(name);
                            b.load(None);
                        }
                        Some(BrowserEntry::File(name)) if code == KeyCode::Enter => {
                            picked = Some(b.dir.join(name).display().to_string());
                        }
                        _ => {}
                    },
                    KeyCode::Char('/') => {
                        s.browser = None;
                        s.input = s.fields.get(s.selected).map(|f| f.label_value.clone()).unwrap_or_default();
                        s.editing = true;
                        self.input_cursor = None;
                        return false;
                    }
                    KeyCode::Esc => s.browser = None,
                    KeyCode::Char(c) if !c.is_control() => b.type_ahead(c, Instant::now()),
                    _ => {}
                }
                if let Some(path) = picked {
                    if let Some(field) = s.fields.get_mut(s.selected) {
                        field.label_value = path;
                    }
                    s.browser = None;
                }
                return false;
            }
            if s.editing {
                // Choice fields are edited via an inline picker (arrow keys +
                // Enter over the option list) instead of free-text entry.
                if let Some(options) = s.fields.get(s.selected).and_then(|f| s.options(f.kind)) {
                    match code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            s.choice_idx = s.choice_idx.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if s.choice_idx + 1 < options.len() { s.choice_idx += 1; }
                        }
                        KeyCode::PageUp => s.choice_idx = s.choice_idx.saturating_sub(PICKER_PAGE),
                        KeyCode::PageDown => s.choice_idx = (s.choice_idx + PICKER_PAGE).min(options.len().saturating_sub(1)),
                        KeyCode::Home => s.choice_idx = 0,
                        KeyCode::End => s.choice_idx = options.len().saturating_sub(1),
                        KeyCode::Enter => {
                            if let Some(opt) = options.get(s.choice_idx) {
                                if let Some(field) = s.fields.get_mut(s.selected) {
                                    field.label_value = opt.clone();
                                }
                            }
                            s.editing = false;
                        }
                        KeyCode::Esc => {
                            s.editing = false;
                        }
                        _ => {}
                    }
                    return false;
                }
                if let Some(ConfigFieldKind::Columns(all)) = s.fields.get(s.selected).map(|f| f.kind) {
                    let mut finish: Option<bool> = None;
                    if let Some(field) = s.fields.get_mut(s.selected) {
                        let (mut items, shown) = column_editor_items(&field.label_value, all);
                        let i = s.choice_idx.min(items.len().saturating_sub(1));
                        match code {
                            KeyCode::Up | KeyCode::Char('k') => s.choice_idx = i.saturating_sub(1),
                            KeyCode::Down | KeyCode::Char('j') => {
                                if i + 1 < items.len() { s.choice_idx = i + 1; }
                            }
                            KeyCode::PageUp => s.choice_idx = i.saturating_sub(PICKER_PAGE),
                            KeyCode::PageDown => s.choice_idx = (i + PICKER_PAGE).min(items.len().saturating_sub(1)),
                            KeyCode::Home => s.choice_idx = 0,
                            KeyCode::End => s.choice_idx = items.len().saturating_sub(1),
                            KeyCode::Char('+') | KeyCode::Char('=') if i > 0 && i < shown => {
                                items.swap(i, i - 1);
                                field.label_value = items[..shown].join(",");
                                s.choice_idx = i - 1;
                            }
                            KeyCode::Char('-') | KeyCode::Char('_') if i + 1 < shown => {
                                items.swap(i, i + 1);
                                field.label_value = items[..shown].join(",");
                                s.choice_idx = i + 1;
                            }
                            KeyCode::Char(' ') => {
                                let key = items[i];
                                let mut visible = items[..shown].to_vec();
                                if i < shown {
                                    if shown > 1 { visible.remove(i); }
                                } else {
                                    visible.push(key);
                                }
                                field.label_value = visible.join(",");
                                let (items, _) = column_editor_items(&field.label_value, all);
                                s.choice_idx = items.iter().position(|k| *k == key).unwrap_or(0);
                            }
                            KeyCode::Enter => finish = Some(true),
                            KeyCode::Esc => {
                                field.label_value = s.input.clone();
                                finish = Some(false);
                            }
                            _ => {}
                        }
                    }
                    let quick = s.quick;
                    match finish {
                        Some(true) if quick => match self.save_config_from_editor() {
                            Ok(()) => {
                                self.config_editor = None;
                                self.clipboard_msg = Some((" Columns saved ".into(), std::time::Instant::now()));
                            }
                            Err(e) => {
                                if let Some(ref mut s) = self.config_editor {
                                    s.save_error = Some(e);
                                }
                            }
                        },
                        Some(false) if quick => self.config_editor = None,
                        Some(_) => {
                            if let Some(ref mut s) = self.config_editor {
                                s.editing = false;
                                s.input.clear();
                            }
                        }
                        None => {}
                    }
                    return false;
                }
                // Order fields (currently just graph_order): arrows move a
                // cursor between items, +/- move the item under the cursor
                // up/down in the order — no free-text entry, so a typo or
                // dropped token isn't possible.
                if matches!(s.fields.get(s.selected).map(|f| f.kind), Some(ConfigFieldKind::Order)) {
                    match code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            s.choice_idx = s.choice_idx.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let len = s.fields.get(s.selected).map_or(0, |f| f.label_value.split(',').count());
                            if s.choice_idx + 1 < len { s.choice_idx += 1; }
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            if let Some(field) = s.fields.get_mut(s.selected) {
                                let mut items: Vec<&str> = field.label_value.split(',').collect();
                                if s.choice_idx > 0 && s.choice_idx < items.len() {
                                    items.swap(s.choice_idx, s.choice_idx - 1);
                                    field.label_value = items.join(",");
                                    s.choice_idx -= 1;
                                }
                            }
                        }
                        KeyCode::Char('-') | KeyCode::Char('_') => {
                            if let Some(field) = s.fields.get_mut(s.selected) {
                                let mut items: Vec<&str> = field.label_value.split(',').collect();
                                if s.choice_idx + 1 < items.len() {
                                    items.swap(s.choice_idx, s.choice_idx + 1);
                                    field.label_value = items.join(",");
                                    s.choice_idx += 1;
                                }
                            }
                        }
                        KeyCode::Enter => {
                            s.editing = false;
                            s.input.clear();
                        }
                        KeyCode::Esc => {
                            if let Some(field) = s.fields.get_mut(s.selected) {
                                field.label_value = s.input.clone();
                            }
                            s.editing = false;
                            s.input.clear();
                        }
                        _ => {}
                    }
                    return false;
                }
                match code {
                    KeyCode::Enter => {
                        if let Some(field) = s.fields.get_mut(s.selected) {
                            field.label_value = s.input.clone();
                        }
                        s.editing = false;
                        s.input.clear();
                    }
                    KeyCode::Esc => {
                        s.editing = false;
                        s.input.clear();
                    }
                    _ => { super::line_edit::edit(&mut s.input, &mut self.input_cursor, code); }
                }
                return false;
            }
        } else {
            return false;
        }

        // --- Navigation / action mode (borrow state per arm, not function-level) ---
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(ref mut s) = self.config_editor {
                    let mut new_sel = s.selected;
                    loop {
                        if new_sel == 0 { break; }
                        new_sel = new_sel.saturating_sub(1);
                        if let Some(f) = s.fields.get(new_sel) {
                            if !matches!(f.kind, ConfigFieldKind::Section) {
                                s.selected = new_sel;
                                break;
                            }
                        }
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(ref mut s) = self.config_editor {
                    let last = s.fields.len().saturating_sub(1);
                    let mut new_sel = s.selected;
                    loop {
                        if new_sel >= last { break; }
                        new_sel = new_sel.saturating_add(1);
                        if let Some(f) = s.fields.get(new_sel) {
                            if !matches!(f.kind, ConfigFieldKind::Section) {
                                s.selected = new_sel;
                                break;
                            }
                        }
                    }
                }
            }
            KeyCode::PageUp => {
                for _ in 0..5 {
                    if let Some(ref mut s) = self.config_editor {
                        let mut new_sel = s.selected;
                        loop {
                            if new_sel == 0 { break; }
                            new_sel = new_sel.saturating_sub(1);
                            if let Some(f) = s.fields.get(new_sel) {
                                if !matches!(f.kind, ConfigFieldKind::Section) {
                                    s.selected = new_sel;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            KeyCode::PageDown => {
                for _ in 0..5 {
                    if let Some(ref mut s) = self.config_editor {
                        let last = s.fields.len().saturating_sub(1);
                        let mut new_sel = s.selected;
                        loop {
                            if new_sel >= last { break; }
                            new_sel = new_sel.saturating_add(1);
                            if let Some(f) = s.fields.get(new_sel) {
                                if !matches!(f.kind, ConfigFieldKind::Section) {
                                    s.selected = new_sel;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            KeyCode::Home => {
                if let Some(ref mut s) = self.config_editor {
                    s.selected = 1;
                }
            }
            KeyCode::End => {
                if let Some(ref mut s) = self.config_editor {
                    s.selected = s.fields.len().saturating_sub(1);
                }
            }
            KeyCode::Enter => {
                // Determine action from field at current selection (immutable borrow)
                let action = match self.config_editor {
                    Some(ref s) => {
                        match s.fields.get(s.selected).map(|f| &f.kind) {
                            Some(ConfigFieldKind::SaveButton) => Some("save"),
                            Some(ConfigFieldKind::Str) | Some(ConfigFieldKind::U64) | Some(ConfigFieldKind::F64) | Some(ConfigFieldKind::Choice(_)) | Some(ConfigFieldKind::Order) | Some(ConfigFieldKind::Columns(_)) | Some(ConfigFieldKind::Theme) | Some(ConfigFieldKind::Path(_)) => {
                                Some("edit")
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match action {
                    Some("save") => self.save_editor_and_close(),
                    Some("edit") => {
                        let field_info = match self.config_editor {
                            Some(ref s) => {
                                s.fields.get(s.selected).map(|f| (f.label_value.clone(), f.kind))
                            }
                            _ => None,
                        };
                        if let Some(ref mut s) = self.config_editor {
                            if let Some((val, kind)) = field_info {
                                if let ConfigFieldKind::Path(path_kind) = kind {
                                    s.browser = Some(PathBrowser::open(path_kind, &val));
                                    return false;
                                }
                                if let Some(options) = s.options(kind) {
                                    s.choice_idx = options.iter().position(|o| *o == val).unwrap_or(0);
                                } else if let ConfigFieldKind::Order | ConfigFieldKind::Columns(_) = kind {
                                    // s.input snapshots the pre-edit order so Esc can restore it —
                                    // +/- mutate field.label_value live so the list re-renders in
                                    // its new order immediately, not just on confirm.
                                    s.input = val;
                                    s.choice_idx = 0;
                                } else {
                                    s.input = val;
                                }
                                s.editing = true;
                                self.input_cursor = None;
                            }
                        }
                    }
                    _ => {}
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => self.save_editor_and_close(),
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                // Closing used to drop edits silently — easy to lose them by
                // confirming a field and expecting that alone to apply it.
                if let Some(ref mut s) = self.config_editor {
                    if s.dirty() && !s.confirm_discard {
                        s.confirm_discard = true;
                        return false;
                    }
                }
                self.config_editor = None;
            }
            _ => {}
        }
        if !matches!(code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q')) {
            if let Some(ref mut s) = self.config_editor {
                s.confirm_discard = false;
            }
        }
        false
    }

    pub(crate) fn save_editor_and_close(&mut self) {
        match self.save_config_from_editor() {
            Ok(()) => {
                self.config_editor = None;
                self.clipboard_msg = Some((" Config saved ".into(), std::time::Instant::now()));
            }
            Err(e) => {
                if let Some(ref mut s) = self.config_editor {
                    s.save_error = Some(e);
                }
            }
        }
    }
}
