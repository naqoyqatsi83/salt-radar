# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.3] - 2026-06-17

### Fixed
- Minion list now shows only minions with accepted keys (not stale cache entries) — total went from 3954 to actual accepted count.
- Dashboard key status panel total sums the four displayed key categories instead of using raw cache count.
- Home/End keys jump to first/last item in Minions, Events, Jobs, Alerts tabs.

## [0.3.2] - 2026-06-17

### Fixed
- Fleet scan stability: saved stdout fd so event thread is immune to fd-level redirects
- Fleet overview distribution bars no longer randomly reorder on refresh (added alphabetical secondary sort)
- Last seen timestamp now populated from return events in the event stream
- Minion list: unresponsive minions shown with dim foreground and `—` dashes instead of stale data
- Timed-out count shown in fleet overview header
- Minion info and fleet data sorted by ID for stable ordering across refreshes
- Minion detail popup shows osfinger cleanly when fleet scan data is available
- Fleet scan detects unresponsive minions via salt-key pre-check and reports them as timed out

## [0.3.1] - 2026-06-17

### Fixed
- Event sniffer: added spacing between `#` and Time columns so they don't merge together on large event numbers

## [0.3.0] - 2026-06-17

### Added
- Admin mode with danger-zone confirmation dialog (press `a` to toggle)
- Fleet scan via Salt LocalClient (`grains.items` on all minions) — press `s` in admin mode
- High-contrast selection mode togglable with `c` (steel-blue background + bold), works universally including inside popups
- Advanced event sniffer: filter events by minion ID substring from Events tab (`n` key)
- Event sniffer shows `# | Time | JID | Type` columns with full pretty-printed JSON body per event
- Sniffer pause/resume (`p`), event detail via Enter, clipboard copy of all events (`y`)
- Return-to-sniffer: pressing q/Esc on an event detail opened from the sniffer returns to the sniffer
- Sniffer auto-scroll: selected event header stays at ~1/3 from viewport top
- Event number column (`#`) in sniffer for orientation
- Footer/dismiss line pinned to bottom of every popup (always visible regardless of content length)
- Date formatting in clipboard copies (UTC `%Y-%m-%d %H:%M:%S` instead of raw Unix timestamps)

### Changed
- Events view date format from `%H:%M:%S` to `%Y-%m-%d %H:%M:%S` (local + UTC)
- Minions tab redesigned: 55/45 horizontal split with minion list / fleet overview (OS Distribution, Salt Version, Architecture bars)
- Event sniffer now shows full JSON body with no truncation (removed 5-line limit)
- Sniffer column layout: `# | Time | JID | Type` with JSON indented below each row
- `detail_popup::render_popup` now uses separate layout for body + pinned footer with scroll support
- Dashboard header version bumped to v0.3.0

### Fixed
- Filter-header bug: header line now always shown when data exists (Events, Jobs, Alerts)
- Dashboard: inactive services (e.g. salt-api when stopped) no longer show stale uptime — `uptime_seconds` set to 0 when not active
- Column alignment between header and data rows in Minions tab
- High contrast color with JID text: changed to steel-blue `Rgb(40,55,80)` for readability
- Contrast toggle blocked by open popup — universal `c` handler added before popup dispatch
- Sniffer scrolling: cursor no longer runs off-screen, uses Paragraph::scroll() with auto-scroll
- Clipboard copy of events uses proper date format instead of Unix epoch timestamp

## [0.2.0] - 2026-06-16

### Added
- Services panel with dynamic discovery (systemctl list-unit-files salt-*.service), cross-referenced with /proc for accurate active/running state
- Services panel columns: Unit, Status, State, Memory, Uptime with proper alignment
- Hostname display in Dashboard header (from socket.gethostname())
- btop-style responsive sparklines: horizontal layout (60/40) on wide terminals, vertical stacking on narrow (<80 cols)
- btop-style scrolling sparklines: newest data on the right, window scrolls left as new points arrive
- Ring buffer expansion from 60 to 300 samples for wider time window
- Adjustable refresh interval via +/- keys (range 500ms–30s, default 2s), shown in panel titles
- Interval hint in bottom status bar (Dashboard tab)
- Separate Source column in Jobs tab (master/local) to distinguish salt-call from master-scheduled jobs
- salt-call job tracking: synthetic job entries created from return events when no /new event precedes them

### Changed
- Dashboard services panel height now exactly fits discovered service count instead of oversized Min(4) constraint
- Sparklines use 1 data point per column (window width determines how far back you see)
- ZMQ/Event Rate/Minion Activity panel titles show current refresh interval
- Network & Latency panel now uses same btop-style responsive/sparkline layout as Event Rate/Minion Activity

### Fixed
- Sparklines showing no bars: data was taken from oldest ring buffer entries instead of newest (ordered[0..points] → ordered[end-points..end])
- 'q' key in detail popup quitting entire program instead of closing popup (added had_popup guard and missing KeyCode::Char('Q') to popup close handlers)
- Services uptime not parsed: added chrono format support for systemd timestamps with named timezones (CEST, UTC, etc.)
- Services panel not wrapped in block frame (was missing Borders::ALL)
- ZMQ process detection including salt-api and salt-syndic processes
- Service discovery including disabled services (--all flag)
- Duplicate function definition errors in service collector
- sparkline width calculation using full panel width before layout split

## [0.1.0] - 2026-06-15

### Added
- Initial release of salt-radar TUI for monitoring Salt Master
- Dashboard tab with radar spinner, uptime, processes, services, host resources, ZMQ latency with sparkline, event rate, minion activity matrix, job pulse
- Minions tab with key status, grains, connection status
- Events tab with live event stream and filtering
- Jobs tab with live job tracking (JID, function, target, returned/targeted, age)
- Alerts tab with minion staleness, pending keys, service health, disk usage alerts
- Job detail popup with per-minion return navigation (j/k), scroll mode (Enter), copy to clipboard (y/Y)
- Clipboard integration: tmux load-buffer + xclip via X11 forwarding (works from MobaXterm → SSH → tmux → root)
- ZMQ latency measurement via agent round-trip (Rust-side timing)
- Agent communication via NDJSON over stdin/stdout with file-based command queue
- Salt event bus subscription for real-time updates
- Configurable refresh intervals via `salt-radar.toml`

### Changed
- Job detail output now matches `salt-run jobs.lookup_jid` exactly by parsing CLI output and splitting by minion headers

### Fixed
- Clipboard freeze: subprocess timeout with kill -9 prevents TUI hang
- ZMQ ping no longer blocks agent loop (Rust measures round-trip)
- TMUX socket detection works as root (parses TMUX env or scans /tmp/tmux-*/default)
- X11 user detection for su -c xclip (fallback users: detected, vagrant, peto)
- Screen corruption after clipboard: needs_terminal_clear flag for clean re-render
- Job detail fun/tgt/start now populated from event data (not empty)
- Jobs view column widths: JID 28, Function 30, Target 36
