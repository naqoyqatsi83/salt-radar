# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Event type breakdown panel on Dashboard mid section: real-time counts and proportional bars for auth, job return, minion start, beacon, and other events — shown alongside Services and Minion Keys in a 3-column layout
- Config editor popup (F3): edit all 12 TOML parameters grouped by section with real-time editing, validation, and save
- Collapsible timed-out minion section (z key) with expand/collapse indicator on Minions tab
- Configurable sniffer buffer size via `sniffer_buffer_size` in `salt-radar.toml`
- Minion list key status column (accepted/pending/rejected/denied) with color coding
- Full minion grains viewer in detail popup: press `g` in admin mode to fetch and scroll through all grains
- Minion connection tracking via `alive_check` agent command (passive cache-file check)
- Admin mode minion ping (`test.ping`) from detail popup with connection result
- Minion detail popup: key status, connection status, OS, Salt version, CPU architecture, last seen time
- Agent-based system data collection: key listing, process listing, service discovery, system info, ZMQ connections, minion ping, minion grains commands
- Agent debug logging to `/tmp/salt-radar-agent-ndjson.log`
- Deployment target and testing documentation in CLAUDE.md
- `sniffer_buffer_size` field to config and TOML

### Changed
- System data collection moved from Rust-side `/proc` collectors to Python agent for reliability
- Event panel header shows `Events (current/max)` instead of total only
- Default `event_buffer_size` increased from 5000 to 10000
- Minion detail popup layout widened to 80% width with split info/grains sections
- Agent event subscription wrapped in try/except for graceful startup
- `DetailPopup::Minion` variant refactored to carry ping state, grains data, and scroll position
- Dashboard key status panel uses max of key list and minion key_status for totals
- Pulse view filtering uses `key_status` field fallback when key data missing

### Fixed
- Sniffer buffer now respects configurable `max_size` instead of hardcoded 1000
- Timed-out minion `key_status` and enriched fields (osfinger, osrelease, salt_version, cpuarch, last_seen) preserved across cache list refreshes
- Key status shown in minion list even when key data is from minion_info fallback
- Fleet scan minions get `key_status: Accepted` by default

## [0.6.0-rc1] - 2026-07-04

### Added
- Mouse click on Pulse view minion grid now selects the exact minion indicator instead of just left/right half
- Double-click on list items (Minions, Events, Jobs, Alerts) opens the detail popup (400ms threshold, same cell)
- Right-click on a popup closes it (except AdminConfirm confirmation dialog)
- F2 hotkey toggles mouse mode on/off, with visual indicator in the key hints bar (default: enabled)

### Changed
- List tab click positioning rewritten from proportional calculation to exact row mapping with scroll offset for precise item selection
- `DBL_CLICK_MS` constant renamed to `dbl_click_ms` for clippy compliance

### Fixed
- AdminConfirm popup OK/Cancel buttons now correctly distinguished by exact text hitbox coordinates — clicking Cancel no longer enables admin mode

## [0.5.1] - 2026-06-29

### Changed
- Events tab split into left/right panels: event list on left (50%),
  selected event preview on right (50%) showing tag, timestamps,
  function, and JSON body inline
- Pulse tab completely reworked: replaced concentric ring constellation with a wrapped activity grid
  that scales from 15 to 4,000+ minions — each minion shown as a colored `·` in reading order
- Grid navigation: Up/Down rows, Left/Right columns (both wrap), no ring math
- Updated Cargo.toml version to 0.5.1, key hints, and help text

### Added
- Passive grain harvesting: minion data (osfinger, osrelease, os, saltversion, cpuarch) now populated
  from any grains.item/items return events on the bus, not only during active fleet scans

### Fixed
- Pulse view now filters to only accepted-key minions, matching Dashboard and Minions views — no longer shows stale entries from deleted keys
- Pulse tab dots now show green for all known minions instead of grey (removed overly strict has_data gate)
- Pulse tab return tracking now extracts minion ID from event tag (salt/job/*/ret/minion_id) instead of data["id"], matching the existing proven event-parsing pattern
- Pulse tab green tier extended to 300s (5min) to match minion `/start` interval, removed yellow tier
- Pulse tab flash now uses bright lime-green `Rgb(120,255,120)` + BOLD for clear visibility
- Pulse tab broadened to track all minion events: returns (`/ret/`), minion start (`/minion/*/start`), and auth/key events
- Color tiers: bright green (<5s), green (<5min), orange (<10min), grey (>=10min), red (timed out)
- Pulse tab now also tracks beacon events (`salt/beacon/*`) as minion activity
- Pulse tab now catches any event carrying a minion ID in `data["id"]` as a fallback, covering auth, key, and other events
- Pulse tracking moved to drain_agent_messages() at event receipt time, not deferred to update()
- Fleet overview now populates from passive grain collection and shows without admin mode
- Fleet overview labels passive data as such with hint to run full scan for complete results

## [0.5.0] - 2026-06-28

### Added
- Network traffic monitoring on Dashboard Host Resources panel: bytes in/out, current transfer rate (B/s, KB/s, MB/s, GB/s auto-scaled), cumulative totals from `/proc/net/dev`
- Fleet overview now displays distribution bars incrementally during scan instead of hiding everything until complete
- Minion list OS column no longer capped at 30 chars — split proportionally with Salt version column so both remain visible
- Fleet scan progress now counted from accepted keys (not agent's optional expected_minions list), fixing "1/1" display bug
- Missing data shown consistently as "?" across OS Distribution, Salt Version, and Architecture in fleet overview
- Minion list selection highlight now consistent with other tabs — stops at last text character instead of spanning full width
- README: added fancy badge row (Rust, Linux, ratatui, Salt, MIT)

### Changed
- Updated Cargo.toml version to 0.5.0, key hints, and help text

## [0.4.5] - 2026-06-26

### Fixed
- Home/End navigation in Event Sniffer now works (jumps to newest/oldest event)
- Ctrl+H (^H) from terminals that send it instead of DEL now correctly treated as Backspace for all filter inputs
- Clipboard: xclip availability is now checked before use, with clear status bar message when missing
- Clipboard: tmux `set-clipboard on` setting is now verified, with warning when off
- Clipboard: runs xclip detached (never killed) so it stays alive as clipboard selection owner
- Clipboard: detects X11 user from tmux socket ownership for SSH X11 forwarding (localhost:N) where no /tmp/.X11-unix/X socket exists
- Clipboard: XAUTHORITY fallback reads home dir from `/etc/passwd` and finds cookie via `/proc/<pid>/environ`
- Clipboard: also checks `/run/user/<uid>/xauth*` for random-named pam_xauth files on Ubuntu
- Clipboard: comprehensive X11 diagnostics written to `/tmp/.salt-radar-clip-debug-<pid>.log`
- Clipboard: warnings shown in popup footer (not just status bar) so they're visible during popup usage
- Temp files now use PID-based names (`/tmp/.salt-radar-*.<pid>.*`) so multiple users/instances don't conflict
- Temp files cleaned up on exit (agent cmd, clipboard text, debug logs)

## [0.4.4] - 2026-06-24

### Changed
- Job detail popup redesigned with split-panel layout: returns list on left (25%), return content preview on right (75%)
- Returns list now shows compact entries (minion_id: OK/FAIL (retcode=X)) without summary text
- Enter on a return switches focus to the right panel for scrolling through the return output
- Return list can be filtered with `/` key by hostname substring; Enter commits filter, Esc cancels, selection snaps to nearest match on commit
- Right panel (preview) supports scrolling with j/k, PgUp/PgDn, Home/End, and Enter to enter/exit scroll mode
- y/Y copy works from either panel

### Fixed
- 'c' key no longer triggers high-contrast toggle during return filter input
- Job detail lookup timeout: pending query auto-clears after 65s if agent hangs, closing the popup instead of showing permanent "Querying master..."
- Return filter no longer causes empty left panel: scroll calculation now uses filtered position, not full-list index
- Return filter snaps to first match when lookup data arrives after filter was committed
- Pressing `/` to start a new filter correctly clears previous filter text

## [0.4.3] - 2026-06-24

### Fixed
- Job detail popup now scrolls when navigating many minion returns — selected return stays visible instead of scrolling off-screen
- PageUp/PageDown/Home/End keys now work for navigating minion returns in job detail popup

## [0.4.2] - 2026-06-22

### Fixed
- Popup overlay bleed-through: added `Clear` widget in `render_popup` to prevent underlying panels from showing through
- Hotkeys (1-5, Tab, c) now muted during sniffer keyword highlight input so characters typed are consumed by the input handler instead of switching tabs

## [0.4.1] - 2026-06-22

### Added
- Job Cache split panel: Jobs tab split into Live Jobs (left, 55%) and Job Cache (right, 45%)
- Job Cache pagination: `[`/`]` adjust page limit (±500), `,`/`.` navigate pages
- Server-side job cache search: filter across jid/fun/tgt/user on the full cache (not just displayed page)
- Page indicator in Job Cache panel title (page number, limit, total count)
- 120s timeout guard for cache loading with retry message
- Event detail scroll mode: Enter to scroll full event JSON, with Home/End navigation
- Job detail shows all return lines by default (no 30-line cap), matching Events view
- Viewport-aware scroll clamping: End positions last lines at viewport bottom
- Event sniffer keyword highlighting: press `h` to set highlight words, matches shown in bold yellow
- Job detail output now preserves Salt's native ANSI color codes for state/function/ID coloring
- Copy-to-clipboard `y` strips ANSI codes automatically, yielding clean plain text
- Sniffer selection background fills to full popup width for easier visual tracking

### Changed
- Tab now consistently switches tabs everywhere (previously toggled panel focus on Jobs tab)
- `←`/`→` arrows switch focus between Live Jobs and Cache panels on Jobs tab
- Job detail `y` key shows exact `salt-run jobs.lookup_jid` console output; `Y` still copies raw JSON
- Live Jobs now uses the same agent-based lookup as Cache Jobs, so both show identical formatted output
- Agent runs `salt-run --force-color` with `CLICOLOR_FORCE=1` to emit ANSI codes for job detail display

### Fixed
- Job cache search now works across pagination: server filter preserved independently of input text
- Job detail success/fail detection: derives success and retcode directly from return value structure (state dicts check `result` key, primitives check truthiness, error strings detected)
- `cmd_jid_lookup` agent command: rewrote CLI text parser to `--out=json` structured output, eliminating fragile minion-header parsing
- `cmd_jobs_list` agent command: switched from `RunnerClient` (hung indefinitely in background threads due to Python/Salt threading issues) to `subprocess.run(["salt-run", "jobs.list_jobs", "--out=json"])`
- Job cache timestamps: use JID-derived epoch (YYYYMMDDHHMMSS) as primary source for reliable UTC timestamps, displayed in local time
- `_parse_stamp` handles ISO-8601 with T or space separators, with or without fractional seconds, and `datetime` objects
- Clipboard `y`/`Y` from cache job detail popup now works correctly

## [0.3.6] - 2026-06-20

### Changed
- Fleet scan is now fully event-driven: `cmd_async` fires the job and returns immediately. Returns arrive incrementally via the ZMQ event bus — minion grains (osfinger, saltversion, cpuarch) appear in the list as each minion responds, with no blocking wait.
- Fleet scan 60s grace period: minions that don't respond within 60 seconds are marked as non-responsive instead of being ignored.

### Fixed
- Clipboard: XAUTHORITY is now auto-resolved from the process environment or tmux session global environment (`tmux show-environment -g XAUTHORITY`). This means xclip works under `sudo` without any `sudoers` configuration — the code finds DISPLAY and XAUTHORITY via the original tmux session even when the root shell has neither set.
- Clipboard: removed buggy `find_xauthority()` code that caused "X11 connection rejected because of wrong authentication" errors. Replaced with clean XAUTHORITY forwarding via env/tmux.
- Clipboard: no hardcoded usernames — X11 socket owner detection uses `/tmp/.X11-unix/` ownership lookup with dynamic fallback to `USER`/`LOGNAME` env vars.

## [0.3.5] - 2026-06-18

### Added
- Minions tab: responsive minions sorted to top after fleet scan (alive first, timed-out below, each group by ID).
- Press `x` on Minions tab to toggle hiding non-responsive (timed-out) minions. Only available after at least one fleet scan.

### Changed
- Fleet scan runs in a background thread with 10s timeout (interim approach).

## [0.3.4] - 2026-06-17

### Fixed
- Header and help menu now show correct version (used compile-time constant instead of hardcoded string).
- Service unit column widened from 16 to 24 chars for longer service names.
- Dashboard key status panel total sums displayed categories instead of raw cache count.
- Minion list now shows only minions with accepted keys (not stale cache entries).

### Performance
- Heavy collectors (services, processes, keys, system) now refresh at `dashboard_secs` interval (default 2s) instead of every tick, eliminating ~14 systemctl subprocess calls per second.
- Removed duplicate `systemctl list-unit-files` call in services collector.
- Job detail lookup runs in a background thread with 30s timeout — no longer blocks the agent (was freezing for 4+ minutes with unresponsive minions).
- Filter matching on Events and Jobs tabs uses `HashSet` from accepted keys instead of cloning 3954 cache entries per keystroke.

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
