# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0-rc14] - 2026-08-30

### Changed
- Config Editor's `graph_order` field is no longer hand-typed free text — editing it now expands into a reorderable vertical list (`↑↓` selects an item, `+`/`-` moves it up/down, `Enter` confirms, `Esc` reverts), removing the chance of a typo or dropped panel name that free-text editing allowed.

### Fixed
- `graph_order`'s new reorderable-list editor didn't actually save: `Save & Restart` silently discarded the edited order and fell back to the hardcoded default, because the field-extraction step that builds the saved config only recognized the old free-text/choice field kinds — the new list-editor kind wasn't in that list.
- Config Editor's border hints stayed static ("Esc/q:close ↑↓:move ...") while reordering `graph_order`, never mentioning `+`/`-`. They now switch to the reorder-mode key set (`Esc:cancel`, `↑↓:select`, `+/-:move item`, `Enter:confirm`) while that list is open, same as Job Detail's hints already change with its focus/scroll state — replacing a separate inline text line under the list that duplicated the same information.

### Fixed
- Refresh interval (`+`/`-`) only worked from the Dashboard tab, even though its hint is shown globally in the title frame and the help overlay never said it was Dashboard-only either — now works from any tab.
- Mouse mode (`F2`) couldn't be toggled while any popup was open — added to the same "works even with a popup open" key set as contrast/tab-switching/`F3`.
- Minion Events popup (and Minion/Event/Alert Detail, Event Sniffer): section-divider rules under the title/column headers were a fixed-length string that only ever matched one particular popup width, so they looked arbitrarily short or misaligned on anything else — now sized from the popup's actual rendered width.
- Minion Events popup didn't explain why some rows are dimmed — added "(dim = older than 5m)" next to the column header, matching the actual hardcoded threshold.
- Events tab list: Local/UTC timestamp color now matches the refresh-interval value color in the title frame (`TEXT_TITLE`) instead of a dim gray that made it hard to distinguish from genuinely stale/dim content.
- Job Cache's "Request timed out — press r to retry" message didn't quote the key, unlike every other inline key mention.
- Minion Events popup: the "(dim = older than 5m)" note was appended directly onto the already width-filled column-header row, so on most terminals it was clipped down to a single stray `(` at the right edge. Moved to the block's own border (right-aligned, alongside the shortcut hints) where it can't overflow.
- Border-hint spacing was inconsistent three different ways depending which helper built it: `border_info` padded both sides (`┤ text ├`), `border_hints`/`value_hint` padded only the trailing side (`┤key:label ├`). Standardized on padding both sides everywhere (`┤ key:label ├`), matching the "more above"/"more below" indicators which already did.
- Minion Events popup's title bar showed blank-looking runs like `[connected,    v]` when grains hadn't been retrieved for that minion yet (empty OS/version fields still got formatted into the template). Now only includes the fields actually known.
- `F1` is now bound to help, aliasing `?` (the conventional terminal/GUI help key, previously unbound). While at it, help (`?`/`F1`) now also works while a popup is open, same as `F2`/`F3`/`c`/tab-switching already did.
- Title frame's centered clock used its own one-off dim color instead of matching the refresh-interval value next to it — now both use the same color.
- Popup shortcut display was inconsistent across the app: Minion Detail, Event Detail, Job Detail, Alert Detail, Event Sniffer, and Minion Events each used a different hand-written bracket-text footer, Admin Confirm used the old yellow-key bottom bar, and Config Editor showed no shortcuts at all. Every popup now uses the same btop-style `┤key:label├` border hints as the rest of the app.
- Admin mode couldn't be toggled from inside the Minion Detail popup despite its own hint advertising `a` for it — the global admin-toggle handler is unreachable while any popup is open, and the popup had no handler of its own. Confirming/cancelling now returns to the same Minion Detail popup instead of dropping back to the Minions list.
- Grains with nested values (interfaces, disks, `pythonpath`, etc.) were flattened into a single-line Python `repr()` truncated at 500 chars. The agent now pretty-prints them as indented JSON (cap raised to 2000 chars), and the TUI renders each line indented under its key with lightweight type coloring instead of one unreadable line.
- Grains scroll: `End`/`PageDown` were unbounded (`End` set the raw counter to `usize::MAX/2`), and the render-time clamp treated every grain as exactly one line, so scrolling to the end pinned the *last* grain to the *top* of the frame with blank space below it, and `PageUp` needed dozens of presses before any visible movement. Fixed with a height-aware max-scroll computed from actual rendered line counts, plus a live `▲ more above`/`▼ more below` border indicator replacing the old in-content "N items (X%)" line.
- Event Detail required pressing `Enter` before arrow keys/`PgUp`/`PgDn` would scroll at all — that gate only makes sense for Job Detail's list-vs-content disambiguation and served no purpose here, so it's gone; scrolling now works immediately. Also fixed the identical "scrolls past the end, then PageUp does nothing for a while" bug grains had, via the same viewport-aware max written back from render each frame.
- Job Detail's return-content preview had the same unbounded-scroll bug — `content_scroll_max` was written but never actually used to clamp anything, and `End` was still `usize::MAX/2` — so `End` could scroll content off the bottom of the panel. Fixed the same way, with the same `▲`/`▼` more-content indicators.
- `y`/`Y` in Event Detail did the exact same thing (copy tag+timestamp+pretty-JSON either way) despite the hint claiming `Y` copies JSON only — restored the intended distinction (`y` = readable copy, `Y` = raw JSON), matching the convention Job Detail already uses.
- `Last Seen` in Minion Detail could appear frozen at a fixed offset (e.g. "2h ago") regardless of actual recent activity, whenever the master's local timezone isn't UTC. The agent's event-timestamp parser (`_parse_stamp`) treated Salt's UTC `_stamp` field as local time when converting to a Unix timestamp, silently shifting every event's stored time by the local UTC offset. Fixed to explicitly tag the parsed time as UTC — matching the neighboring `_jid_to_epoch`, which already did this correctly — which also fixes the same bug in job-cache start-time fallback parsing.
- Minion Detail popup closed on `Enter` in addition to `Esc`/`q`, unlike every other popup and undocumented anywhere — removed.

### Changed
- Documented every `PgUp`/`PgDn`/`Home`/`End` binding that was already functional but not shown anywhere: Minions, Events, Alerts, Jobs (Live + Cache), Config Editor, Minion Detail grains, Event Sniffer, Minion Events popup, Job Detail (all panels/modes), and Pulse (verified via code that they're wired through the shared list-navigation path before adding the hint).
- Consistency pass across popup/panel hints: merged `Esc`/`q` where both did the same thing but only one was shown (Job Detail, Config Editor); fixed `j/k`/"arrows" labels to the `↑↓` glyph used everywhere else (Job Detail).
- Pulse tab's Minion Pulse panel now shows its own navigation hints (`↑↓:row`, `←→:col`, `PgUp/PgDn:jump 10`, `Home/End:first/last`) in its border, matching every other panel.
- Admin-mode key was shown as `A` in a few spots (grains popup hint and body text, fleet-scan prompts) despite only lowercase `a` being used everywhere else — normalized, and every inline key mention in body text is now wrapped in quotes (e.g. `press 's'`) for readability.
- Help overlay: new "Minion Detail" section explaining how `Last Seen` is calculated. README: documented Event Detail's `y`/`Y` copy distinction (previously only Job Detail's was documented).

## [0.6.0-rc13] - 2026-08-30

### Changed
- Shortcut hints moved out of the bottom key-hint bar and into the border of the panel each one affects (btop-style: `┤key:label├` embedded in the panel's own frame), so hints stay next to the behavior they switch instead of a separate legend — the title frame carries the global keys (`?`/`F2`/`F3`/`a`/`q`), each Dashboard sparkline panel carries its own `F4`-`F8` hide toggle plus a shared `m` bars/dots toggle, and Minions/Events/Jobs/Alerts carry their own filter/detail/action keys. The bottom bar is now status-only (interval, agent connectivity, admin/contrast state, selection position). Dropped the redundant `h`/`H` help aliases — `?` only, matching btop's own help key
- Refresh interval hint now matches btop's own adjustable-value style — `┤-2000ms+├` with the live interval value shown between the `-`/`+` keys — and moved to the title frame's top line, left of the `alerts:` count, instead of a static `+/-:interval` label at the bottom. The bottom status bar's redundant `ref: X.Xs` text is gone, and the `alerts:`/`backlog dropped:` indicators are now bracket-wrapped (`┤...├`) to match the rest of the border-hint style instead of floating unbracketed on the border
- Pulse tab now has a direct jump key, `6` (was reachable only via `Tab`/mouse) — the tab bar numbers it like the other five tabs instead of showing a plain `◉` marker
- High-contrast toggle (`c`) is back as a title-frame hint (`┤c:contrast on/off├`) — it was dropped entirely when the bottom key-hint bar was trimmed down to status-only
- Config Editor (`F3`) now shows `┤saved to: <resolved path>├` on its bottom border, so it's visible which file settings actually get written to
- Config file lookup now falls back to `$XDG_CONFIG_HOME/salt-radar/salt-radar.toml` (usually `~/.config/salt-radar/salt-radar.toml`), creating the directory on first save, when `./salt-radar.toml` isn't present in the launch directory. Previously it only ever looked at `./salt-radar.toml` relative to whatever directory the binary happened to be launched from, which silently dropped a fresh config into e.g. `~/Downloads/salt-radar.toml` if that's where you ran it from. A `./salt-radar.toml` still takes priority when present, so an existing per-directory config keeps working unchanged

## [0.6.0-rc12] - 2026-08-29

### Added
- Help overlay: new "Pulse Status" section explaining what `Connected`/`Unknown`/`Timed out` mean in the Pulse Details panel and where each comes from (#52)
- Dashboard: `F4`-`F8` toggle the CPU/Memory/ZMQ/Events/Minion-activity graphs on and off directly (`F4`=CPU, `F5`=Memory, `F6`=ZMQ, `F7`=Events, `F8`=Minion activity), without going through the Config Editor — `1`-`5` were already taken for tab switching. The Config Editor's `show_*_graph` entries are unchanged; the layout is now also saved to `salt-radar.toml` automatically on quit, not just via an explicit Config Editor save (#50)

### Changed
- Sparkline panels (CPU/Memory/ZMQ/Events/Minion activity) now default to Dots instead of Bars on first run (#49)

### Added
- Dashboard Event Types panel: `minion/refresh/*` events (grains/pillar/modules/beacon-list refresh traffic) now break out into their own `Refresh` category instead of falling into `Other`

### Fixed
- Mouse-click row selection in Minions, Events, Alerts, Jobs, and Job Cache selected the row below the one clicked. The shared click handler assumed panel content started one row earlier than it actually does — stale from before the title/tab bar was merged into a single 3-row bordered frame (#32) — so every list panel's hit-testing was off by one row
- Pulse view: clicking a minion in the Minion Pulse dot grid selected the minion one row below the one clicked. Mouse hit-testing re-derived the grid's screen geometry with hardcoded numbers that assumed the layout from before the title frame and Legend panel existed, instead of using the grid's actual rendered area — it now reads the real area recorded during draw, so it can't drift out of sync with the layout again
- Agent event-bus listener (`_event_loop`) fell back to a guessed `<cachedir>/master` socket path whenever `sock_dir` wasn't in `opts`, which resolves to the wrong directory (real default is `/var/run/salt/master`, unrelated to `cachedir`). In practice unreachable — `opts` always comes from `salt.config.master_config()`, which resolves `sock_dir` from Salt's own defaults — but now reads `opts["sock_dir"]` directly instead of carrying a wrong fallback
- Pulse view's Details panel showed every minion as `Unknown` regardless of real status. The liveness check (`alive_check`) marks a minion "up" if it has a cache entry on disk, but checked for a flat `<cachedir>/minions/<mid>.p` file — Salt actually stores a directory per minion containing `data.p` (`<cachedir>/minions/<mid>/data.p`), so the existence check always failed and no minion was ever reported up
- Pulse view's Details panel flapped a minion's status between `Connected` and `Unknown` with no real change on the minion. `minion_cache_list` (roster refresh, every `dashboard_secs` ticks — default 2) unconditionally forced `connected = true` for every minion in the cache, while `alive_check` (the actual liveness probe, every 5 ticks) set it based on a real check — since the cache refresh runs more often, it kept clobbering alive_check's result back to `true` until the next real check corrected it down again, repeating in a cycle. The cache refresh now preserves the existing `connected` value instead of overwriting it; only `alive_check` sets it
- Dashboard ZMQ latency (`agent_ipc` and `job_ping` sources) tracked `tick_interval_ms` almost exactly instead of real round-trip time — changing the refresh interval changed the reported latency in lockstep with no change on the master itself. The `zmq_ping`/`job_ping` reply was arriving within milliseconds but sitting unread in the agent message queue until the main loop's `event::poll(timeout)` happened to expire at the next tick, and since latency was measured from send to *drain* rather than send to actual receive, that idle wait was counted as latency. The main loop now polls at a short cadence whenever a ping is outstanding, so the reply is drained within ~100ms of arriving regardless of `tick_interval_ms`
- Pulse view's Legend panel reserved 8 rows (2 for its border + 6 content rows) for only 5 entries, leaving a blank line at the bottom. Trimmed to 7
- Dashboard Services panel: the `Status` dot was left-aligned in its column, so it sat directly against the `Unit` column with no visible gap for long unit names. Right-aligned so it lines up under the last `s` of the `Status` header instead

### Removed
- Dropped the memory diagnostics log added while tracking down #48 (RSS + per-collection size logging to `/tmp/.salt-radar-memdiag-<pid>.log`) now that the growth is understood and bounded — the `agent_rx` cap/backlog-drop indicator from rc11 stays

## [0.6.0-rc11] - 2026-08-29

### Fixed
- The Rust TUI process's memory grew without bound over long sessions on large fleets (observed: ~80MB at start, 300MB+ after 90 minutes), while the sibling Python agent stayed flat. The inbound `agent_rx` queue fed by the agent's NDJSON reader thread had no size cap, while the consumer (`drain_agent_messages`) throttled itself to 200 messages per call and, while idle, only ran once per `tick_interval_ms` (2s default) — a ceiling of roughly 100 msgs/sec sustained. A broadcast job against a large fleet (a scheduled highstate, `mine.update`, `test.ping`) fires thousands of events in a burst, easily exceeding that ceiling, and the excess just queued in memory forever. The queue is now capped (dropping oldest in favor of freshest once full, with a `backlog dropped: N` indicator in the title bar if it's ever hit), and the main loop now polls at the fast cadence whenever the queue is non-empty so bursts drain in ~100ms slices instead of waiting a full tick — this alone did not fully resolve the growth on further testing, so growth continues to be investigated (#48)
- Added a built-in memory diagnostics log (`/tmp/.salt-radar-memdiag-<pid>.log`) written every ~60s with process RSS alongside the length/capacity of every long-lived collection (events, jobs, minion_info, alerts, job-latency windows, service trends, the agent_rx queue, etc.), to distinguish an actual growing Rust-level collection from allocator-level RSS growth (glibc not returning freed pages) when tracking down #48 further

## [0.6.0-rc10] - 2026-08-29

### Added
- Per-service overrides for the "Service memory high" alert (Rule 13): `salt-master`/`salt-minion`/`salt-api`/`salt-syndic` can each set their own absolute (`service_mem_abs_threshold_*_mb`) and percent-of-total (`service_mem_pct_threshold_*`) threshold instead of sharing one global pair — `salt-master` legitimately runs with a much larger working set than the others, so a single number was either too loose for the master or too aggressive for everything else. Each override defaults to 0 ("no override, fall back to the global default"), editable via the Config Editor or `salt-radar.toml` (#46)
- Minions list: a `●` marker in the ID gutter for minions that checked in recently — the "fresh" threshold is derived from the fleet's own median check-in age (clamped 5s-60s) instead of a fixed number, so it stays meaningful whether minions ping every few seconds or every few minutes, and a fleet that goes fully idle stops showing anyone as fresh instead of the threshold drifting up in lockstep with it (#43)
- Dashboard: the bottom sparkline panels (ZMQ / Events / Minion Activity, plus two new ones — CPU and Memory utilization history) can each be shown or hidden independently via `show_*_graph` in the Config Editor's `[dashboard]` section, and their stacking order is configurable via `graph_order` (default `cpu,mem,zmq,events,activity`) — a comma-separated list of panel keys; unrecognized/duplicate keys are ignored and any panel missing from the list is appended after the ones named. CPU/Memory default off since enabling them changes the existing layout's panel count (#41)
- Pulse view: the freshest recency tier (return < 5s ago) now renders as a `•` bullet instead of the same small `·` middle-dot every other tier uses, since color alone on a single terminal cell was too subtle to reliably tell the brightest tier apart from the rest; a new Legend panel below the dot grid spells out what each glyph/color combination means (`•` bright green = <5s, `·` green = <5min, `·` orange = <10min, `·` dim gray = stale/never, `·` red = timed out)

### Fixed
- Dashboard ZMQ panel: the `ret` port's `:` was pulled into its own standalone dim-gray span while `pub`'s and `api`'s colons were baked into their port's own colored span, so two of the three port colons matched their port number's color and the middle one didn't. `ret`'s `:` now folds into the same span as its port digits (#47)
- Config Editor: the high-contrast toggle (`c`/`C`) was swallowed while navigating between fields — the editor intercepted every keystroke early as a "global overlay" before the universal contrast-toggle handler ever ran, and never grew its own `c`/`C` arm to compensate. It now toggles contrast in navigation mode (matching the rest of the app) while still typing a literal `c` when editing a field's text (#45)
- Config Editor: on a small terminal, the field list rendered as one unscrolled `Paragraph` from the top with no scroll offset — navigating past the visible area moved the selection off-screen with no way to tell it could still be reached. The popup now tracks the selected field's line index and scrolls to keep it in view, the same windowing approach every other scrollable panel already uses (#44)
- Dashboard Services panel: the `Memory` column padded only the digit portion to a 3-digit budget (`{:>3} MB`), so any service at or above 1000 MB (routine for `salt-master`'s Worker/ReqServer/EventPublisher children) overflowed its column and threw every field after it out of alignment, by a different amount per row depending on that row's digit count. The combined `"<value> MB"` string is now right-aligned as one field, sized for up to 5 digits (#40)
- Alerts panel: the header's spacers before `Rule` and `Description` were hand-typed magic-width strings that didn't match the data rows' actual field widths, so `Rule`/`Description` never lined up with their columns; separately, the `Rule` field's fixed 16-char width was narrower than several real rule names (e.g. `Service memory growth` at 21 chars), which — since `{:<16}` doesn't truncate — ran the rule text straight into `Description` with zero separating space for any alert using one of those longer names. Header and data rows now share the same width constants, and `Rule` is widened to fit the longest current name (#42)
- Dashboard: CPU/Memory sparklines rendered as full-height bars in Bars display mode even at a genuinely low value (e.g. 3%) — `Sparkline` auto-scales to the *visible window's own max* when no explicit ceiling is set, and unlike the other sparkline panels (which normalize against their own rolling peak before handing off data), CPU/Memory pass a raw absolute percentage, so a small value with nothing higher currently on screen filled the whole bar. Bars mode now renders on the same fixed 0-100 scale Dots mode already used

## [0.6.0-rc9] - 2026-08-28

### Added
- Refreshed README screenshots for every tab against the updated UI
- Config Editor: fields with a fixed, small set of valid values (currently `zmq_latency_source`) now edit via an inline picker — Enter opens a list of the valid options with the current one pre-selected, ↑↓ moves, Enter confirms — instead of free-text entry where a typo silently fell back to default behavior (#37)
- Scrollable list panels (Minions, Live Jobs, Job Cache, Events, Alerts) now show a "▲ more above" hint in the title and a "▼ more below" hint on the bottom border when the windowed list has content scrolled out of view in that direction, via a shared `scroll_window()`/`list_block()` helper instead of each view silently clipping with no way to tell there's more to scroll to (#38)
- New "Service memory high" alert rule (Rule 13) flags a unit whose current RSS crosses a configurable absolute (`service_mem_abs_threshold_mb`, default 2048 MB) or percent-of-total-system-memory (`service_mem_pct_threshold`, default 8%) threshold, independent of the existing growth-rate alert — a slow, low-slope leak that's been running for weeks would never trip the rate check alone. Either threshold can be disabled by setting it to 0. Configurable via the `[alerts]` section or the in-app Config Editor (#29)

### Fixed
- Config Editor's label column was padded to a hardcoded 22 chars, which several labels (e.g. `service_mem_growth_mb_per_hour` at 31 chars) already ran past, jamming the value straight against the label with no separating space. Width is now derived from the longest label actually present
- ZMQ panel's `job_ping` latency source always read a flat `0ms` — the round-trip clock started only after the agent's response (carrying the dispatched job's JID) arrived, but the actual dispatch happens *inside* that same response's round trip, so on a fast/local fleet the job could already have returned before the clock even started. The clock now starts on the Rust side before dispatch is requested; the agent also dispatches via `LocalClient.cmd_async()` directly instead of shelling out to the `salt` CLI, so per-call interpreter/CLI startup no longer pollutes the measurement
- Job Pulse `p50/p95` completion latency stuck at `0.0s/0.0s` for jobs dispatched against targets whose minion list isn't resolved at dispatch time (e.g. some glob/compound matchers, async batches) — `target_count` stayed 0 forever so those jobs never met the `returned >= target_count` finalize condition. Jobs with an unknown target count now finalize on first return instead of never (#25)
- Check-in Age `p50/p90/p99` on the Minions view could read stuck at `0s` on large fleets — the agent's per-minion grains-cache enrichment loop (`minion_cache_list`) ran under a single `try/except` covering the whole batch, so one bad cache lookup aborted enrichment for every minion after it, leaving them at `last_seen: 0` and silently excluded from the percentile population. Each minion is now looked up independently so one failure no longer drops the rest of the fleet (#26)
- Dashboard Services panel: data rows were one column narrower than the header in the `Status` field, shifting every column after it (`State`/`Flp`/`Memory`/`T`/`Uptime`) out from under its label; `Memory` and `Uptime` also right-aligned their combined value+unit string as one blob, so the unit suffix landed in a different column depending on the value's digit count. Number and unit now render in their own fixed-width sub-fields (#28)
- Dashboard Services/Key Status/Event Types row: rebalanced the even 3-way split to 2/1/2 since Key Status only ever needs a handful of fixed-format count rows, leaving Services and Event Types cramped at equal thirds; Event Types' panel title now renders in the same title color as its row-mates instead of ratatui's dimmer default (#27)
- Panel titles were styled ad-hoc per call site, so most views (Minions, Events, Jobs, Alerts, Pulse, Config Editor, and the Returns/Preview detail popups) rendered their title in whatever fallback color was left over from the block's border styling instead of the app's title color, while only a few Dashboard panels applied it explicitly. Added a shared `titled_block()` helper and moved every plain-string `.title(...)` call site onto it, so panel titles are now the same color everywhere (#33)
- Dot-mode (braille) sparklines lit the top-left dot first for a minimal value instead of filling from the bottom like Bars mode — the fill loop used the bit position directly as the braille dot index instead of an explicit bottom-to-top fill order, so increasing values actually zigzagged between the left/right dot columns rather than rising monotonically. Sparklines now fill dot-by-dot from the bottom row up (#30)
- Title bar and tab menu merged into a single bordered frame with the app name/version/master version, a centered clock, and the alert count embedded directly in the top border (left/center/right), and the tab list sharing the frame's one content row with host/uptime info. Replaces the previous single-row `Borders::ALL` title bar, which had no room to draw a real box and rendered as a dangling top edge (#32); also removes the duplicate app version, master version, and hostname that the per-tab Dashboard header used to repeat one row below it — that header is gone, and its unique remaining info (radar/host uptime, now both individually labeled instead of one shared ambiguous `uptime:` label with an understated host figure) moved into the frame, visible on every tab instead of only Dashboard (#34)
- Minions → Fleet Overview: the OS Distribution/Salt Version/Architecture tables padded the name column to a hardcoded 22 chars, so any name longer than that (e.g. a long `osfinger` string) overflowed the field and threw the count/bar/% columns out of alignment for that row. The name column width is now derived from the longest name actually present across the three tables (clamped to a sane min/max), so bars and counts line up regardless of name length (#31)
- Dashboard Event Types panel: the `Auth` row's bar was sized using the same width budget as every other row, then had a `(X.X/s)` rate suffix appended on top afterward with no reserved space for it — once the auth rate picked up, the row could overflow past the panel's right edge. Moved the rate into the panel's title (`Event Types  auth: X.X/s`), consistent with how Services/Key Status/Events already surface a summary stat in their title instead of a row, removing the overflow risk entirely (#35)
- Jobs → Job Cache: the fixed-width `JID`/`Fun`/`Target`/`User`/`Started` columns needed ~106 columns before `Started` even started rendering, so it silently clipped off the right edge at common terminal widths — unlike `Fun`/`Target`, which already truncate with `..` past their limit, `Started` had no fallback at all. Reordered to `JID`/`User`/`Started`/`Target`/`Fun` so a narrow panel clips the columns that already handle overflow instead of the one that doesn't (#39)
- Dashboard's Minion Key Status panel showed a bare `churn:X/min` KPI with no explanation anywhere in the app of what it measures (new pending PKI keys registered in the last 60s). Relabeled to `new pending:X/min` and added a short definition to the in-app Help overlay (`?`/`h`) (#36)

### Changed
- Dashboard's "ZMQ" latency figure was a synthetic local IPC ping between the Rust TUI and its own Python helper subprocess — it never touched Salt's actual ZeroMQ transport or the master's real health, despite the panel implying otherwise. The measurement source is now configurable via `zmq_latency_source` in a new `[dashboard]` config section (also editable in the Config Editor): `agent_ipc` (default — the original local ping, relabeled "agent IPC" instead of implying it reflects Salt transport health), `job_ping` (periodically dispatches a real `test.ping` and times dispatch → first return over the actual event bus, throttled well below tick cadence since it puts real traffic on the bus), or `master_stats` (listens for Salt's own `salt/stats/<worker>` events — weighted-mean command execution time across workers — requires `master_stats: True` in the master's own config, which salt-radar can't set itself). Whichever source is active, the panel reads an honest "n/a" instead of a number until it has a real sample. The pub/ret/api connection-count part of the panel, which was already real, is unchanged (#37)

## [0.6.0-rc8] - 2026-08-26

### Added
- Service restart/flap count (rolling 24h) and memory growth trend detection on the Dashboard Services panel: new `Flp` (flap count) and `T` (memory trend arrow) columns per unit, plus two new alert rules — "Service flapping" (`service_flap_threshold_24h`) and "Service memory growth" (`service_mem_growth_mb_per_hour`) — the latter using a linear-regression slope over sampled memory history rather than a static high-water-mark, and requiring the most recent portion of the window to still be trending upward so growth that plateaued (e.g. warm-up/caching) doesn't alert (#24)

## [0.6.0-rc7] - 2026-08-25

### Added
- Screenshots for every tab in README.md

### Fixed
- Agent's `/proc/<pid>/stat` parser was splitting on whitespace with fixed field indices, which silently misaligned every field (RSS, utime, stime) for any process whose `comm` contains a space — inflated the Dashboard's Services memory panel by orders of magnitude. Now locates the `comm` field by its closing paren instead of a fixed-index split.
- Service memory in the Dashboard `Unit` table now reads the systemd unit's cgroup `MemoryCurrent` (covers the whole process tree, shared pages counted once) instead of just the main process's own `VmRSS` (missed all forked Worker/ReqServer/EventPublisher children). Falls back to `VmRSS` of the main process if cgroup accounting is unavailable.
- Salt processes are now classified with a `Minion` role (was falling into `Unknown`/`?` for any master that also runs a co-located minion). The Services panel's per-role memory line now wraps across multiple rows instead of being hard-clipped mid-word when it doesn't fit on one line, and the panel now reserves the right amount of height for it instead of pushing service rows off the bottom.

## [0.6.0-rc6] - 2026-08-23

### Added
- MinionEvents popup: filter items by keyword with `/` key (matches tag, JID, function, description)
- `minion_start` event tag recognized by both EventBreakdown and MinionEvents popup (was falling into "other" bucket)
- Job Pulse panel on Dashboard now shows rolling job-health KPIs: return success rate, stuck/in-flight job count, completion latency (p50/p95), and partial-return rate
- "Stuck job" alert rule: fires when a dispatched job hasn't fully returned within `job_straggler_secs`
- Worker pool saturation gauge on Dashboard Services panel (`Workers X% busy`), from real per-process CPU sampling
- Auth event rate shown on Dashboard Event Types panel (events/sec)
- "Reauth storm" alert rule: fires when auth rate and worker saturation are elevated together (`auth_storm_rate_per_sec`, `worker_saturation_pct`)
- Pending-key churn rate shown on Dashboard Minion Key Status panel and as a new alert rule (`pending_key_churn_per_min`)
- Check-in age percentiles (p50/p90/p99) on the Minions tab Fleet Overview panel

### Fixed
- MinionEvents popup: selection highlight now stops at content width instead of filling full popup width
- MinionEvents popup: selection highlight width limited to text content using per-span background styling
- `last_zmq_return` no longer grows unbounded on minion churn — stale entries are pruned on each roster refresh
- Roster reconciliation on `minion_cache_list` refresh and per-event grains harvest now uses an ID-indexed lookup instead of an O(n) scan per minion/event
- Minion roster is only re-sorted when a change actually affects ordering, instead of every tick
- Remove unbounded debug logging to `/tmp/salt-radar-agent-ndjson.log` in the agent (was leftover dev instrumentation with no rotation/cap)
- `self.jobs` no longer grows unbounded — `JobEntry.age_seconds` was never actually updated after creation, so the age-out cleanup never removed anything
- Process role classification (Master/Worker/API/Syndic/...) now reads `/proc/<pid>/cmdline` instead of `/proc/<pid>/comm`, which was always the truncated interpreter path and never matched any role — `SaltProcess.role` was effectively always `Unknown`
- `SaltProcess.cpu_percent` is now real (computed from `/proc/<pid>/stat` utime/stime deltas between polls) instead of a hardcoded `0.0`
- `KeyInfo.mtime_epoch` is now real (read from the PKI key file's mtime) instead of a hardcoded `0`, for both the `salt.key.Key` and `salt-key -L` fallback paths

## [0.6.0-rc4] - 2026-07-07

### Added
- Per-minion event/return history view on Minions tab: press `r` to open a filtered popup showing all events, returns, starts, beacons, cache jobs, and alerts for the selected minion
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
- Ghost minions: `cmd_minion_cache_list` now uses `salt-key -L` as the sole source for the minion roster; cache entries (which can contain stale/removed minions) only enrich existing entries
- Agent system info collector: add `/proc/net/dev` parsing to `cmd_get_system_info` so Dashboard network panel shows actual byte counts and transfer rates
- MinionEvents popup: auto-load job cache data when popup opens instead of requiring manual Jobs tab visit
- MinionEvents popup: use distinct cyan color for cache job JID column instead of dim gray that merged with stale entries
- MinionEvents popup: apply uniform TEXT_DIM to all columns (not just time) when event is older than 5 minutes
- MinionEvents popup: fix minion connected/disconnected flapping in header by removing `connected = true` from event processing, making alive_check the sole source
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
