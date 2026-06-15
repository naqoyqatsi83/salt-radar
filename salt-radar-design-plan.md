# salt-radar — Design Plan

This document outlines the original design decisions and implementation roadmap for the Salt Master monitoring TUI application.

## Overview

A non-intrusive, read-only TUI for monitoring a Salt Master. Designed with focus on reliability, minimal resource usage, and passive observation rather than active probing.

---

## Architecture

### Core Components

- **Agent**: Rust-side agent communicating with Salt Master via NDJSON over stdin/stdout
- **Collector Pattern**: Separate collectors for different data sources (keys, processes, services, system info, events, jobs)
- **TUI Framework**: ratatui (v0.29) + crossterm (v0.28)
- **Event Bus**: Salt events streamed from master for real-time updates
- **Configuration**: TOML-based (`salt-radar.toml`) with sensible defaults

### Data Flow

1. **Initial setup**:
   - Load config from `salt-radar.toml` or defaults
   - Spawn agent process
   - Subscribe to Salt events
   - Start collectors (run on intervals)

2. **Runtime updates**:
   - Collect system info periodically (every 2s)
   - Collect keys/minions periodically (every 5s)
   - Collect processes periodically (every 2s)
   - Collect services periodically (every 2s)
   - Collect ZMQ latency periodically (every 2s)
   - Update alerts based on collected data

3. **Rendering**:
   - 2s frame rate with 60s event history retained
   - Efficient layout with minimal redraws
   - Real-time updates reflected immediately

---

## Core Features

### Dashboard Tab
- **Radar spinner**: Visual indicator showing activity
- **Host uptime**: Time since last system boot
- **Processes & Services**: Current Salt processes and their health
- **Host resources**: CPU load, memory usage
- **ZMQ latency**: Round-trip time to master (sparkline)
- **Event rate**: Events per second (sparkline)
- **Minion activity**: Checkin activity over last 60s (sparkline + breakdown)

### Minions Tab
- **Key status**: All minions with key status and metadata
- **Grains**: Machine information for each minion
- **Connection status**: Live connection status
- **Search/filter**: Filter minions by ID, key status, or metadata

### Events Tab
- **Live event stream**: Real-time Salt events with filtering
- **Tag filtering**: Filter by event tag patterns
- **Minion filtering**: Show only events from specific minions
- **Data navigation**: View event details, copy to clipboard

### Jobs Tab
- **Job tracking**: Active and recent jobs with JID, function, target
- **Per-minion breakdown**: Jobs targeting multiple minions
- **Event-driven updates**: Jobs appear as they are created
- **Detail view**: Full job return data per minion (scrollable)

### Alerts Tab
- **Staleness alerts**: Minions not seen within timeout
- **Key alerts**: Pending/rejected/denied keys
- **Service alerts**: Failed Salt services
- **Disk usage alerts**: High usage on cache/pki directories
- **Config-driven**: Alert thresholds from TOML

---

## Tabs & Navigation

- **Tab switching**: `←/→` or `1-5`
- **Help**: `?` or `q`
- **Exit**: `q`

---

## Panels & Layout

### Dashboard
- **Header**: `salt-radar v0.1.0 ● ● Salt <version> | keys: A1 P0 | radar <uptime> | host <uptime>`
- **Main layout**:
  - Uptime gauge (left)
  - Processes/Services (middle)
  - Host resources (right)
  - Event rate sparkline (bottom)
  - ZMQ latency sparkline (bottom)

### Minions
- **Header**: `Minions (A1 active, P0 pending, R0 rejected)`
- **List**: Minion ID, key status, OS, salt version, last seen, CPU arch

### Events
- **Header**: `Events (2/s | all tags) | <minion filter>`
- **List**: Timestamp, tag, minion_id, data preview

### Jobs
- **Header**: `Jobs | <filter>`
- **List**: JID, function, target, returned/targeted, age

### Alerts
- **Header**: `Alerts (3 active) | <filter>`
- **List**: Alert key, severity, description, active since, count

---

## Data Sources

### System Collectors
- **System info**: `/proc/stat`, `/proc/meminfo`, disk usage
- **Salt processes**: `/proc/*/cmdline`, `/proc/*/stat`
- **Host uptime**: `/proc/uptime`

### Salt Collectors
- **Keys**: `salt-run manage.keys` via agent
- **Minion cache**: `salt-cache-*` file paths
- **Config snapshot**: `salt-run config.values` via agent
- **ZMQ ping**: Custom `zmq_ping` command via agent

### Event Processing
- **Real-time**: Agent streams Salt events via NDJSON
- **Event parsing**: Deserialize JSON events
- **Tag filtering**: Filter by event tag patterns
- **Minion extraction**: Extract minion ID from events

### Job Tracking
- **Event-driven**: Jobs created via `salt/job/*/start` events
- **Return tracking**: Jobs completed via `salt/job/*/ret/*` events
- **Detail lookup**: Use `salt-run jobs.lookup_jid` when needed for full returns

---

## Rendering Details

### Dashboard Header
```rust
let header = Line::from(vec![
    Span::styled("salt-radar v0.1.0 ● ● ", Style::default().fg(color::PRIMARY)),
    Span::styled("Salt ", Style::default().fg(color::TEXT)),
    Span::styled(version, Style::default().fg(color::ACCENT)),
    Span::styled(" | keys: A1 P0 | ", Style::default().fg(color::TEXT_DIM)),
    Span::styled("radar ", Style::default().fg(color::TEXT)),
    Span::styled(format!("{}", app.session_uptime), Style::default().fg(color::CYAN)),
    Span::styled(" | host ", Style::default().fg(color::TEXT)),
    Span::styled(format!("{}", host_uptime), Style::default().fg(color::YELLOW)),
]);
```

### Processes & Services Panel
```rust
let mut service_lines: Vec<Line<'static>> = vec![];

// Display header for service columns
service_lines.push(Line::from(vec![
    Span::styled("  Unit             ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
    Span::styled("Status ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
    Span::styled("State                   ", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
    Span::styled("Memory", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
    Span::styled("Uptime", Style::default().fg(color::TEXT_LABEL).add_modifier(Modifier::BOLD)),
]));

for service in &app.services {
    // Service rendering logic...
}
```

### Minion Activity Panel
```rust
// Show sparkline + counts
let sparkline = Line::from(vec![
    Span::styled("checkins/s ", Style::default().fg(color::TEXT_LABEL)),
    // Draw sparkline from activity_matrix...
    Span::styled("    3792 active   16 quiet >5m   4 silent", Style::default().fg(color::TEXT)),
]);
```

---

## Error Handling

### Collector Failures
- **Graceful degradation**: If collector fails, show error state but continue
- **Retry logic**: Exponential backoff for transient failures
- **Fallback data**: Use last successful data when unavailable

### Agent Communication
- **Connection health**: Track agent connection state
- **Restart capability**: Restart agent if it exits unexpectedly
- **Cleanup**: Ensure child processes are terminated on exit

### Filesystem Access
- **Permission handling**: Handle permission errors gracefully
- **Missing files**: Show appropriate error messages

---

## Configuration

### salt-radar.toml Schema
```toml
[app]
refresh_rate = 2000  # milliseconds

[collectors.keys]
interval = 5000

[collectors.minions]
interval = 5000

[collectors.processes]
interval = 2000

[collectors.services]
interval = 2000

[collectors.system]
interval = 2000

[collectors.zmq]
interval = 2000

[alerts.staleness]
active_secs = 300
quiet_secs = 1800

[alerts.keys]
notify_pending = true
notify_rejected = true
notify_denied = true

[alerts.services]
notify_failed = true

[alerts.disk]
cache_usage_pct = 90
pki_usage_pct = 90
```

---

## Performance Considerations

### Memory Management
- **Efficient data structures**: Use `VecDeque` for circular buffers
- **Clone-on-write**: Share data where possible to avoid duplication
- **Explicit drops**: Drop unused data promptly

### CPU Usage
- **Selective polling**: Only poll collectors that need updates
- **Batch operations**: Group related operations where possible
- **Async where appropriate**: Use async I/O for blocking operations

### UI Performance
- **Minimal redraws**: Only update components that changed
- **Efficient rendering**: Use ratatui's efficient rendering model
- **Bounded buffers**: Limit history size to control memory usage

---

## Testing

### Unit Tests
- **Collectors**: Test parsing of system files and Salt output
- **UI components**: Test line drawing and formatting functions
- **Error handling**: Test failure cases and recovery

### Integration Tests
- **Full workflow**: Test complete agent-collector-TUI integration
- **Real Salt Master**: Test against a real Salt Master (optional)
- **Configuration loading**: Test various config combinations

---

## Future Enhancements

### Phase 1 (Completed)
- Basic TUI skeleton
- Core dashboard with all panels
- Event and job tracking
- Alert system

### Phase 2 (Planned)
- Dynamic service discovery (in progress)
- Minion activity sparkline (in progress)
- Host uptime display
- Enhanced filtering/search

### Phase 3 (Future)
- Fleet version drift panel (deferred)
- Per-minion key status filtering
- Advanced event filtering
- Historical analysis
- Configurable alert thresholds

---

## 1. Goals & non-negotiables

- **Read-only.** salt-radar never fires events, never accepts/rejects/deletes
  keys, never calls `cmd.run` / `state.apply` / anything that targets
  minions. Every data source listed below is either a filesystem read, a
  `/proc` read, a `systemctl` status query, or a *subscription* to the
  master's event bus (subscribing does not affect publishers).
- **Single Rust binary** (`salt-radar`) for the UI + OS-level data, matching the
  psnet codebase style (ratatui + crossterm, dark theme, tab navigation).
- **One bundled Python helper** (`salt-radar-agent`) for everything that needs
  Salt's own libraries (event bus, job cache lookups, effective config,
  minion cache, version). It must run with the same Python environment Salt
  itself uses (`import salt` must succeed), which in practice means it runs
  on the master host as root, using the system/venv Python that has the
  `salt` package installed.
- **Root required**, same as psnet — needed to read
  `/etc/salt/pki/master/*`, `/var/cache/salt/master/*`, and to open the
  master's IPC sockets.

---

## 2. Architecture

```
┌─────────────────────────┐        NDJSON over stdin/stdout       ┌──────────────────────────┐
│   salt-radar (Rust binary)    │ ─────────────────────────────────────▶│  salt-radar-agent (Python)     │
│   ratatui + crossterm    │◀───────────────────────────────────── │  imports salt.*           │
│                           │                                       │                           │
│  - tab navigation/UI      │        commands / streamed events     │  - salt.config            │
│  - /proc, systemctl,      │                                       │  - salt.utils.event       │
│    pki dir collectors     │                                       │  - salt.cache             │
│  - alert rule engine      │                                       │  - salt.runner            │
└─────────────────────────┘                                       └──────────────────────────┘
```

### 2.1 Why split this way

- Everything Rust touches directly (`/proc/net/tcp`, `/proc/<pid>/...`,
  `systemctl show`, directory listings under `/etc/salt/pki/master/`) is
  stable across Salt versions — it's just OS state.
- Everything that depends on *how Salt itself* serializes/stores data
  (event bus wire format, job cache layout, minion cache backend, effective
  config merging with includes/defaults) is delegated to a script that uses
  Salt's own code to do that work. This means salt-radar doesn't need to track
  Salt's internal protocol changes — Salt does that for itself.
- The agent is a subprocess, not a library linked into Rust, so there's no
  need for PyO3/FFI complexity — just a process boundary and a JSON
  line-protocol, which is also easy to debug by running the agent standalone
  and watching it print JSON.

### 2.2 Process model

`salt-radar` spawns `salt-radar-agent` once at startup (e.g.
`python3 /usr/share/salt-radar/salt_radar_agent.py`) and keeps it running for the
whole session:

- **stdin**: salt-radar writes one-line JSON commands.
- **stdout**: agent writes one-line JSON responses/streamed messages.
- **stderr**: agent's own diagnostics/errors (logged by salt-radar, shown in a
  status line, never crashes the UI).

If the agent dies, salt-radar shows a persistent "agent disconnected — Salt data
unavailable" banner on affected panels, keeps the OS-level panels (Dashboard
process/service/connection data) working, and attempts periodic restart.

### 2.3 Agent protocol (NDJSON)

Requests (salt-radar → agent), one JSON object per line:

```json
{"id": 1, "cmd": "version"}
{"id": 2, "cmd": "config_snapshot"}
{"id": 3, "cmd": "minion_cache_list"}
{"id": 4, "cmd": "jid_lookup", "jid": "20260613120501123456"}
{"id": 5, "cmd": "subscribe_events"}
{"id": 6, "cmd": "unsubscribe_events"}
```

Responses (agent → salt-radar), one JSON object per line:

```json
{"id": 1, "type": "result", "cmd": "version", "ok": true, "data": {"salt_version": "3007.1", "python_version": "3.11.x"}}
{"id": 4, "type": "result", "cmd": "jid_lookup", "ok": true, "data": {"found": true, "fun": "state.apply", "tgt": "web*", "returns": {"web01": {...}, "web02": {...}}}}
{"type": "event", "tag": "salt/job/20260613.../new", "data": {...}, "ts": 1781234567.123}
{"type": "error", "id": 5, "message": "could not open event bus: <details>"}
```

- `subscribe_events` starts a background thread in the agent that calls
  `iter_events()` and writes one `{"type": "event", ...}` line per event,
  indefinitely, until `unsubscribe_events` or the agent exits.
- Responses to one-shot commands (`version`, `config_snapshot`,
  `minion_cache_list`, `jid_lookup`) are synchronous request/response,
  matched by `id`.

---

## 3. salt-radar-agent (Python) — commands and Salt APIs

> **Verification needed (Claude Code, do this first):** locate the installed
> Salt package (`python3 -c "import salt, os; print(os.path.dirname(salt.__file__))"`)
> and read `salt/utils/event.py`, `salt/cache/__init__.py`,
> `salt/config/__init__.py`, and `salt/runner.py` for the *actual* function
> signatures on this system before writing the agent. The sketches below are
> the right APIs to use, but exact kwargs/return shapes vary by Salt version.

### 3.1 `version`
Use `salt.version.__version__` (and `sys.version` for Python version). No
config load required — cheapest possible command, useful as an agent
"ping".

### 3.2 `config_snapshot`
Load the effective master config via `salt.config.master_config(path)` where
`path` defaults to `/etc/salt/master` (overridable via salt-radar.toml). This is
a pure read + YAML merge with defaults/includes — no daemon connection.

Extract and return at least:
- `worker_threads`
- `transport` (zeromq / tcp)
- `sock_dir` (needed by the event subscriber too)
- whether `reactor` is configured (non-empty list → reactors active) and
  how many reactor entries / sls files
- `fileserver_backend`, `gitfs_remotes` (count + repo URLs, no credentials)
- `ext_pillar` backends configured (names only)
- `master_job_cache` / returner used for job cache
- `pillar_cache` enabled/disabled
- `presence_events` enabled/disabled (relevant for Minions tab accuracy —
  see §6.2)
- `interface`, `publish_port` (4505), `ret_port` (4506) — in case these are
  non-default, so the Dashboard's connection-count panel queries the right
  ports

Also return `cpu_count` (via `os.cpu_count()`) so salt-radar can show
`worker_threads` vs cores directly.

### 3.3 `minion_cache_list`
Read cached minion data (grains etc.) via the cache subsystem rather than
guessing file formats:

```python
import salt.cache
cache = salt.cache.factory(opts)
minion_ids = cache.list('minions')
for mid in minion_ids:
    data = cache.fetch('minions/{}'.format(mid), 'data')  # verify exact bank/key
    mtime = cache.updated('minions/{}'.format(mid), 'data')  # if available
```

For each minion id, return: `id`, `os`, `osrelease`, `saltversion`,
`cpuarch` (or whatever grains subset is cheap), and the cache entry's last-
updated timestamp (used as "last seen" — see §6.2). If `cache.updated()`
isn't available in this Salt version, fall back to filesystem mtime of the
underlying cache file (still via a path derived from `opts['cachedir']`,
not hardcoded).

### 3.4 `subscribe_events`
```python
import salt.utils.event
event_bus = salt.utils.event.get_event(
    'master',
    sock_dir=opts['sock_dir'],
    transport=opts['transport'],
    opts=opts,
    listen=True,
)
for evt in event_bus.iter_events(full=True):
    # evt expected to contain 'tag' and 'data'
    emit_json({"type": "event", "tag": evt['tag'], "data": evt['data'], "ts": time.time()})
```
Run this in a background thread so the agent can still answer one-shot
commands on stdin while streaming. Wrap in try/except — if the event bus
can't be opened (permissions, wrong sock_dir), return a single `error`
response and do not crash the agent.

### 3.5 `jid_lookup`
Use the runner client for a single targeted lookup:
```python
import salt.runner
runner = salt.runner.RunnerClient(opts)
result = runner.cmd('jobs.lookup_jid', [jid])
meta = runner.cmd('jobs.list_job', [jid])  # fun, tgt, start time, etc.
```
This reads only the cache entries for that one `jid` — no directory walk of
the whole job cache. Return `found: false` cleanly if the jid doesn't exist
or has expired from the cache (don't treat as an error).

---

## 4. Rust-side collectors (no agent involved)

Located under `src/collectors/`. All are simple, synchronous, polled on a
timer (default 2s, configurable).

### 4.1 `keys.rs` — key inventory
List files in:
- `<pki_dir>/master/minions` → accepted
- `<pki_dir>/master/minions_pre` → pending
- `<pki_dir>/master/minions_rejected` → rejected
- `<pki_dir>/master/minions_denied` → denied

`pki_dir` comes from the agent's `config_snapshot` (don't hardcode
`/etc/salt/pki`). Filenames = minion IDs; also capture each file's mtime
(useful for "pending key age" alerts).

### 4.2 `processes.rs` — Salt process memory/CPU
Enumerate processes (reuse psnet's procfs/sysinfo approach). Group by
matching `/proc/<pid>/cmdline` against `salt-master`, `salt-api`,
`salt-syndic`. Note: `salt-master` forks multiple worker processes
(MWorker pool, ReqServer, EventPublisher, Maintenance, etc.) — inspect
`/proc/<pid>/cmdline` on the actual target system, since some Salt versions
set distinct process titles for these via `setproctitle` and some don't.
Design for both cases:
- If distinct titles are visible: show a small breakdown table (process
  role → PID, RSS, CPU%).
- If not: group all `salt-master`-cmdline processes under one "salt-master
  (N workers)" row with summed RSS and a min/max/avg per-worker RSS.

Sum RSS per logical component (Master, API, Syndic) for the Dashboard's
headline numbers.

### 4.3 `services.rs` — service health
For each of `salt-master`, `salt-api`, `salt-syndic` (skip any not present
in `systemctl list-unit-files`):
```
systemctl is-active <unit>
systemctl show <unit> --property=ActiveState,SubState,ActiveEnterTimestamp
```
`ActiveEnterTimestamp` → service uptime. A change in this timestamp between
polls (without salt-radar itself restarting) indicates the service restarted —
feed this to the alert engine.

### 4.4 `connections.rs` — ZMQ transport connection counts
Parse `/proc/net/tcp` and `/proc/net/tcp6` (same approach as psnet's
connection table), filter rows where local port == `publish_port` (default
4505) or `ret_port` (default 4506, both from `config_snapshot`, not
hardcoded). Report:
- total + ESTABLISHED count per port
- a table of distinct remote addresses with connection counts (best-effort
  "who's talking to me right now" — does not attempt to resolve to minion
  IDs in v1; see §9 future ideas)

### 4.5 `system.rs` — host vitals
Load average, memory (`/proc/meminfo`), disk usage for the filesystems
backing `cachedir` and `pki_dir` (from `config_snapshot`), current time,
salt-radar's own uptime, and "seconds since last event seen" (fed by the Events
subsystem — see §5).

---

## 5. Views

### 5.1 Dashboard

Single-screen overview, refreshed every 2s. Suggested layout (rows):

```
┌ salt-radar — <master_hostname> ───────────────── Salt 3007.1 │ 2026-06-14 15:42:03 UTC ┐
│ Keys:  accepted 142   pending 3   rejected 1   denied 0                          │
│ Processes:  salt-master 1.2 GB (12 workers)   salt-api 180 MB   salt-syndic —    │
│ Services:   salt-master ● active (up 14d 2h)   salt-api ● active   salt-syndic — │
│ ZMQ:    :4505 publish  118 established   :4506 return  6 established            │
│ Config: worker_threads 12 / 16 cores   transport zeromq   reactor: 2 configured  │
│         fileserver: roots,gitfs(2)   ext_pillar: none   job_cache: local         │
│ Host:   load 1.4/0.9/0.7   mem 6.1/32 GB   disk(cache) 41%   disk(pki) 2%        │
│ Events: 1,204 seen this session   last event 1s ago   alerts: 2 active ⚠         │
└────────────────────────────────────────────────────────────────────────────────┘
```

Data sources: §4.1–4.5 (Rust) + `config_snapshot`/`version` (agent, fetched
once at startup and refreshed every ~60s since this rarely changes).

### 5.2 Minions Explorer

Sortable/filterable table, one row per minion ID known from *either* the key
directories or the minion cache (union of both, so pending/rejected/denied
minions show up too, with status clearly marked).

Columns: `ID`, `Key status` (accepted/pending/rejected/denied, color-coded),
`OS`/`osrelease`, `Salt version`, `Last seen` (relative time), `Conn?`
(best-effort, derived from §4.4's remote-address table if it can be
correlated, otherwise "—").

- Live text filter on minion ID (glob/substring), same UX as psnet's `f` +
  typing filter.
- Filter/toggle by key status.
- `Enter` opens a detail popup with full grains dump from
  `minion_cache_list` and the raw key-file mtime.

"Last seen" = `max(minion_cache last-updated, most recent event timestamp
seen for this minion id during this session)`. If `presence_events` is
`False` in the master config (common default), display a small note in this
tab: *"last-seen is based on cached data + live events only; enable
`presence_events: True` in the master config for more accurate presence —
salt-radar does not change this for you."*

### 5.3 Events (Sniffer)

The core "packet sniffer" view.

**Layout**: scrolling table (newest at bottom or top, toggleable), columns
`Time`, `Tag`, `Summary` (e.g. "fun=state.apply tgt=web* jid=20260613..."),
plus a header strip with live throughput: events/sec (sparkline, like
psnet's speed graphs) and a small histogram of the top tag prefixes seen in
the current window.

**Filters** (combined with AND), entered via a filter bar like psnet's
`f` + typing:
- `tag:` glob/regex against the event tag (e.g. `salt/job/*/ret/*`,
  `salt/auth`, `salt/presence/*`, or any custom reactor-fired tag).
- `fun:` glob against `data['fun']` when present (e.g. `state.apply`,
  `grains.*`) — this is the "filter by job type" the user asked for.
- `minion:` glob against the minion id, checked against `data['id']`,
  `data['minion_id']`, and the trailing path segment of the tag (covers
  `salt/job/<jid>/ret/<minion>` style tags).

**Detail popup** (`Enter`): pretty-printed JSON of `data`, plus a raw
hex+ASCII view of the original bytes if/when the agent can supply them —
direct callback to psnet's packet detail popup. (v1 can show pretty JSON
only; raw bytes are a stretch goal since the agent receives already-decoded
Python objects from `iter_events`.)

**Buffering**: keep the last N events (configurable, default 5,000) in a
ring buffer in Rust; filters apply to the buffer instantly without
re-subscribing.

### 5.4 Jobs

Two halves of one tab:

**Live job tracker (top)** — built purely from the Events subscription
(no job-cache scanning, per your feedback on cost):
- On `salt/job/*/new`: create a row — `jid`, `fun`, `tgt`, `target_count`
  (len of `data['minions']` if present), `returned 0/target_count`,
  `age`.
- On `salt/job/*/ret/<minion>`: increment `returned`, track per-minion
  success/failure (`data['success']` / `data['retcode']`).
- Rows age out of the view after a configurable time (default 30 min) but
  remain available for the JID lookup box below by jid.

**JID lookup (bottom)** — a text input where the user pastes/types any
`jid`, including ones from before salt-radar started. Sends `jid_lookup` to the
agent (§3.5) and shows a popup with `fun`, `tgt`, start time, and per-minion
results (success/fail, short result summary). This is the single place
salt-radar does an on-demand, targeted historical query — explicitly *not* a
background scan.

### 5.5 Alerts / Health

A list view of currently-active anomalies, each derived from data the other
tabs already collect (no new probes). Suggested rules, each independently
toggleable/threshold-configurable in `salt-radar.toml`:

| Rule | Source | Default threshold |
|---|---|---|
| Stale minion | Minions tab "last seen" | > 1h |
| Key pending too long | Key file mtime in `minions_pre` | > 1h |
| Job stragglers | Live job tracker | no new returns for 2 min while `returned < target_count` |
| Service down | `services.rs` | configured unit not `active` |
| Master/API restarted | `services.rs` `ActiveEnterTimestamp` change | any change since last poll |
| Cache/PKI disk usage high | `system.rs` | > 85% |
| Minion/master version skew | Minions tab vs `version` | any mismatch (informational severity) |

Each alert has a severity (info/warn/crit), a one-line description, and an
"active since" timestamp. A small badge on the Dashboard (`alerts: N
active`) links here. This tab replaces the psnet "Alerts" tab role.

---

## 6. Configuration file (`salt-radar.toml`)

```toml
[salt]
master_config = "/etc/salt/master"   # passed to salt.config.master_config
python_bin     = "/usr/bin/python3"  # must have `import salt` working
agent_path     = "/usr/share/salt-radar/salt_radar_agent.py"

[refresh]
dashboard_secs = 2
minions_secs   = 5
config_secs    = 60

[events]
buffer_size = 5000

[alerts]
stale_minion_secs   = 3600
pending_key_secs    = 3600
job_straggler_secs  = 120
disk_usage_pct      = 85
```

---

## 7. Project layout

```
salt-radar/
├── Cargo.toml
├── README.md
├── salt-radar.toml.example
├── agent/
│   └── salt_radar_agent.py
└── src/
    ├── main.rs              # entry point, terminal setup, event loop
    ├── app.rs               # app state, tab switching, input handling
    ├── config.rs            # salt-radar.toml loading
    ├── agent.rs             # spawn + NDJSON protocol to salt-radar-agent
    ├── types.rs              # shared structs (MinionInfo, JobEntry, AlertEntry, ...)
    ├── collectors/
    │   ├── mod.rs
    │   ├── keys.rs
    │   ├── processes.rs
    │   ├── services.rs
    │   ├── connections.rs
    │   └── system.rs
    ├── alerts/
    │   ├── mod.rs
    │   └── rules.rs
    └── ui/
        ├── mod.rs            # tab bar, status/keybinding line
        ├── dashboard.rs
        ├── minions.rs
        ├── events.rs
        ├── jobs.rs
        ├── alerts.rs
        └── widgets/
            ├── table.rs
            ├── sparkline_panel.rs
            └── detail_popup.rs
```

Suggested crates (reuse what psnet already uses where possible): `ratatui`,
`crossterm`, `serde`/`serde_json`, `procfs` or `sysinfo`, `chrono`. Add
nothing exotic — no zmq/msgpack crates needed, since the agent handles all
Salt-protocol concerns.

---

## 8. Implementation phases (for Claude Code)

1. **Scaffold**: `cargo init`, ratatui+crossterm shell with 5 empty tabs
   (Dashboard / Minions / Events / Jobs / Alerts), tab-switch keys, quit on
   `q`, status bar with keybinding hints — mirroring psnet's `ui/mod.rs`
   structure.
2. **Agent skeleton**: `salt_radar_agent.py` with the NDJSON loop and `version` +
   `config_snapshot` commands only. Test standalone via stdin/stdout before
   wiring into Rust. **Do the "Verification needed" API check from §3 here.**
3. **Agent integration**: `agent.rs` spawns the process, sends/receives
   NDJSON on background threads/channels, exposes results into shared app
   state. Dashboard shows version + config snapshot fields.
4. **Rust collectors**: implement §4.1–4.5, wire into Dashboard for the full
   layout in §5.1.
5. **Events tab**: add `subscribe_events`/`minion_cache_list` to the agent;
   Rust ring buffer + live table + filter bar (`tag:`/`fun:`/`minion:`) +
   throughput sparkline + detail popup.
6. **Jobs tab**: derive live job tracker from the same event stream (fan out
   to both Events and Jobs views); add `jid_lookup` agent command and the
   JID input/popup.
7. **Minions tab**: combine key directory listings with
   `minion_cache_list`, live "last seen" tracking, filter/sort, detail
   popup.
8. **Alerts tab**: implement rule engine from §5.5 over already-collected
   state, configurable via `salt-radar.toml`, Dashboard badge.
9. **Config + polish**: `salt-radar.toml` loading/defaults, theming consistent
   with psnet's dark palette, help overlay, agent-crash banner + restart,
   README (install, root requirement, `python_bin`/`agent_path` setup).

---

## 9. Explicitly out of scope for v1 (future ideas)

- **Key management actions** (accept/reject/delete pending keys) — would
  need a separate, clearly-labeled "actions" mode with confirmation
  dialogs, deliberately deferred so v1 stays strictly view-only.
- **Topology view** — dropped per discussion.
- **Minion IP → minion-ID correlation** for the connection table in §4.4 —
  possible via grains (`fqdn_ip4`/`ip4_interfaces` in `minion_cache_list`)
  but not required for v1.
- **Agent-side event pre-filtering** — if event volume on very busy masters
  makes the NDJSON pipe a bottleneck, add `tag:`/`fun:` filters to the
  `subscribe_events` command so the agent filters before serializing.
- **Multi-master/syndic awareness** beyond showing whether `salt-syndic` is
  running locally.
- **Exporting the event buffer** to a file (jsonl) for later offline
  review.
