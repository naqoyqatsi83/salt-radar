<p align="center">
  <img src="https://img.shields.io/badge/rust-1.70%2B-orange?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/platform-Linux-1793D1?style=for-the-badge&logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/TUI-ratatui-blue?style=for-the-badge" alt="Ratatui">
  <img src="https://img.shields.io/badge/Salt-monitoring-00E272?style=for-the-badge&logo=saltproject&logoColor=white" alt="Salt">
  <img src="https://img.shields.io/badge/license-MIT-green?style=for-the-badge" alt="MIT License">
</p>

<h1 align="center">salt-radar</h1>

<p align="center">
  <b>A non-intrusive, read-only TUI for monitoring a Salt Master</b><br>
  Built with <a href="https://github.com/ratatui-org/ratatui">ratatui</a> + <a href="https://github.com/crossterm-rs/crossterm">crossterm</a>
</p>

<br>

salt-radar gives you a live, at-a-glance view of your Salt Master infrastructure — directly in your terminal. It reads from `/proc`, `systemctl`, and Salt's own APIs (via a bundled Python agent) to show you what's happening without needing the Salt CLI or web UI.

No database, no web server, no JavaScript. Just a single binary and a Python agent.

## Features

### Dashboard
Live overview of your Salt Master with real-time sparkline panels:

- **Host Resources** — CPU load, memory usage, disk usage (cache + PKI)
- **Services** — All `salt-*.service` units auto-discovered via systemctl, with active state, memory, and uptime
- **Minion Key Status** — Accepted / Denied / Unaccepted / Rejected key counts
- **ZMQ Latency** — Round-trip latency sparkline with established connection counts per port
- **Event Rate** — Events/sec sparkline with peak and total counters
- **Minion Activity** — Job return rate sparkline with last-active minion tracking
- **Job Pulse** — Active jobs with progress bars

### Minions
- Scrollable minion list with dynamic column widths (ID, OS, Salt Version, Arch)
- Fleet overview panel: OS Distribution, Salt Version, and Architecture distribution bars
- Detail popup with key info and connection status
- Filter by ID, OS, or version (`/` key)

### Events
- Live event stream from the Salt event bus (via ZMQ)
- Full date/time in local and UTC columns
- Filter by tag, function, minion ID, or any JSON field
- Advanced **Event Sniffer** — filter events by minion ID substring (`n` key):
  - Pause/resume live capture (`p`)
  - Navigate events with full pretty-printed JSON body
  - Open event detail from sniffer (`Enter`), return to sniffer on close
  - Copy all captured events to clipboard (`y`)

### Jobs
- Split panel: **Live Jobs** (left) from event bus and **Job Cache** (right) from Salt's master job cache
- Job cache supports pagination (`[`/`]` adjust limit, `,`/`.` page navigation) and server-side search
- Per-minion return navigation (j/k), scrollable output viewer (Enter)
- Copy return output to clipboard (y = Salt outputter format, Y = raw JSON)
- Job detail popup shows correct OK/FAIL status derived from return values

### Alerts
- Minion staleness detection
- Pending key warnings
- Service health monitoring
- Disk usage thresholds
- Configurable thresholds via `salt-radar.toml`

### Admin Mode
- Toggle with `a` (danger-zone confirmation dialog required)
- **Fleet scan** (`s`): runs `grains.items` on all minions via Salt LocalClient, populates OS/Salt Version/Arch data

### Clipboard

Copy-to-clipboard (`y` / `Y`) uses two methods:

1. **tmux load-buffer** — pushes text into the tmux paste buffer via OSC 52 escape sequence. MobaXterm (or any terminal emulator that handles OSC 52) captures this and places it on the system clipboard. Works automatically when running inside tmux. Requires `set -g set-clipboard on` in `~/.tmux.conf` to push to the system clipboard (without it, content is only available via `Ctrl+b ]` inside tmux).

2. **xclip** — pushes text to the X11 clipboard via `su` as the user that owns the X11 socket. The code auto-resolves DISPLAY and XAUTHORITY by checking the process environment first, then falling back to the tmux session's global environment, and finally trying the detected user's default `~/.Xauthority` path. This means clipboard works even when salt-radar is launched under `sudo`, with no `sudoers` configuration required.

If DISPLAY and XAUTHORITY can't be resolved, a transient yellow hint is shown in the status bar explaining that X11 clipboard is unavailable. Run with `ssh -X` or `ssh -Y` to enable X11 forwarding.

### Everywhere
- **High-contrast selection mode** (`c`) — steel-blue background + bold, works inside popups too
- **Clipboard integration** — tmux load-buffer + xclip via X11 forwarding. DISPLAY and XAUTHORITY are auto-resolved from the process environment or the tmux session's global environment — no manual configuration needed even when running under `sudo`
- Filter-as-you-type with `/` on all list tabs
- Keyboard-driven navigation with `?` for help

## Installation

### Requirements
- A Salt Master (reads `/etc/salt/master`, connects to the Salt event bus)
- Python 3 with the `salt` package installed
- **Optional — clipboard:** `xclip` (for X11 clipboard integration). Not required when using tmux with `set -g set-clipboard on` and a terminal that supports OSC 52 (MobaXterm, iTerm2, etc.)

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

All settings are optional. The config file is looked up in the current directory or `~/.config/salt-radar/salt-radar.toml`.

```toml
[salt]
# Path to Salt master configuration file
# master_config = "/etc/salt/master"

# Python binary that has the `salt` package installed
# python_bin = "/opt/saltstack/salt/bin/python3"

[refresh]
dashboard_secs = 2
minions_secs = 5
config_secs = 60

[events]
buffer_size = 5000

[alerts]
stale_minion_secs = 3600
pending_key_secs = 3600
disk_usage_pct = 85.0
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `q` | Quit |
| `Tab` | Switch tab |
| `1`–`5` | Jump to tab |
| `↑`/`↓` | Scroll / navigate |
| `PgUp`/`PgDn` | Page scroll |
| `Enter` | Open detail popup |
| `Esc` | Close popup / help |
| `/` | Filter (current tab) |
| `a` | Toggle admin mode |
| `c` | Toggle high-contrast selection |
| `?` / `h` | Help overlay |
| `+`/`-` | Adjust refresh interval (Dashboard) |
| `m` | Toggle sparkline style (Dashboard) |

### Per-tab

| Tab | Key | Action |
|-----|-----|--------|
| Events | `n` | Launch event sniffer |
| Minions | `r` | Minion event/return history popup |
| Minions | `s` | Fleet scan (admin mode only) |
| Jobs (Cache) | `/` | Filter cache entries (server-side) |
| Jobs (Cache) | `[`/`]` | Decrease/increase page limit by 500 |
| Jobs (Cache) | `,`/`.` | Previous/next page |
| Jobs (Cache) | `r` | Refresh job cache |
| Jobs (Cache) | `←`/`→` | Switch focus between Live Jobs and Cache panels |

### Popups

| Key | Action |
|-----|--------|
| `y` | Copy to clipboard |
| `Y` | Copy raw JSON (job detail) |
| `j`/`k` | Next/previous return (job detail) |
| `PgUp`/`PgDn` | Jump 10 returns |
| `Home`/`End` | First/last return (job detail) |
| `Enter` | Enter scroll mode (event/job detail) |
| `Esc` | Exit scroll mode |
| `Home`/`End` | Jump to start/end of output (scroll mode) |
| `h` | Edit highlight keywords (sniffer) |
| `p` | Pause/resume (sniffer) |

## How It Works

```
┌──────────────────────────────────────────┐
│          salt-radar (Rust TUI)           │
├──────┬─────────┬────────┬───────┬────────┤
│ Dash │ Minions │ Events │  Jobs │ Alerts │
├──────┴────┬────┴────────┴───────┴────────┤
│   Python Agent (ZMQ, client, fleet scan) │
├───────────┴──────────────────────────────┤
│  Rust: /proc, systemctl, ZMQ, clipboard  │
│  Salt Master: ZMQ bus, PKI, job cache    │
└──────────────────────────────────────────┘
```

The Rust TUI spawns a Python agent subprocess and communicates via NDJSON over a file-based command queue. The agent subscribes to the Salt event bus (ZMQ) and dispatches commands to Salt's client/runners. Meanwhile, the Rust side collects OS-level data directly from `/proc` and `systemctl` for the dashboard panels.

All data stays local — no external dependencies, no database, no HTTP server.

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
├── app.rs               # Application state, key handling, clipboard
├── types.rs             # Data types (minion, event, job, alert, etc.)
├── config.rs            # TOML config loading
├── agent.rs             # Python agent subprocess management
├── alerts/              # Alert engine (staleness, keys, services, disk)
├── collectors/          # OS-level data collection
│   ├── keys.rs          # PKI key file parsing
│   ├── processes.rs     # /proc scanning for salt processes
│   ├── services.rs      # systemctl service discovery
│   ├── connections.rs   # /proc/net/tcp for ZMQ connections
│   └── system.rs        # System load, memory, disk usage
└── ui/                  # Terminal UI
    ├── mod.rs           # Master draw, theme, key hints, help
    ├── dashboard.rs     # Dashboard tab
    ├── minions.rs       # Minions tab
    ├── events.rs        # Events tab
    ├── jobs.rs          # Jobs tab
    ├── alerts_view.rs   # Alerts tab
    └── detail_popup.rs  # All popup windows (minion, event, job, sniffer, etc.)
agent/
└── salt_radar_agent.py  # Python agent: Salt client, event subscriber, fleet scan
```

## License

MIT — see [LICENSE](LICENSE).

salt-radar is not affiliated with or endorsed by SaltStack / VMware / Broadcom.
