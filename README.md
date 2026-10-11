<p align="center">
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/platform-Linux-1793D1?style=for-the-badge&logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/TUI-ratatui-blue?style=for-the-badge" alt="Ratatui">
  <img src="https://img.shields.io/badge/Salt-monitoring-00E272?style=for-the-badge&logo=saltproject&logoColor=white" alt="Salt">
  <img src="https://img.shields.io/badge/license-MIT-green?style=for-the-badge" alt="MIT License">
</p>

<h1 align="center">salt-radar</h1>

<p align="center">
  <b>A non-intrusive, read-only TUI for monitoring a Salt Master</b><br>
  Built with <a href="https://github.com/ratatui/ratatui">ratatui</a> + <a href="https://github.com/crossterm-rs/crossterm">crossterm</a>
</p>

<br>

salt-radar gives you a live, at-a-glance view of your Salt Master infrastructure — directly in your terminal. It reads from `/proc`, `systemctl`, and Salt's own APIs (via a bundled Python agent) to show you what's happening without needing the Salt CLI or web UI.

No database, no web server, no JavaScript. Just a single binary and a Python agent.

## Screenshots

<table>
<tr><td colspan="2">

**Dashboard** — host resources, real systemd services, minion keys, event types, job pulse, and live ZMQ/event/minion activity sparklines
<img src="docs/screenshots/dashboard.png" alt="Dashboard tab">

</td></tr>
<tr>
<td width="50%">

**Minions** — fleet list plus OS/version/arch distribution
<img src="docs/screenshots/minions.png" alt="Minions tab">

</td>
<td width="50%">

**Events** — live event bus stream with pretty-printed detail preview
<img src="docs/screenshots/events.png" alt="Events tab">

</td>
</tr>
<tr>
<td width="50%">

**Jobs** — live jobs from the event bus alongside the paginated job cache
<img src="docs/screenshots/jobs.png" alt="Jobs tab">

</td>
<td width="50%">

**Alerts** — 14 rules covering minion staleness, keys, service health, memory, disk, jobs, auth, and API latency
<img src="docs/screenshots/alerts.png" alt="Alerts tab">

</td>
</tr>
<tr>
<td width="50%">

**API** — live salt-api request log and per-endpoint breakdown, built from cpstats
<img src="docs/screenshots/api.png" alt="API tab">

</td>
<td width="50%">

**Pulse** — per-minion return activity at a glance, with connection detail on the side
<img src="docs/screenshots/pulse.png" alt="Pulse tab">

</td>
</tr>
<tr><td colspan="2">

**Job Detail** — every minion's return for one job, side by side with its full Salt output; the screen behind any popup is dimmed
<img src="docs/screenshots/job-detail.png" alt="Job Detail popup">

</td></tr>
<tr><td colspan="2">

**Themes** — btop++-style color themes, including ports of all of btop's own; shown here: Default, Nord, Dracula, Gruvbox Light, Tokyo Night, Solarized Dark
<img src="docs/screenshots/themes.png" alt="Theme gallery">

</td></tr>
</table>

## Features

### Dashboard
Live overview of your Salt Master with real-time sparkline panels:

- **Host Resources** — CPU load, memory usage, disk usage (cache + PKI)
- **Services** — All `salt-*.service` units auto-discovered via systemctl, with active state, memory, uptime, a rolling 24h restart/flap count (`Flp`), and a memory-growth trend arrow (`T`)
- **Minion Key Status** — Accepted / Denied / Unaccepted / Rejected key counts, plus new-pending-keys-per-minute
- **ZMQ** — established connection counts per port (pub/ret/api), plus a latency sparkline whose source is configurable (`zmq_latency_source`): a local agent round-trip (default), a real `test.ping` round-trip over the event bus, or Salt's own `master_stats` instrumentation. The graph uses a log scale up to `zmq_latency_ceiling_ms` — the latency that counts as "bad", drawn red at the top — defaulting per source (100 ms for the agent round-trip, 1 s for `test.ping`, 500 ms for `master_stats`)
- **Event Rate** — Events/sec sparkline with peak and total counters
- **Minion Activity** — Job return rate sparkline with last-active minion tracking
- **Level colors** — the CPU, Memory and ZMQ graphs are colored btop-style by height: each row by how high it sits on the graph's fixed scale, blending green → yellow → amber → red toward the top (Memory has its own calmer cyan → blue → amber → red gradient, as btop's memory meters do), so a bar's color shows how high it reached and the history never recolors. Their headline figures (and the Host Resources meters) use the same smooth gradient instead of hard thresholds, so a value hovering near a boundary doesn't flicker between colors
- **CPU / Memory** — utilization-history sparklines, toggle alongside the other three sparkline panels via `show_*_graph`; stacking order of all five panels is configurable via `graph_order` — in the Config Editor (`F3`, edited as a reorderable list: `↑↓` to select, `+`/`-` to move) or by hand in `salt-radar.toml`
- **Job Pulse** — Active jobs with progress bars, rolling success rate, stuck-job count, and p50/p95 completion latency. Shares a row with salt-api (side by side) when both are shown, since neither needs the terminal's full width; Job Pulse alone takes the full row when salt-api isn't shown
- **salt-api** — shown only when a `salt-api` process is detected; request rate, error rate, `POST /login` p50/p95 latency, in-flight/total request counts, and the most recent request per worker thread (path, status, timing). Needs `rest_cherrypy: {collect_stats: True}` in the master config, plus one of two ways to access the `/stats` endpoint that serves this data — salt-radar polls `GET /stats`, salt-api's own built-in in-memory request-stats endpoint (CherryPy's `cpstats`), which records the request line, client, status and timing entirely in process memory. This is a different mechanism from Salt's `log_level`/access logging: nothing is ever written to disk, and the request *line* it records never includes the body — so credentials sent in a POST body (e.g. `/login`) are never captured. Request rate is a real recent figure (computed by salt-radar from the delta between polls); error rate and login latency are also tracked by salt-radar itself, since cpstats only exposes a lifetime-average rate and a live per-thread snapshot, not cumulative error/latency counters of its own.
  - **`stats_auth_mode = "none"` (default)** — relies on `stats_disable_auth: True` in the master config, so `/stats` needs no credentials at all.
    - **Security tradeoff**: this setting removes Salt's normal auth-token requirement for the `/stats` endpoint specifically, so once enabled, *anyone who can reach salt-api's REST port* can poll `/stats` with no credentials — not just salt-radar. What leaks to that caller: request lines including query strings (so API endpoint paths/usage patterns), client IP:port, response status, and timing — but never request/response bodies or headers, so auth tokens and credentials (including `POST /login` payloads) are never exposed this way. The risk is proportional to how exposed salt-api's port already is: low if it's bound to localhost or a trusted management network, real if it's reachable more broadly. If that matters in your environment, use one of the two authenticated modes below instead, or firewall salt-api's port to trusted hosts.
  - **`stats_auth_mode = "plaintext"` or `"pillar"`** (Config Editor, `F3`, `[api]` section) — the agent logs into salt-api via eauth instead, and sends the resulting token as `X-Auth-Token` on every `/stats` poll. `stats_disable_auth` can stay unset. Both modes need `stats_auth_eauth` (the eauth backend, e.g. `pam`) set:
    - `plaintext` reads `stats_auth_username`/`stats_auth_password` straight out of `salt-radar.toml` — **marked insecure**: stored, and passed to the agent, as plain text.
    - `pillar` instead resolves username and password from two independent colon-delimited pillar paths, `stats_auth_username_pillar_path` and `stats_auth_password_pillar_path` — like two separate `pillar.get` calls (e.g. `salt_radar:api_user` / `salt_radar:api_password` for a pillar shaped `salt_radar: {api_user: ..., api_password: ...}`). Compiled **master-side, for this master's own local minion ID** — the same one `salt-call pillar.get` would resolve on this box (its cached `/etc/salt/minion_id`, same as `salt.config.get_id`), not something you configure — so it's naturally scoped to just this host: a `top.sls` entry keyed to this master's own minion ID (not `'*'`, which would hand the credential to every minion in the fleet) is all that's needed, and no other minion's pillar compilation ever matches it. No live round-trip either way — this reads local pillar SLS files the same way `salt-run pillar.show_pillar` would. Requires a minion co-located with the master (common, but not universal); resolved once and cached (recompiling pillar every poll would be wasteful), and a login that then fails re-resolves it once in case the values rotated in pillar since.
    - **Minimal eauth scope**: salt-api's `/stats` endpoint only checks that the login succeeded — it does **not** consult `external_auth`'s per-function ACL the way `/`, `/run`, `/minions`, etc. do. A dedicated eauth account with an **empty permission list** (e.g. `external_auth: {pam: {stats_monitor: []}}`) can therefore log in and read `/stats`, but that same token is rejected by every actual command-execution endpoint — about as close to a "stats-only" role as Salt's eauth model gets.
    - A stale/rejected token (server restart, `token_expire`) triggers one automatic re-login-and-retry per poll before the panel reports unreachable.
- Every panel above the sparklines — Host Resources, Services, Minion Key Status, Event Types, Job Pulse, salt-api — can be individually hidden via `show_host_resources`/`show_services`/`show_key_status`/`show_event_types`/`show_job_pulse`/`show_salt_api` in the Config Editor (`F3`), same on/off toggle as the sparkline panels' `show_*_graph`; hiding one of Services/Minion Key Status/Event Types reflows the remaining columns in that row to fill the freed width instead of leaving a gap

### Minions
- Scrollable minion list: ID, key status, OS, Salt version, plus optional Arch, Seen (time since last check-in) and Fail% (share of the minion's last 50 returns that failed, worst first when sorted) columns; sorting keeps timed-out minions in their own section
- Freshly-active marker (`●`) next to minions that checked in recently, threshold adapts to the fleet's own median check-in age
- Fleet overview panel: OS Distribution, Salt Version, and Architecture distribution bars
- Detail popup with key info and connection status
- Activity popup (`r`): the minion's recent jobs with function, args, result (OK / FAIL rc=N, or `2 ok 1 chg 1 fail` for state runs) and full JID, plus Job Cache entries that targeted it by name, glob or list; a summary line counts its last 50 returns and names the latest failure
- Filter by ID, OS, or version (`/` key) — scopes the fleet overview breakdown to the matching minions too, when "follow filter" is on (`f` to toggle, on by default; startup default configurable via `follow_filter` under `[minions]`)

### Events
- Live event stream from the Salt event bus (via ZMQ)
- Full date/time in local and UTC columns, plus an optional Minion column (the event's minion ID)
- Filter by tag, function, minion ID, or any JSON field — a comma-separated list of terms is OR'd together, and each term may be a glob (`web1,web2` or `web*`), mirroring Salt's own `-L`/glob targeting syntax. Matched terms are colored consistently (same term, same color, everywhere) from a small high-contrast palette, hashed from the term text
- Event detail popup:
  - `t` toggles between raw JSON and a **pretty view** for job-return events — the exact `salt-run jobs.lookup_jid --force-color --state-output=full --state-verbose=True` output (same subprocess call Job Detail uses), so formatting and colors are authoritative Salt output, not an approximation. Falls back to a client-side rendering only when no live fetch is possible (non-job-return events) or after a fetch failure/timeout
  - Copy to clipboard (`y` = tag/timestamp + body, `Y` = body only) matches whatever's currently on screen — pretty or raw
- Advanced **Event Sniffer** (`n` key) — filter events by minion ID/glob, same syntax as the main filter:
  - Sniffed minion IDs are colored wherever they occur in captured content (tag and JSON body), same hashed palette as the main filter
  - `h` sets an additional keyword search *within* what's already being sniffed — unlike the minion-ID coloring above, this one also narrows the displayed list to matches
  - Pause/resume live capture (`p`)
  - Navigate events with full pretty-printed JSON body
  - Open event detail from sniffer (`Enter`), return to sniffer on close
  - Copy captured (and currently filtered/shown) events to clipboard (`y`), or save them to file (`s`)

### Jobs
- Split panel: **Live Jobs** (left) from event bus and **Job Cache** (right) from Salt's master job cache
- Job cache supports pagination (`[`/`]` adjust limit, `,`/`.` page navigation) and server-side search
- **Args** column shows what a job was run with: for `state.apply`/`state.sls` the requested formula (e.g. `apache.install`), `highstate` when none was named, and a dimmed `(test)` marker for `test=True` dry runs; for other functions their arguments (e.g. `uptime` for `cmd.run`). The `/` filter matches it too
- Sortable and configurable columns (see [Columns and sorting](#columns-and-sorting)) — the Job Cache is sorted across the whole cache by the agent, not just the page on screen
- Default order (no sort column picked): Live Jobs run oldest to newest by JID, tail-style like Events — the cursor stays on the newest job as new ones arrive, until you move it (`End` resumes following); the Job Cache lists newest first
- Per-minion return navigation (j/k), scrollable output viewer (Enter)
- Copy return output to clipboard (y = Salt outputter format, Y = raw JSON), or save it to file (s/S, same two formats)
- Jobs with hundreds of returns aren't limited to one minion at a time: `a`/`A` save every return in the job to a single file (Salt outputter format / one JSON object keyed by minion id), from the return list
- Job detail popup shows correct OK/FAIL status derived from return values
- Always shows full per-state detail (`--state-output=full --state-verbose=True`), regardless of the master's own `state_output`/`state_verbose` defaults — opening the popup is an explicit request to see everything about that one job, even a "terse"/"changes" master would otherwise hide unchanged/no-op states here

### Pulse
- Per-minion return activity at a glance: every minion as a dot in a wrapped grid, color- *and* shape-tiered by recency (`●` bright green flash → `•` green → `·` orange → `·` grey → `·` red for timed out) so adjacent tiers stay distinguishable even at a glance
- Scales from a handful of minions to 4,000+ without layout changes
- Detail panel on the side: key status, connection status, OS, Salt version, last return time

### Alerts
14 rules, evaluated every tick and de-duplicated (repeat firings bump a counter instead of spamming new entries):

- **Stale minion** — not seen within `stale_minion_secs`
- **Pending key** — pending longer than `pending_key_secs`
- **Service down** — a discovered `salt-*.service` unit is inactive
- **Disk usage high** — cache or PKI disk over `disk_usage_pct`
- **Legacy protocol** — minions still using Salt's legacy transport
- **Salt warning** — `salt/warning/*` events on the bus
- **Auth failure** — failed authentication on the ZMQ bus
- **Stuck job** — dispatched but not fully returned within `job_straggler_secs`
- **Reauth storm** — auth rate and worker-pool saturation elevated together (`auth_storm_rate_per_sec`, `worker_saturation_pct`)
- **Pending key churn** — new pending keys arriving faster than `pending_key_churn_per_min`
- **Service flapping** — a unit restarted more than `service_flap_threshold_24h` times in 24h
- **Service memory growth** — sustained upward memory slope past `service_mem_growth_mb_per_hour`, independent of flapping
- **Service memory high** — a unit's current RSS crosses an absolute (`service_mem_abs_threshold_mb`) or percent-of-total-memory (`service_mem_pct_threshold`) threshold, independent of growth rate; salt-master/minion/api/syndic can each override these globals (e.g. `service_mem_abs_threshold_master_mb`)
- **API auth latency** — salt-api's `POST /login` p95 processing time crosses `api_auth_latency_warn_secs`, a common early symptom of PAM/LDAP/eauth backend trouble (needs the salt-api panel's `collect_stats` setup — see Dashboard above)

All thresholds are configurable via `salt-radar.toml` or the in-app Config Editor (`F3`).

### API
Shown only once salt-api's `/stats` is reachable (see Dashboard above for the `collect_stats` setup and the `stats_disable_auth`/`stats_auth_mode` access options):

- Live request log — age, method, client, path, status, timing — built by salt-radar itself from cpstats' per-thread snapshots (filterable by `/`); `Enter` opens a detail popup with the full request line (query string included), client, status, and processing time
- Per-endpoint breakdown — request count, avg/p95 latency, error rate, grouped by method+path
- Best-effort, not a complete record: cpstats only ever exposes the *latest* request per CherryPy worker thread, so a thread that completes more than one request between two polls only shows the last one
- Bounded two ways, both configurable under `[api]` (`log_cap`, default 1000; `log_max_age_secs`, default 3600): capped at `log_cap` entries, and entries older than `log_max_age_secs` are pruned every poll regardless of how full the log is. The salt-api panel's own error-rate/login-latency rolling window (`stats_window`, default 200 requests) lives in the same section.
- Only what cpstats records is ever visible: no request/response body, no headers — so no credentials, payload content, or which minion/job/user was involved
- salt-radar's own `GET /stats` poll (the request that populates this log) is filtered out by default — `hide_stats_polls` (`[api]`, default `true`) — since at the Dashboard refresh interval it would otherwise drown out real traffic with one entry per tick. Set to `false`, or press `x` on the API tab to toggle it live — the filter applies at display time, so it hides/shows already-captured entries retroactively, not just ones captured from that point on. `/stats` requests are still always captured and still count against `log_cap`/`log_max_age_secs` even while hidden — this only controls what's shown.

### Admin Mode
- Toggle with `a` (danger-zone confirmation dialog required)
- **Fleet scan** (`s`): runs `grains.items` on all minions via Salt LocalClient, populates OS/Salt Version/Arch data; it finishes as soon as every minion has answered, and marks any still silent after 60s as timed out

### Columns and sorting
- The Minions, Events, Jobs (both tables), Alerts and API request log tables let you choose which columns are shown and in what order: `v` opens a picker for the focused table (`+`/`-` move a column, `Space` shows/hides it, `Enter` saves), also editable in the Config Editor — `minion_columns`, `event_columns`, `live_columns`/`cache_columns`, `alert_columns` and `request_columns` in their sections of `salt-radar.toml`
- Every table except Events (a live tail that follows the newest event) sorts by any column: click its header, or `o` to step through columns and `O` to reverse — ascending, descending, then back to the default order (Minions by ID, Live Jobs oldest-to-newest and Job Cache newest-first by JID, Alerts and the API log in arrival order). Sorting lasts for the session
- Optional columns beyond the defaults: Live Jobs Started, Minions Arch/Seen/Fail%, Events Minion, Alerts Since/Count
- When a panel is too narrow for every column, the text columns shrink together instead of cutting off the rightmost ones

### Config Editor
- `F3` opens an in-app editor for every setting in `salt-radar.toml`, grouped by section
- Free-text fields edit inline; fields with a fixed set of valid values (e.g. `zmq_latency_source`) open an inline picker — arrow keys to move, `Enter` to confirm
- Settings that must point at something existing — `agent_path`, `python_bin`, `master_config` (files) and `save_dir` (a directory) — open a file browser instead of free text, starting at the current value (or the nearest folder that exists): `↑↓` to move, `Enter`/`→` to open a folder or pick a file, `←`/`Backspace` up a level, `.` to show hidden files, and for `save_dir` a "select this directory" entry; type the start of a name to jump to it, and `/` lets you type a path by hand. A path that doesn't exist is flagged `✗ not found` in the editor, so typos stand out
- `color_theme` opens a picker of every theme found (see [Themes](#themes)) and previews each one live as you move through the list; `Esc`, or closing without saving, puts the saved theme back
- `graph_order` opens as a reorderable list instead of free text — `↑↓` selects an item, `+`/`-` moves it up/down, `Enter` confirms, `Esc` reverts — since hand-typing a comma-separated list makes it easy to typo or drop a panel
- The `*_columns` settings open the same way, plus `Space` to show/hide a column (hidden columns are listed unchecked below the shown ones)
- `Save & Restart` (or `s` from anywhere in the editor) writes the TOML file and applies the new config immediately, no process restart needed. Only settings that differ from the defaults are written, so everything you haven't changed keeps following the defaults of future versions (a file saved by an older version, which held every setting, slims down on its next save). Confirming a field with `Enter` only stages it — the title shows `● unsaved changes` until saved, and closing with unsaved changes asks first (`Esc`/`q` again discards)

### Clipboard and save-to-file

Every popup with copy-to-clipboard (`y` / `Y`) also supports saving the same content to a file (`s` / `S`) — a small dialog opens on top of the popup, proposing a filename from what's being saved; edit it, `Enter` saves (warning first if the file already exists), `Esc` cancels. JSON saves (`S`, and `A` for all of a job's returns) get a `.json` name, text saves `.txt`. Default save directory is `~` (the launching user's home), configurable as `save_dir` under `[export]` (or live, in the Config Editor).

Copy-to-clipboard (`y` / `Y`) uses two methods:

1. **tmux load-buffer** — pushes text into the tmux paste buffer via OSC 52 escape sequence. MobaXterm (or any terminal emulator that handles OSC 52) captures this and places it on the system clipboard. Works automatically when running inside tmux. Requires `set -g set-clipboard on` in `~/.tmux.conf` to push to the system clipboard (without it, content is only available via `Ctrl+b ]` inside tmux).

2. **xclip** — pushes text to the X11 clipboard via `su` as the user that owns the X11 socket. The code auto-resolves DISPLAY and XAUTHORITY by checking the process environment first, then falling back to the tmux session's global environment, and finally trying the detected user's default `~/.Xauthority` path. This means clipboard works even when salt-radar is launched under `sudo`, with no `sudoers` configuration required.

If DISPLAY and XAUTHORITY can't be resolved, a transient yellow hint is shown in the status bar explaining that X11 clipboard is unavailable. Run with `ssh -X` or `ssh -Y` to enable X11 forwarding.

### Everywhere
- **Color themes** — btop++-style `.theme` files (see below); the screen behind any popup is dimmed so the popup stands out — how strongly is `popup_dim_pct` in `[ui]` (default 70%, `0` turns it off)
- **Clipboard integration** — tmux load-buffer + xclip via X11 forwarding. DISPLAY and XAUTHORITY are auto-resolved from the process environment or the tmux session's global environment — no manual configuration needed even when running under `sudo`
- Filter-as-you-type with `/` on all list tabs — each tab keeps its own filter independently, comma-separated terms (each optionally a glob) are OR'd together, and matched terms are consistently colored
- Keyboard-driven navigation with `?` for help

## Installation

### Requirements
- A Salt Master (reads `/etc/salt/master`, connects to the Salt event bus)
- Python 3 with the `salt` package installed
- **Optional — clipboard:** `xclip` (for X11 clipboard integration). Not required when using tmux with `set -g set-clipboard on` and a terminal that supports OSC 52 (MobaXterm, iTerm2, etc.)

### From a release package

Each [GitHub release](https://github.com/naqoyqatsi83/salt-radar/releases) has a `.deb`, an `.rpm` and a `.tar.gz`, all with the same statically linked binary, so it runs on any x86_64 master regardless of its glibc version:

```bash
sudo apt install ./salt-radar_<version>_amd64.deb      # Debian/Ubuntu
sudo dnf install ./salt-radar-<version>-1.x86_64.rpm   # RHEL/Rocky/Fedora/SUSE (or rpm -i)
```

The packages install `/usr/bin/salt-radar`, the Python agent at `/usr/share/salt-radar/salt_radar_agent.py`, and the bundled themes in `/usr/share/salt-radar/themes/` — the built-in config defaults already point at these, so no config file is needed to start. The `.tar.gz` holds the same three pieces for a manual install; put its `themes/` files in `/usr/share/salt-radar/themes/` or `~/.config/salt-radar/themes/`.

### From source

```bash
git clone https://github.com/naqoyqatsi83/salt-radar.git
cd salt-radar
cargo build --release
```

The binary will be at `target/release/salt-radar`.

### Configuration

Copy the example config and adjust as needed:

```bash
cp salt-radar.toml.example salt-radar.toml
```

All settings are optional and can also be edited live from inside the app (`F3`), which writes the file back on save — the Config Editor's own bottom border shows exactly which file it's writing to. salt-radar looks for `./salt-radar.toml` in the current directory first (so a per-directory config still works, e.g. if you copied the example there); if that's not present, it falls back to `$XDG_CONFIG_HOME/salt-radar/salt-radar.toml` (usually `~/.config/salt-radar/salt-radar.toml`), creating the directory on first save. See [`salt-radar.toml.example`](salt-radar.toml.example) for the complete, commented list of every setting (connection paths, refresh intervals, event buffer sizes, all alert thresholds including per-unit memory overrides, and the Dashboard's `zmq_latency_source`/`show_*_graph`/`graph_order` settings) — a representative slice:

```toml
[salt]
# master_config = "/etc/salt/master"

[refresh]
dashboard_secs = 2

[alerts]
stale_minion_secs = 3600
disk_usage_pct = 85.0
service_mem_abs_threshold_mb = 2048.0

[dashboard]
zmq_latency_source = "agent_ipc"  # or "job_ping", "master_stats"
```

### Themes

Colors come from a theme, picked with `color_theme` in the `[ui]` section (or in the Config Editor, which previews each theme live as you scroll the list, like btop's options menu). The built-in theme is `Default`; the package also ships ports of all of btop++'s bundled themes (Nord, Dracula, Gruvbox, Tokyo Night, Solarized, Everforest, Kanagawa, … and `btop` itself) in `/usr/share/salt-radar/themes/`, and your own `*.theme` files go in `~/.config/salt-radar/themes/` (the user's copy wins on a name clash), and `color_theme` can also be a path to a theme file. The format is the same as btop++'s — `theme[key]="#RRGGBB"` lines, and any key a theme leaves out keeps its Default color. [`salt-radar.theme.example`](salt-radar.theme.example) is the Default theme with every key documented; copy it to `~/.config/salt-radar/themes/<name>.theme` to start your own. Set `theme_background = false` to keep your terminal's own (e.g. transparent) background instead of the theme's.

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `q` | Quit |
| `Tab` | Switch tab |
| `1`–`7` | Jump to tab (Dashboard/Minions/Events/Jobs/Alerts/API/Pulse) |
| `↑`/`↓` | Scroll / navigate |
| `PgUp`/`PgDn` | Page scroll |
| `Home`/`End` | First/last row |
| `Enter` | Open detail popup |
| `Esc` | Close popup / help |
| `/` | Filter (current tab) |
| `a` | Toggle admin mode |
| `?` / `F1` | Help overlay; `Tab` switches to What's new (the changelog) |
| `F2` | Toggle mouse mode |
| `F3` | Open Config Editor |
| `+`/`-` | Adjust refresh interval |
| `m` | Toggle sparkline style (Dashboard) |
| `F4`–`F8` | Toggle CPU/Memory/ZMQ/Events/Minion-activity graphs (Dashboard) |

Most of these are also shown live, btop-style, in the border of the panel they affect (e.g. the title frame's bottom border shows `-2000ms+` for the current refresh interval) — the table above is the full reference.

### Per-tab

| Tab | Key | Action |
|-----|-----|--------|
| Events | `n` | Launch event sniffer |
| Events | `s` / `S` | Save the filtered event list to a file (text / NDJSON) |
| Minions | `r` | Minion event/return history popup |
| Minions | `s` | Fleet scan (admin mode only) |
| Minions | `x` | Hide/show non-responsive minions |
| Minions | `z` | Collapse/expand timed-out minions into one row (when not hidden) |
| Minions | `f` | Toggle whether the Fleet Overview breakdown follows the active list filter |
| Minions, Jobs, Alerts, API | `o` / `O` | Sort by the next column / reverse the sort (also: click a column header) |
| Minions, Events, Jobs, Alerts, API | `v` | Choose which columns the focused table shows, and their order |
| Jobs (Live) | `x` | Show only failed jobs / all jobs |
| Jobs (Cache) | `/` | Filter cache entries (server-side) |
| Jobs (Cache) | `[`/`]` | Decrease/increase page limit by 500 |
| Jobs (Cache) | `,`/`.` | Previous/next page |
| Jobs (Cache) | `r` | Refresh job cache |
| Jobs (Cache) | `←`/`→` | Switch focus between Live Jobs and Cache panels |
| Pulse | `↑`/`↓`/`←`/`→` | Navigate the minion grid |
| API | `x` | Hide/show salt-radar's own `GET /stats` poll in the request log (hidden by default — see `hide_stats_polls`) |

### Config Editor

| Key | Action |
|-----|--------|
| `↑`/`↓` | Move between fields |
| `PgUp`/`PgDn` | Page through fields |
| `Home`/`End` | First/last field |
| `Enter` | Edit field / open picker (fixed-option fields) or file browser (path fields) / confirm picker selection |
| `Esc` | Cancel edit or close the picker without changing the value |
| `s` | Save all changes and apply them |
| `←` / `Backspace` | Up one folder (file browser) |
| `.` | Show/hide hidden files (file browser; while not typing a name) |
| `/` | Type the path by hand (file browser) |
| letters, digits | Jump to the first entry starting with what you type (file browser) |

### Popups

| Key | Action |
|-----|--------|
| `y` | Copy to clipboard |
| `Y` | Copy raw JSON (job detail) |
| `s` | Save to file |
| `S` | Save raw JSON to file (job detail; NDJSON in the sniffer) |
| `a` | Save every return in the job to one file (job detail) |
| `A` | Save every return as one JSON object (job detail) |
| `j`/`k` | Next/previous return (job detail) |
| `PgUp`/`PgDn` | Jump 10 returns |
| `Home`/`End` | First/last return (job detail) |
| `Enter` | Enter scroll mode (event/job detail) |
| `/` | Search the return output (job detail, output pane) or the help pages, matches highlighted |
| `n`/`N` | Next/previous search match, marked in place (job detail, also from the returns list; help) |
| `f`/`F` | Next/previous failed state, marked in the output (job detail; also from the returns list) |
| `Esc` | Exit scroll mode |
| `Home`/`End` | Jump to start/end of output (scroll mode) |
| `t` | Toggle raw JSON / pretty view (event detail) |
| `h` | Edit keyword filter (sniffer) |
| `p` | Pause/resume (sniffer) |
| `g` | Fetch grains for this minion (minion detail, admin mode only) |

## How It Works

```
┌────────────────────────────────────────────────────────────────────┐
│                       salt-radar (Rust TUI)                        │
├─────────┬─────────┬─────────┬─────────┬─────────┬─────────┬────────┤
│Dashboard│ Minions │  Events │   Jobs  │  Alerts │   API   │ Pulse  │
├─────────┴─────────┴─────────┴─────────┴─────────┴─────────┴────────┤
│ detail popups · Config Editor · themes · alert engine              │
│ collectors: /proc · systemctl · /proc/net/tcp · PKI key dir        │
└───────────────┬─────────────────────────────────┴──────────────────┘
                │ commands                        ▲ results, events
                │ (stdin)                         │ (stdout)
                │                                 │
┌───────────────▼─────────────────────────────────┴──────────────────┐
│              Python agent (runs on Salt's own Python)              │
│ event bus subscriber · LocalClient / runners · fleet scan          │
│ job cache reader · jobs.lookup_jid · salt-api /stats poller        │
└──────────────────────────────────┬─────────────────────────────────┘
                                   │
┌──────────────────────────────────▼─────────────────────────────────┐
│        Salt Master: event bus · job cache · PKI · salt-api         │
└────────────────────────────────────────────────────────────────────┘
```

The Rust TUI spawns a Python agent subprocess and talks to it in NDJSON: commands go to the agent's stdin, results and Salt events come back on its stdout. The agent runs on Salt's own Python, subscribes to the master's local event bus, and calls Salt's LocalClient and runners (job lookups, the job cache, fleet scans) — plus salt-api's `/stats` endpoint for the API tab. Meanwhile, the Rust side collects host data itself: processes and memory from `/proc`, services from `systemctl`, ZMQ connection counts from `/proc/net/tcp`, and minion keys straight from the PKI directory.

All data stays local — no external dependencies, no database, no HTTP server.

### A note on the two memory numbers on the Dashboard

The **Services** panel shows two different memory readings that measure different
things and are not expected to match:

- **Per-role breakdown** (`Master x4: 212MB`, `Worker x5: 299MB`, etc.) — the sum
  of each individual process's own RSS (`/proc/<pid>/stat`), independently, across
  every process matching that role. Since Salt's Worker/ReqServer/EventPublisher
  children are forked from the master and share most of their loaded code/library
  pages with it via copy-on-write, summing their RSS independently counts those
  shared pages once per process — so this number is a reasonable *upper bound*,
  not the service's true footprint.
- **`Unit` table memory** (`salt-master.service 51 MB`) — the systemd unit's
  cgroup memory accounting (`MemoryCurrent`), which counts each physical page
  once no matter how many of the service's processes share it, and covers the
  whole process tree (main daemon plus all forked children). This is the more
  accurate figure for "how much RAM does this service actually cost."

If cgroup memory accounting isn't available for a unit, the `Unit` table falls
back to the main process's own `VmRSS`, which — like the per-role breakdown —
only reflects that one process, not its forked children.

## Development

```bash
# Build
cargo build

# Run with live config
cargo run

# Release build
cargo build --release
```

### Project Structure

```
src/
├── main.rs              # Terminal setup, event loop, tick scheduling
├── app/                 # Application state and behavior
│   ├── mod.rs           # App state, tick/update loop, filtering and sorting, job tracking
│   ├── keys.rs          # Keyboard and mouse input for tabs, lists and every popup
│   ├── messages.rs      # Agent results/events in, agent commands out, fleet scan
│   ├── config_editor.rs # Config Editor: field model, path browser, saving, theme preview
│   └── clipboard.rs     # Clipboard (tmux/OSC 52, X11 via xclip) and save-to-file
├── types.rs             # Data types (minion, event, job, alert, etc.)
├── config.rs            # TOML config loading, theme discovery and parsing
├── agent.rs             # Python agent subprocess management
├── alerts/              # Alert engine (14 rules: staleness, keys, services, memory, disk, jobs, auth, api)
├── collectors/          # OS-level data collection
│   ├── keys.rs          # PKI key file parsing
│   ├── processes.rs     # /proc scanning for salt processes
│   ├── services.rs      # systemctl service discovery
│   ├── connections.rs   # /proc/net/tcp for ZMQ connections
│   └── system.rs        # System load, memory, disk usage
└── ui/                  # Terminal UI
    ├── mod.rs           # Master draw, theme, title/tab frame, key hints, help
    ├── dashboard.rs     # Dashboard tab
    ├── minions.rs       # Minions tab
    ├── events.rs        # Events tab
    ├── jobs.rs          # Jobs tab
    ├── alerts_view.rs   # Alerts tab
    ├── api_view.rs      # API tab
    ├── pulse.rs         # Pulse tab
    ├── table.rs         # Shared table layout: column widths, headers, sort markers
    ├── minion_events.rs # Per-minion event/return history popup
    ├── config_editor.rs # In-app config editor (F3)
    └── detail_popup.rs  # All other popup windows (minion, event, job, sniffer, etc.)
agent/
└── salt_radar_agent.py  # Python agent: Salt client, event subscriber, fleet scan
themes/                  # Bundled color themes (ports of btop++'s), installed to /usr/share/salt-radar/themes
salt-radar.theme.example # The Default theme with every key documented
```

## Acknowledgements

salt-radar stands on the shoulders of other people's work:

- **[btop++](https://github.com/aristocratos/btop)** by Jakob P. Liljenberg ([aristocratos](https://github.com/aristocratos)) — a huge thank-you for the inspiration and for such great work. A lot of salt-radar's look and feel comes straight from btop: shortcut hints living in the panel borders, braille sparklines, graphs colored by level, and the whole theme system — the `.theme` file format is btop's, and the bundled themes are ports of btop's own (Apache-2.0). If you like salt-radar, go give btop a star.
- **The theme authors** — every bundled theme is a port of a btop theme and keeps its original author's credit at the top of the file, and most of them carry a well-known color scheme by its own designers (Nord, Dracula, Gruvbox, Solarized, Tokyo Night, Everforest, Kanagawa, Flexoki and more).
- **[psnet](https://github.com/psmux/psnet)** — salt-radar's original dark palette, its ratatui TUI style, and its `/proc` / `/proc/net/tcp` collection approach were modeled on psnet.
- **[ratatui](https://github.com/ratatui/ratatui)** and **[crossterm](https://github.com/crossterm-rs/crossterm)**, which do the heavy lifting of drawing to the terminal, plus serde, serde_json, toml, chrono, base64 and libc.
- **[Salt](https://github.com/saltstack/salt)** itself — the Python agent uses nothing beyond Python's standard library and Salt's own client APIs.

## License

MIT — see [LICENSE](LICENSE).

salt-radar is not affiliated with or endorsed by SaltStack / VMware / Broadcom.
