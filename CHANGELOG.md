# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

This release folds in `v0.6.0-rc1` through `v0.6.0-rc16`. Where a later RC
changed or fixed something an earlier RC in this same range introduced, only
the final end-state behavior is described below — the intermediate iterations
aren't (see the individual RC tags for that history). One such case: rc13
added a `~/.config` fallback for `salt-radar.toml`; rc14 then reworked the
Config Editor's border hints around it. The behavior actually shipping in
this release is the "Config file lookup" entry under Changed below.

### Added
- Job Cache: live "N jobs read" progress while loading, instead of an opaque "Loading..." with no feedback for the whole fetch. The agent now walks the job cache directory itself and reports progress every 250 jobs when the master's job-cache backend is the default `local_cache` (falling back to the previous single-shot `salt-run jobs.list_jobs` call, with no live count, for any other backend).
- Config Editor (`F3`): edit every `salt-radar.toml` setting grouped by section, with live validation and save-on-restart. Fixed-choice fields (e.g. `zmq_latency_source`) use an inline picker instead of free text, and `graph_order` edits as a reorderable list (`↑↓` select, `+`/`-` move, `Enter` confirm) instead of a hand-typed comma-separated string.
- Dashboard: CPU and Memory utilization-history sparklines (`F4`/`F5`), alongside the existing ZMQ/Events/Minion-activity panels (`F6`/`F7`/`F8`) — all five enabled by default. Which panels are shown and their stacking order are configurable (`show_*_graph`, `graph_order`) and saved automatically on quit, not just via an explicit Config Editor save.
- Dashboard Job Pulse panel: rolling job-health KPIs — return success rate, stuck/in-flight job count, completion latency (p50/p95), partial-return rate.
- Dashboard Event Types panel: real-time counts and proportional bars per category (auth, job return, minion start, beacon, refresh, other), with the live auth rate shown in the panel title.
- Dashboard Services panel: per-unit restart/flap count (`Flp`, rolling 24h) and a memory-trend arrow (`T`, linear-regression slope over sampled history that requires the recent portion of the window to still be trending upward), plus a worker-pool saturation gauge (`Workers X% busy`) from real per-process CPU sampling.
- Dashboard ZMQ panel: the latency measurement source is configurable (`zmq_latency_source`) — `agent_ipc` (default, local Rust↔agent round-trip), `job_ping` (dispatches a real `test.ping` and times it over the actual event bus), or `master_stats` (reads Salt's own `master_stats` events; requires `master_stats: True` on the master).
- Minions tab: per-minion event/return history popup (`r`) showing events, returns, starts, beacons, cache jobs, and alerts for the selected minion, filterable by keyword (`/`); check-in age percentiles (p50/p90/p99) and a `●` "recently checked in" marker in the ID gutter, with the freshness threshold derived from the fleet's own median check-in age.
- Minion Detail popup: full grains viewer (`g`, admin mode) with indented, type-colored JSON; admin-mode minion ping (`test.ping`) with a live result shown inline.
- Fleet Overview (Minions tab): OS/Salt-version/architecture distribution bars populate passively from grains traffic on the event bus, without requiring a full admin-mode fleet scan (`s`) first.
- New alert rules: Stuck job, Reauth storm, Pending key churn, Service flapping, Service memory growth, and Service memory high — the last with independent absolute (`service_mem_abs_threshold_mb`) and percent-of-total-memory (`service_mem_pct_threshold`) thresholds, each overridable per standard Salt service (`salt-master`/`salt-minion`/`salt-api`/`salt-syndic`) since the master legitimately runs with a much larger working set. Total alert rule count: 13.
- Pulse tab: a Legend panel explaining each recency tier's glyph/color, a direct tab-jump key (`6`), and a distinct `•` glyph for the freshest tier (<5s).
- Mouse support (`F2`): click to select list rows, double-click to open a detail popup, right-click to close a popup; on/off indicator in the title frame.
- Every scrollable list or popup (Minions, Events, Jobs, Alerts, Config Editor, Grains/Event/Job detail, Pulse) shows "▲ more above" / "▼ more below" hints when content extends past the visible window, and documents its `PgUp`/`PgDn`/`Home`/`End` bindings.
- `F1` bound to help, aliasing `?`.
- README screenshots refreshed for every tab.

### Changed
- Shortcuts moved out of the bottom key-hint bar and into the border of the panel each one affects (btop-style `┤key:label├`), so a hint stays next to the behavior it switches; the bottom bar is now status-only (interval, agent connectivity, admin/contrast state, selection position). Every popup — previously a mix of hand-written footers, or none at all for the Config Editor — now uses the same border-hint style.
- Refresh interval (`+`/`-`) is shown as a live btop-style value (`┤-2000ms+├`) in the title frame and works from any tab, not only the Dashboard.
- Title bar and tab menu merged into one bordered frame (app name/version, master version, centered clock, alert count, host/uptime), replacing both the old single-row title bar and the per-tab Dashboard header that used to repeat the same information one row below it.
- Config file lookup: `./salt-radar.toml` in the launch directory wins if present; otherwise it falls back to `$XDG_CONFIG_HOME/salt-radar/salt-radar.toml` (usually `~/.config/salt-radar/salt-radar.toml`), creating the directory on first save. An existing per-directory config keeps working unchanged.
- Sparkline panels default to Dots instead of Bars on first run. CPU/Memory sparklines in Bars mode render on a fixed 0-100 scale instead of auto-scaling to the visible window's own max, which previously exaggerated small values to a full-height bar.
- Service memory reporting reads the systemd unit's cgroup `MemoryCurrent` (covers the whole process tree, shared pages counted once) instead of just the main process's own RSS, falling back to RSS when cgroup accounting is unavailable.
- System data collection (keys, processes, services, system info) moved from Rust-side `/proc` polling to the Python agent for reliability.
- Column widths across Dashboard (Services, Key Status, Event Types), Minions (Fleet Overview, list), Jobs (Live/Cache), Alerts, and Config Editor are now derived from the actual content present instead of hardcoded guesses, so long unit/rule/label names no longer overflow their column or misalign the ones after it.
- Admin-mode key is consistently shown/typed as lowercase `a` everywhere (was `A` in a few spots), and every inline key mention in body text is wrapped in quotes (e.g. `press 's'`) for readability.
- Fold the CPU/Memory/ZMQ/Events/Minions dashboard sparkline panels' below-graph summary line (avg, load, peak, connection counts, last-activity) into the panel's frame title, reclaiming that row for the sparkline itself instead of duplicating stats already shown up top.
- Wrap every panel's top-border title (name plus any bundled stats/counts) in a single `┤ ... ├` bracket across the whole app, matching the bracket style already used for bottom-border key hints, instead of running the title as bare text against the border.
- Border-embedded key hints and title brackets now match the color of the border they're embedded in (dim `BORDER` or bright `BORDER_HI`) instead of always rendering dim, so a highlighted/focused panel doesn't show brighter corners next to dimmer hint brackets.
- Config Editor, and the single always-navigable panel on each of Minions/Events/Alerts/Pulse (the minion list, event list, alert list, and the Pulse dot-grid), now render their border in the bright `BORDER_HI` color like other focused/interactive panels already did, instead of the dim default.
- Whichever tab's panel is behind an open popup (a detail popup, Config Editor, or the help overlay) now dims from `BORDER_HI` back to `BORDER` while the popup has focus, instead of still reading as the focused element underneath it.
- Every "▲ more above" / "▼ more below" scroll indicator now matches the color of the border it's embedded in (dim or bright) instead of always rendering in a fixed dim color regardless of the panel's actual state.
- Events tab and the Event Sniffer popup now display oldest-at-top, newest-at-bottom (tail-style), matching Live Jobs' existing convention instead of Job Cache's opposite one; Job Cache is also reordered to match. Selection stays pinned to the newest entry while at the tail (like `tail -f`), but the instant you move away from it, following turns off so a busy master doesn't drag your selection to a different event/entry out from under you — pressing `End` (or scrolling back down to the last row) re-engages it.
- Event Sniffer's title, status, and column-header rows now stay pinned at the top of the popup while scrolling through captured events, instead of scrolling out of view along with the event list.

### Fixed
- Job Cache entries always showed the default `glob` target type in the detail popup instead of the job's actual one (compound, list, etc.) — the agent checked the same (lowercase, always-absent) `tgt_type` key twice instead of falling back from the capitalized `Target-type` key `jobs.list_jobs` actually returns.
- Event Sniffer popup reset its selection to the newest event on every single newly-captured event, regardless of what the user had selected — on an active master, this made it impossible to keep an event selected long enough to inspect it.
- Jobs tab: switching focus between the Live Jobs and Job Cache panels (`←`/`→`) stopped changing either panel's border color once any popup had ever been opened while on that tab — `popup_tab` is only set when a popup opens and never cleared when it closes, so the new focus-dimming check was reading it as "a popup is still open" forever after the first one, permanently dimming both panels regardless of which had actual keyboard focus.
- Event Sniffer popup never showed the "▼ more below" indicator even when its event/JSON list overflowed the visible area — it always passed a hardcoded "nothing more" flag instead of computing it from the actual scroll position and content height.
- Minion Detail popup's "Full Grains" section label was nearly unreadable — it shared the divider rule's very dark color instead of a proper text color.
- Rust TUI process memory grew unbounded on long sessions against a large fleet (observed ~80MB at start, 300MB+ after 90 minutes) — the inbound agent-message queue had no size cap, and the consumer throttled to the tick interval even during event bursts. The queue is now capped (oldest entries dropped in favor of freshest once full, with a `backlog dropped: N` indicator if ever hit), and the main loop polls at a fast cadence whenever the queue is non-empty so bursts drain in ~100ms slices instead of queuing indefinitely.
- Pulse view's Details panel misreported every minion as "Unknown" — the liveness check looked for a flat `<cachedir>/minions/<mid>.p` file, but Salt actually stores a per-minion directory (`<cachedir>/minions/<mid>/data.p`) — and could flap between Connected/Unknown because the more-frequent roster refresh was unconditionally overwriting the real liveness check's result. The roster refresh now preserves the existing `connected` value; only the liveness check sets it.
- ZMQ latency (`agent_ipc`/`job_ping` sources) tracked the refresh interval almost exactly instead of real round-trip time, because a reply sat unread in the queue until the next tick's `event::poll` happened to expire. The main loop now polls at a short cadence whenever a ping is outstanding, so replies are drained within ~100ms of arriving regardless of `tick_interval_ms`.
- `Last Seen` and job-cache start times could read frozen or offset by a fixed amount on masters whose local timezone isn't UTC — Salt's UTC `_stamp` field was being parsed as local time when converting to a Unix timestamp.
- Mouse-click row selection (Minions, Events, Alerts, Jobs, Job Cache) and Pulse minion-grid clicks selected the row/minion one below the one actually clicked — both hit-testing paths now use the panel's real rendered geometry instead of geometry hardcoded before later layout changes.
- Grains, Event Detail, and Job Detail return-content scrolling could scroll past the end with no way back, or need dozens of `PageUp` presses before any visible movement — all three now share the same viewport-aware max-scroll computation, with live "▲ more above"/"▼ more below" indicators. Event Detail also no longer requires pressing `Enter` before arrow/PgUp/PgDn scrolling works.
- Process role classification (Master/Worker/API/Syndic/Minion) now reads `/proc/<pid>/cmdline` instead of the truncated interpreter path in `/proc/<pid>/comm`, which never matched any role.
- `SaltProcess.cpu_percent` and `KeyInfo.mtime_epoch` are now real (computed from `/proc/<pid>/stat` deltas and read from PKI file mtimes) instead of hardcoded to `0`.
- Grains with nested values (interfaces, disks, `pythonpath`, etc.) are now pretty-printed as indented, type-colored JSON instead of a single-line Python `repr()` truncated at 500 chars.
- Admin mode can now be toggled (`a`) from inside the Minion Detail popup, matching its own hint; confirming/cancelling returns to the same popup instead of dropping back to the Minions list.
- Dot-mode (braille) sparklines now fill bottom-up like Bars mode instead of zigzagging between dot columns for an increasing value.
- Panel titles use the app's title color consistently everywhere (most views previously fell back to ratatui's dimmer default).
- Event Detail's `y`/`Y` copy distinction (readable vs. raw JSON) restored, matching the convention Job Detail already used.
- Minion Events popup: dim-row note ("dim = older than 5m") no longer clips off the right edge, its title bar no longer shows blank-looking fields for minions with no grains yet, and section-divider rules are sized to the popup's actual rendered width instead of one fixed length.
- Agent's `/proc/<pid>/stat` parser now locates the `comm` field by its closing paren instead of a fixed whitespace-split index, which previously misaligned RSS/utime/stime (and inflated the Dashboard's memory figures) for any process whose `comm` contains a space.

### Removed
- Debug/diagnostic logging left over from development: the unbounded `/tmp/salt-radar-agent-ndjson.log`, and the temporary `/tmp/.salt-radar-memdiag-<pid>.log` RSS/collection-size log used to track down the memory-growth fix above — both removed once no longer needed, the underlying fixes stayed.

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
