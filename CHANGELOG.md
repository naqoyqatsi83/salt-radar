# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Tell module settings from typos in the Master Config tab (#185): keys that aren't salt-master's own but that Salt reads are a new `module` kind, named with what reads them: salt-api (`rest_cherrypy`, `rest_tornado`, `rest_wsgi`, `api_logfile`, `rest_timeout`), any module Salt ships, found in Salt's own sources (`auth.ldap.*` with the backend's defaults, `cache.redis.*`, `mysql.*`, `vault`), or a valid minion, syndic or salt-ssh option. Modules are found where the master's loader finds them, installed Salt extensions and synced custom modules included, and a dotted option counts only when the module's source names it (`redis.host` on 3008, whose redis cache reads `cache.redis.host`, is caught with that suggestion); a setting for a module Salt 3008 moved out to an extension says it needs the extension. Options inside a salt-api section are checked too: one salt-api doesn't read is flagged and counted as not core, with the option it's probably meant to be (`colect_stats` → `collect_stats`), and the documented `socket_queue_size`, which salt-api never reads, says so. "Not core" is left for keys nothing in Salt reads

### Fixed

- Minions `Proto` column (#183): right-aligned like the other short columns, and `—` for a minion that timed out, as OS, Salt and Arch already were
- salt-api port never detected (#184): the Dashboard's ZMQ line now shows salt-api connection counts, the port read from the merged master config instead of hand-parsing the files; `/login` and `/stats` also follow salt-api's `host` and `root_prefix` instead of always using `127.0.0.1` at `/`

## [0.9.0-rc3] - 2026-10-15

### Added

- Browse the master's configuration in a new Master Config tab (`7`, #178): every setting on one line, the ones set in a file first, in Salt's load order (`/etc/salt/master`, `master.d/*.conf`, `include:`) with the file and line that sets each, then the values Salt worked out at startup, then the defaults, greyed out. The right panel shows the file a setting comes from with its block marked, and the files it overrides; `Enter` opens the full value next to Salt's default, `y`/`s` copy or save it, `/` filters and `r` reads the files again. Keys Salt doesn't know are flagged "not core" (a module's own option or a typo Salt ignores; `worker_thread` names `worker_threads`), and a ↻ marks each setting set in a file that changed after salt-master started, as its value may change on restart (the title counts them, `/restart` lists them). Credential-looking values are redacted by the agent, in the list, the files and copies, so they never reach the TUI. Its divider moves with `Ctrl+←`/`Ctrl+→` too (`master_split_pct`)
- Show how duplicate master settings combine, and the config files Salt rejected (#182): when several files set a setting, the Master Config tab now says what each does, as Salt does it: a `master.d` file replaces the main file's whole value, naming the nested keys that are dropped (`rest_cherrypy: {port: 9000}` loses `collect_stats`), dicts from `master.d` files merge and lists add up, other values are overridden. The setting popup shows each file's own value before the result. A file Salt can't parse (the same key twice is enough) is skipped whole with only a log line; the tab now flags it, lists its settings as ignored with Salt's reason (even when the file isn't valid YAML at all) and marks the line at fault, and a new "Config file rejected" alert fires, as the configuration is now also read at start and with the config refresh
- Tell minions on auth protocol v0 from v2 (#181): minions from before Salt's March 2022 security releases (2014.x–3001.x, 3002.0–3002.7, 3003.0–3003.3, 3004.0) send no protocol version and get no replay protection, but `Proto` showed them as v2. `Proto` now reads v0 (red), v2 (yellow) or v3, and a "No replay protection" alert lists the v0 minions next to the "Legacy auth protocol" one for v2

### Changed

- Pulse moves to `8`, staying the last tab after the new Master Config tab (`7`)
- The Salt major alert only lists minions on a newer major than the master, as "Minion newer than master" (#181): a newer master supports older minions, and the protocol alerts already cover the ones too old for it, so a fleet of 3000–3005 minions on a 3006 master no longer lands there too

### Fixed

- Tab bar clicks one column off: clicking a tab's last letter missed it, and the frame border left of Dashboard selected it; the clickable ranges now come from the labels as drawn

## [0.9.0-rc2] - 2026-10-14

### Added

- Move the divider between a tab's two panels with `Ctrl+←`/`Ctrl+→` (#179), 1% a step (hold the keys to move further) between 20% and 80%, on Minions, Events, Jobs, API and Pulse, to give a long list or a wide preview more room. Each tab keeps its own split, saved the moment it moves as `minions_split_pct`, `events_split_pct`, `jobs_split_pct`, `api_split_pct` and `pulse_split_pct` in `[ui]` (also in the Config Editor); mouse clicks follow the divider
- Show how slowly each minion answers compared with its peers (#176): the optional Minions `Lag` column (`lag`) is the median, over the minion's last 50 jobs, of its return time divided by the job's median, so a consistently slow minion stands out whatever the jobs were (1.0x typical, yellow from 1.5x, red from 3x; sortable worst first). A job is rated once every target returned or it passed `job_straggler_secs`, and later returns against that median; return times under 0.5 s count as 0.5 s so ping jitter doesn't read as lag, and jobs where both the minion and the median took under 0.5 s aren't rated, so mostly fast jobs don't flatten every minion to 1.0x. The optional `Ping` column (`ping`) is the time to the minion's latest `test.ping` return, and the activity popup's summary line adds the lag median and p95 and the last ping
- Warn about minions whose Salt version doesn't fit the master (#177): the optional Minions `Proto` column (`proto`) shows each minion's auth protocol, worked out from its Salt version (v3 from 3006.12, 3007.4 and 3008; v2 before, in yellow). A new "Legacy auth protocol" alert lists minions still on v2, which masters from 3006.17, 3007.9 and 3008 reject by default (`minimum_auth_version: 3`), as a warning, or critical once this master enforces it; "Salt major mismatch" lists minions on another major than the master (3008 minions on a 3006 master). Both replace the "Legacy protocol" rule, which waited for event tags Salt doesn't send

### Fixed

- Minions OS, Salt and Arch empty until a fleet scan (#180): the agent never read the master's minion data cache, which holds every minion's grains (it asked for the wrong cache entry, and a missing import made it give up silently). The columns now fill in at start; the pillar compiled for salt-api stats credentials also gets the master's real grains. Last Seen and the Stale minion alert still go by returns seen live, as the cache is only rewritten when a minion's pillar is compiled, and the Guide no longer says otherwise

## [0.9.0-rc1] - 2026-10-13

### Added

- Confirm before quitting (#173): `q` now asks "Quit salt-radar?" and only `y`/`Enter` quits, while any other key, `q` included, cancels. Extra `q` presses meant for a popup that lag already closed used to quit the app, since queued keys are handled in one batch before the next redraw. Turn it off with `confirm_quit = false` in `[ui]` or the Config Editor; `Ctrl+C` still quits at once
- `graph_style` (`dots`/`bars`) in `[dashboard]`, `mouse` in `[ui]` and `tick_ms` in `[refresh]` (#174): the Dashboard graphs' style (`m`), mouse capture (`F2`) and the refresh speed (`-`/`+`, 500-30000 ms) are now settings, in the Config Editor too, instead of starting as dots, on and 2000 ms every time
- `max_output_kbs` under `[refresh]` (#172): caps the bytes per second salt-radar sends to the terminal. On a busy master the scrolling lists and full Dashboard graphs send a few KB/s (measured: 5 KB/s on a full Dashboard, about 13 times the output with the graphs hidden), which a session-recording SSH proxy such as CyberArk can't keep up with, so the whole connection, tmux and other windows on it included, became sluggish after a few minutes. Over the cap, refreshes wait while key presses still redraw at once; the header shows the cap, yellow while a refresh is waiting. Off (0) by default

### Changed

- Bundled themes use their own palette for the colors salt-radar adds (#145): job outcomes, the four filter highlight colors, the Pulse markers, purple and the title-bar version used one generic set in most themes, so Nord, Tokyo Night, gruvbox and the rest showed the same green, yellow and pink. 30 themes now take them from the color scheme they port (Tokyo Night, gruvbox, Everforest, Kanagawa, One Dark, Solarized, Dracula, …) or from their btop original, which also fixes gruvbox's yellow being its green and Tokyo Night's orange being its yellow. A role stays on the default only where the theme has no color of that kind; every theme still passes the check that keeps status and highlight colors apart and readable
- Events Preview wraps long lines (#169): the tag and the JSON body used to be cut off at the panel's right edge, so long targets, arguments and state comments couldn't be read without opening the event. Wrapped rows hang under their key, and the body's height and its jump to the first filter match count wrapped rows, so the match stays on screen

### Fixed

- Count a minion targeted twice as one target in Live Jobs (#175): a list target naming a minion twice (`-L web01,db01,web01`) used to show 3 targets, 2 returns and one minion waiting forever, as Salt keeps the duplicate in the job's minion list. Live Jobs now counts distinct minions (2 targets, 2 returns), like Job Detail. Other target types (glob, pcre, grain, compound) never produce duplicates
- Dashboard toggles lost on restart (#174): the graph panels (`F4`–`F8`), Minions `f` and API `x` were saved only on a clean quit, so closing the terminal window or a dropped SSH session lost them. Every hotkey toggle, the new graph style, mouse and refresh speed settings included, is now saved the moment it changes, merged into the config file as it is
- Regex filter terms that couldn't be used failing silently (#168): a `/…/` term with a flag other than `/i` (`/fit/g`), or one this regex engine can't compile (look-around, an unclosed group), was quietly matched as plain text and found nothing, which looked like regex filtering being broken. Every list filter's bar now flags such a term with ⚠ and the reason; the Job Cache, which the agent filters with Python's `re`, flags only the flags. The help and README also say that regex terms are case-sensitive unless written `/…/i`
- Minions view not adding up to the Dashboard's key counts (#171): it listed only minions with an accepted key, so pending, rejected and denied keys never showed, and an ID with both an accepted and a denied key showed once, as `denied`, though it was accepted and working. The list now has a row per key, matching Minion Key Status: the denied duplicate is its own row next to the accepted one, other keys show their status with no minion data, and the filter matches key status (`denied`). The "minions" count on the Dashboard's Salt RSS gauge is now the accepted key count
- Live Jobs and Job Detail disagreeing on a return's outcome (#166): Live Jobs went by the event's `success` flag alone, while Job Detail and the Minions Fail% column also counted an empty return as a failure, so `config.get` of a missing key was OK in Live Jobs and RC 1 in Job Detail. All views now share one rule: Salt's `success`/`retcode`, plus failures Salt misses in the return data (a failed state, an `ERROR:` string); an empty return is a success
- Job Detail output search carried over into the next job (#170): the query, its matches and `n`/`N` stayed active after closing the popup, so another job opened mid-search and `n` jumped through the old matches; the `f`/`F` failed-state marker lingered the same way. Opening a job now starts with neither
- Events filter showing events that don't match (#167): a term naming a known minion also showed every job whose target named *any* known minion, so on a fleet targeted by lists most jobs passed any minion term. It now shows only jobs whose target covers one of the minions the term names (`*`, a list member, a glob, an `L@`/`G@` word), and list targets sent as arrays count too. The Preview highlights the matches in the body and, when the first one is below the visible part, starts there so the reason an event matched is on screen

## [0.8.2] - 2026-10-13

### Fixed

- Slow Live Jobs filter on busy masters (#165): every job re-checked each filter term against every accepted minion (to decide whether to also show jobs targeting `*`), so with ~1.3k jobs and a few thousand minions a term like a JID took hundreds of milliseconds per pass, several passes per keystroke and poll; the check now runs once per pass. Regex terms with classes or anchors (`/^db1\d\d$/`) were slow on their own too, as every field was matched with a fresh copy of the regex that had to rebuild its match cache; each term is now compiled once per pass
- Slow Events and Minions filters with many minions, the same causes as #165: the Events filter checked each term, and each job's target, against every accepted minion for every event, so with 3,000 minions applying it froze the TUI for 1-5 s and every incoming event cost up to half a millisecond; it's now resolved once when the filter or the keys change (applying it takes milliseconds, an event under a microsecond). The Minions list and Fleet overview filters now compile regex terms once per pass too (`/^db1\d\d$/` over 3,000 minions: 13 ms to 0.1 ms)
- Mouse in the Job Detail popup (#164): a click on a minion in the returns list now selects it, a double-click opens its return like Enter, and a click on the return preview focuses it; left-clicks in the popup used to be ignored
- Help for paging (#163): the help overlay's keys and border hint now say that `←`/`→` switch its pages (`Shift+Tab` back), and the General keys list `Shift+Tab` for the previous tab; README to match

## [0.8.1] - 2026-10-11

### Fixed

- Theme colors (#162): in `horizon`, `night-owl` and `everforest-light-medium` a highlighted filter or sniffer term was the exact color of an OK result, and in `adapta` two of the term colors were near-identical purples; non-current search matches in `flat-remix-light` and the current match in `paper` and `solarized_light` had too little contrast; and in themes with a transparent background (`adapta`, `flat-remix`, `matcha-dark-sea`) the current search match was drawn in the terminal's own text color on yellow. A test now checks every bundled theme for distinct status and term colors and readable text

## [0.8.0] - 2026-10-11

### Added

- Per-user config under sudo (#160): when salt-radar runs as root with `SUDO_USER` set, it uses the invoking user's config (starting from root's until their first save), personal themes, "updated to vX" marker and home for `~` (the default export directory), and hands what it creates there — config, marker, exports — over to them, instead of every sudo user sharing root's. The README has a note on running through sudo and on several users at once
- A `1337` theme: near-black with the bright accents of the 1337 color scheme — lime for success, cyan for changes, amber and orange for warnings, red for failures (#161)
- Job Cache filter: the same terms as every other filter (#149) — comma-separated (any may match), globs (`*`, `?`, `[…]`) matched against the target, and `/regex/` / `/regex/i` terms, applied by the agent across the whole cache (noted in help's **Filter and search terms**); it was a single plain substring
- Regex terms in every list filter and in the output searches (#148): a filter term wrapped in slashes is a regex (`/web-0[1-5]$/`, `/…/i` ignores case; commas inside it belong to it) on Events, Minions, Live Jobs, Alerts, API, the minion activity popup and Job Detail's host filter — which also gains comma-separated and glob terms like the other filters. Job Detail's output search and the help search take a whole query wrapped in slashes as a regex. Text that isn't a complete `/…/` stays plain, so a filter being typed never errors and `/etc/salt/minion` still searches for the path. Help has a new **Filter and search terms** section with examples
- Text inputs have a real cursor: every filter, search, sniffer prompt, Config Editor field and the save prompt shows where you are (the character under the cursor in reverse video) and edits there — `←`/`→`, `Home`/`End`, `Delete` and `Backspace` — instead of only appending and deleting at the end behind a `|` that blinked with the text length
- Help: an **Event Sniffer terms** section with an example of each kind of term (minion, glob, keyword, regex, regex ignoring case, several, `h`); `F1` in the sniffer prompts opens help right at it
- Event Sniffer: capture by minions, globs, keywords or `/regex/` terms (#146) — the prompt now reads `Sniff for (minion, glob, keyword or /regex/, comma-separated)`; a term wrapped in slashes is a regex (`/…/i` ignores case, commas and slashes inside it are part of it) and an invalid one keeps the prompt open with the error shown; a term that names known minions (an ID or a glob) captures events from or about them, including jobs targeting them by `*`, glob, list or the master's resolved minion list; any other term is a keyword matched anywhere in the tag or data, ignoring case. Keywords and regexes match the event as shown (tag and pretty JSON, line by line), so every match is colored. The `h` filter takes the same keyword and `/regex/` terms, and empty input clears it
- Config Editor: preview `popup_dim_pct` live — once a new value is confirmed with `Enter`, the dimming behind the editor uses it; cancelling or discarding the edit reverts it, like the theme preview
- Show the changelog in the app: the help overlay (`F1`) has a **What's new** page (`Tab` switches to it), built into the binary and searchable like Job Detail's output (`/`, then `n`/`N`, on either help page); after an upgrade the title bar says `updated to vX.Y.Z · F1: what's new` and `F1` opens straight on it. The changelog is also installed to `/usr/share/doc/salt-radar/` by the `.deb` and `.rpm` and included in the `.tar.gz` (#139)

### Changed

- Themes `nord`, `adwaita-dark`, `twilight` and `orange`: the status, pulse and highlight colors salt-radar adds (which btop themes don't define) come from each theme's own palette — Nord's aurora and frost, GNOME's palette, TextMate Twilight's — instead of the generic defaults every port shared; `orange` is monochrome like `greyscale`, its statuses amber shades getting brighter with severity (part of #145)
- Help has three pages, `Keys ├┤ Guide ├┤ What's new`: Keys lists only the shortcuts, and the new Guide page holds the filter, search and sniffer syntax and the notes on what panels show, which used to follow the shortcuts; `Tab` / `←→` cycle the pages, `w` jumps to What's new, and `F1` from the sniffer prompts opens the Guide at the sniffer section; the help window is the size of the Job Detail popup (85% of the screen) instead of 60% × 65%
- Filter and sniffer highlighting gives each term of a search its own color in the order typed (six before any repeats, a glob's minions sharing their term's), instead of picking from four colors by a hash of the text, which often gave two terms of one search the same color

### Fixed

- Config saves are atomic (a temp file renamed into place), so two instances saving at the same moment can't leave a torn file
- Events filter: a glob's minions are highlighted in the glob's one color, instead of a color per minion that reshuffled every few seconds as the minion list refreshed
- Globs take `[…]` character classes like Salt's own targeting (`debian[12]*`, `web-[1-3]`, `[!0-9]`), in every filter and the sniffer — the bracket was matched literally, so such a glob matched nothing
- Show an active filter as `filter: web*` and a search as `search …: match 1 of 4` instead of wrapping them in slashes (`/web*/`), which now reads as a regex
- Search highlighting finds matches that span differently colored parts of a line — `Result: False` in Salt's colored output was never found, as each colored piece was searched on its own
- `/` on the Dashboard no longer opens a filter prompt whose input went nowhere — like Pulse, the Dashboard has nothing to filter
- Event Sniffer: a glob such as `web-*` now captures those minions' events (it only colored them, capturing little more than jobs targeting `*`), capture ignores case like the coloring does, and a keyword no longer captures every job targeting `*`
- Config Editor: `PgUp`/`PgDn` and `Home`/`End` work in the theme and other choice pickers and in the columns picker (paging by 5, like the settings list), and the choice picker's hints show its own keys instead of the settings list's
- Themes: color Salt's own colored output (ANSI codes in job returns) with the theme's colors instead of the terminal's palette, and make the `greyscale` theme grey throughout — its status, pulse and highlight colors were the port's colorful defaults; now the more attention a state needs, the brighter its grey
- Search in Job Detail output and help: `n`/`N` step through each match instead of each matching line (two matches on one line are two stops); the current match is a solid block and the others are tinted, in colors taken from the theme so they read on dark and light themes alike, instead of marking the whole line with the selection color, which turned the matches on it into dark text on blue; `Enter` on a pasted query jumps to the first match; the border no longer says `4 matchs`
- Ignore clicks on the Events list while the event sniffer is shown over it, so a stray click no longer opens an event from the list behind the sniffer
- Help overlay with the mouse on: a right-click closes it like other popups, clicking `Keys` / `What's new` in its border switches pages, and clicks no longer reach the tab behind it
- Keep the open popup when the Config Editor (`F3`) is opened over it: closing the editor returns to the popup as it was (selection, filter, scroll), and the popup keeps updating underneath; clicks no longer reach the tab behind the editor

## [0.7.6] - 2026-10-10

### Added

- Job Detail: show the job's arguments in the header and mark test runs with a `TEST RUN` badge, so a run whose states all read `Result: None` is recognizable as `test=True`
- Job Detail: list targeted minions that haven't returned as `NO RETURN` rows after the returns (selectable and filterable, with an explanation in the output pane), and show them in the header (`Returns: 42/43 (3 failed, 1 no return)`); the target list comes from the job cache

### Changed

- Use one set of outcome colors everywhere a job result is shown (Live Jobs, Job Detail, the minion activity popup, `Fail%`): matching Salt's own highstate output: green succeeded, cyan changed, yellow would change (a `test=True` run's `result: None`), red failed, plus orange for no return / still waiting and dimmed for zero or not applicable — so the `Chg` count reads cyan for a real run and yellow for a test run, Job Detail's header colors `failed` and `no return` separately, `Fail%` is red for any failure instead of yellow below 25%, and runner jobs' `?` counts are dimmed instead of orange
- Job Detail: lay the returns list out as an aligned table with a header row — minion, status, then right-aligned `ok`/`chg`/`fail` counts for state runs (each column as wide as the job's largest number) or the retcode for other jobs; the list widens to fit it, up to 45% of the popup, and names that still don't fit are shortened in the middle (`web-prod-…b-node-17`) so their distinguishing end stays visible

### Fixed

- Job Detail: update the open job as its returns arrive — a return for it on the event bus triggers a fresh lookup a second later (one per burst), keeping the selected minion, filter and scroll; a refresh that fails leaves the data already shown
- Job Detail: shorten a long target (a minion list, a compound match) in the header so the return counts after it stay on screen
- Job Detail: count a minion as failed when its command exited non-zero even though the output looks normal (e.g. `/bin/sh: uptime: not found`, rc 127) — the lookup now uses the retcode Salt stored in the job cache, so Job Detail's failed count matches Live Jobs

## [0.7.5] - 2026-10-10

### Added

- Minion activity popup (`r` on Minions): show each job's function, args, result and full JID, one row per job (a live return replaces its Job Cache entry), Job Cache entries targeted by glob or list, and a summary of the minion's last 50 returns since salt-radar started, with its latest failure (#130)
- Minions: optional `Fail%` column (`fail_pct`) showing the share of each minion's last 50 returns that failed, yellow below 25% and red above, sortable worst first; the activity popup's summary uses the same per-minion history, which no longer depends on the event buffer (#131)
- Minion activity popup: `Enter` on a job return opens Job Detail with that minion's return selected, like Job Cache rows, instead of the raw event
- Job Detail: search the return output with `/`; matches are highlighted, the pane's border shows the match count, and `n`/`N` jump to the next/previous one, marking its line (`match 2 of 5`) and working from the returns list too (#128); the border hints show `n/N` and `f/F` only when there is a match or failed state to jump to, and label `/` as "filter hosts" or "search output" depending on the pane
- Surface failures (#127): Job Detail lists each minion's state counts (`12 ok 3 chg 1 fail`) for state runs, the header counts failed returns, `f`/`F` jump between failed states in the output (marked, with `failed state 1 of 2` in the border), and `x` on Live Jobs shows only jobs with a failed return
- Job Detail: the returns-list filter (`/`) now narrows the list as you type, ignores case, highlights the matching part of each minion ID, and stays visible once confirmed (`Returns (3/12)` and `/web: 3 of 12` in the panel border); `Esc` clears it before closing the popup

### Fixed

- Minion activity popup: make the `/` filter filter the list shown, so `Enter`, copy and clicks act on the highlighted row

## [0.7.4] - 2026-10-10

### Added

- Add saving the Events tab's filtered event list to a file: `s` as readable text, `S` as NDJSON (one `{"tag", "ts", "time", "data"}` object per line, ready for `jq`), through the same save dialog as the popups, with the active filter in the proposed file name (#4)

### Changed

- Event Sniffer: `S` now saves the shown events as NDJSON (it saved the same text as `s`), shown in its hints as `s/S:save txt/json`, with its own actions listed before the standard paging keys so a narrow popup cuts the least useful hints; text exports label their timestamps as UTC
- Split the 7,100-line `src/app.rs` into modules — key handling, agent messages, the Config Editor and clipboard — without behavior changes (#125)

## [0.7.3] - 2026-10-09

### Changed

- Config Editor file browser: type the start of a name to jump to the first matching entry (case-insensitive, shown in the border; a one-second pause starts a new name); typing a path by hand moves from `e` to `/`, and `.` toggles hidden files only when not typing a name (#124)
- Complete the help overlay (`?`): every shortcut from the README's keyboard tables, grouped the same way (General, Per tab, Config Editor, Popups) and scrollable — it was missing e.g. F2, F3, `/`, Home/End and the Job Cache and Config Editor keys; both are now built from one list, checked by a test (#123)
- Save only the settings that differ from the defaults when the Config Editor (or quitting with toggled graphs) writes `salt-radar.toml`, instead of every setting — unchanged settings now follow future defaults instead of staying pinned to the ones of the version that saved the file; files written by older versions slim down to their real changes on the next save (#121)

### Fixed

- Fix the Live Jobs cursor jumping to a different job when new jobs arrive or old ones age out while it isn't following the newest job: it now stays on the same job (#122)
- Fix the API request log showing an empty path, instead of `?`, for a request line it couldn't parse (found by the new unit tests, #119)

## [0.7.2] - 2026-10-09

### Added

- Add a file browser to the Config Editor for settings that must point at an existing path — `agent_path`, `python_bin`, `master_config` (files) and `save_dir` (a directory) — instead of free-text entry, starting at the current value or its nearest existing folder; `e` still allows typing a path, and paths that don't exist are flagged `✗ not found`
- Add `popup_dim_pct` under `[ui]` (also in the Config Editor): how strongly the screen behind a popup is dimmed, 0–100% — default 70%, `0` turns dimming off
- Add `mem_gradient_start`/`mem_gradient_mid`/`mem_gradient_high`/`mem_gradient_end` theme keys: the Memory graph, its headline figure and the Host Resources and Services memory meters get their own level gradient — cyan → blue → amber → red by default, so memory reads blue again while it's calm; the bundled btop ports take it from btop's own memory (`used_*`) gradient (#115)

### Changed

- Finish a fleet scan as soon as every expected minion has answered, instead of always waiting out its 60-second timeout (which still marks minions that never answer as timed out) (#116)
- Keep the Live Jobs cursor on the newest job as new jobs arrive, like the Events tab: moving it elsewhere stops following and `End` resumes it; with a sort column active the cursor stays put (#117)
- Dim the screen behind an open popup more strongly by default (70%, was 40%), so the popup stands out more
- Show the save-to-file prompt (`s`/`S` in a popup) as its own small dialog on top of the popup — directory, file name and the "file exists" warning — instead of a line in the status bar, which the dimmed backdrop washed out
- Propose a `.json` file name when saving JSON — Job Detail `S`/`A`, and Event Detail `S` in raw view — instead of `.txt` for everything

### Fixed

- Fix the Host Resources panel showing `Mem: 0 MB / 0 MB (100%)` as a full red memory bar (and a made-up `1-core host`) while the agent hasn't reported yet — e.g. it's down or still starting: it now reads "waiting for agent…" with empty `n/a` meters, and the Services, CPU and Memory headlines show `n/a`; the Services memory figure also follows the memory gradient now instead of fixed thresholds (#118)

## [0.7.1] - 2026-10-08

### Added

- Add `zmq_latency_ceiling_ms` under `[dashboard]` (also in the Config Editor): the latency that counts as "bad" at the top of the ZMQ graph, defaulting per `zmq_latency_source` — 100 ms for the agent round-trip, 1 s for `test.ping`, 500 ms for `master_stats`
- Add `gradient_start`/`gradient_mid`/`gradient_high`/`gradient_end` theme keys for the level colors below; the bundled btop theme ports take them from each btop theme's own CPU gradient

### Changed

- Color the CPU, Memory and ZMQ graphs btop-style by level: each row by its height on the graph's fixed scale, blending green → yellow → amber → red toward the top, instead of painting the whole graph one color that flipped with the latest value (ZMQ) or never changed (CPU, Memory); the headline figures and Host Resources meters use the same smooth gradient instead of hard thresholds, so a value hovering near a boundary no longer flickers between colors
- Draw the ZMQ graph on a fixed log scale up to `zmq_latency_ceiling_ms` instead of scaling it to its own recent peak, so bar heights — and their colors — mean the same thing over time

### Fixed

- Ship the bundled themes in the `.rpm` and `.tar.gz` release packages too — v0.7.0 only included them in the `.deb` (#114)

## [0.7.0] - 2026-10-08

First stable release since 0.6.2. Besides the changes below, it includes everything from the 0.6.3 release candidates that follow: configurable and sortable table columns with a new Args column, save-to-file from every popup, the Event Detail pretty view, multi-term per-tab filtering, much faster Job Cache and Job Detail on large masters, and many fixes.

### Added

- Add btop++-style color themes: `color_theme` in a new `[ui]` section of `salt-radar.toml` picks the built-in `Default` or a `*.theme` file from `/usr/share/salt-radar/themes/` or `~/.config/salt-radar/themes/` (or a path to one), `theme_background = false` keeps the terminal's own background, and the Config Editor previews each theme live while picking; `salt-radar.theme.example` documents every key (#114)
- Ship ports of btop++'s 41 bundled themes plus its built-in default (as `btop`), installed to `/usr/share/salt-radar/themes/` — status colors keep their meaning (red stays red for failures) and text stays readable on each theme's background and cursor bar (#114)

### Changed

- Dim the screen behind any open popup (detail views, Event Sniffer, Config Editor, admin confirm, help) so the popup stands out — fading toward the theme's background, so light themes wash out instead of going gray; a popup opened on top of another dims the one beneath it too, and the screen returns to normal when it closes (#111)
- Make the selected row stand out in every table and list: a stronger blue cursor bar with bold text, dim text on it lifted so it stays readable, and the Job Detail Returns list now gets the same full-width bar instead of only a `▶` marker (#112)

### Fixed

- Fix the Dashboard ZMQ latency (`agent_ipc`) reading a flat ~100ms no matter how fast the agent answered: commands reached the agent through a temp file polled every 100ms, and replies were timed when the main loop next woke up rather than when they arrived; commands now go straight to the agent and replies are timed on receipt, so the figure is the real round trip (typically a few ms). This also stops commands appended while the file was being emptied from being silently lost, and saving the Config Editor no longer wipes the ZMQ graph unless `zmq_latency_source` changed
- Fix the default (unsorted) order of the job tables: Job Cache now lists newest first (it re-sorted the agent's newest-first page oldest-first), Live Jobs runs oldest to newest by JID instead of event-arrival order, and jobs started within the same second — including under the Job Cache `Started` sort — are ordered by JID instead of coming out in disk-read order (#110)
- Fix the Job Detail Returns list scrolling on `PgUp`/`PgDn`/`Home`/`End` when it already fits on screen (`End` pushed the last return to the top and the rest out of view); it now only scrolls as far as needed to keep the selection visible, those keys skip rows hidden by the return filter, and the Minion detail grains pane no longer leaves dead `↑` presses after `End` (#113)

### Removed

- Remove the `c` high-contrast selection toggle; a high-contrast look is now a matter of picking (or writing) a theme (#114)

## [0.6.3-rc13] - 2026-10-07

### Added

- Add an Args column to Live Jobs and Job Cache showing what each job was run with — the requested formula for `state.apply`/`state.sls` (e.g. `apache.install`), `highstate` when none was named, a dimmed `(test)` marker for `test=True` runs, and the arguments of any other function (e.g. `uptime` for `cmd.run`); the `/` filter on both tables also matches it
- Add column sorting to the Minions, Live Jobs, Job Cache, Alerts and API request log tables: click a column header (or `o` to step through columns, `O` to reverse) to cycle ascending, descending and the default order; the Job Cache is sorted by the agent across the whole cache before paging, not just the page on screen, Minions keep timed-out minions in their own section, and Salt versions sort numerically (3006.10 after 3006.9)
- Add per-table column choice and order for Minions, Events, Live Jobs, Job Cache, Alerts and the API request log — `minion_columns`, `event_columns`, `live_columns`/`cache_columns`, `alert_columns` and `request_columns` in their `salt-radar.toml` sections, editable with `v` on the table or in the Config Editor (`+`/`-` to move, `Space` to show/hide) — with new optional columns: Live Jobs Started, Minions Arch and Seen, Events Minion, Alerts Since and Count

### Changed

- Shrink a table's free-text columns to share the space when its panel is too narrow to fit every column, instead of cutting off the rightmost ones, and give spare width to the widest text column (e.g. Description, Tag, Args) rather than leaving it empty

### Fixed

- Fix the Target column being blank (Job Cache) or `?` (Live Jobs) for list-targeted (`-L`) jobs
- Fix Config Editor changes being silently lost: closing it with unsaved changes now asks first (Esc/q again discards), the title shows when there are unsaved changes, and `s` saves from anywhere in the editor instead of only from the Save button at the bottom
- Fix quitting salt-radar overwriting `salt-radar.toml` with its whole in-memory config, which discarded anything saved to the file while it ran (by hand or from another instance); quitting now only merges in the hotkey toggles changed during the session (F4–F8 graphs, Minions `f`, API `x`)

## [0.6.3-rc12] - 2026-09-30

### Fixed

- Fix Job Cache re-reading the whole master job cache on every search, page, limit change and refresh: the agent now keeps an in-memory index of jobs it has already read and on each request only reads jobs that are new since the last one (and forgets jobs Salt has expired), so only the first load pays the full cost. Concurrent requests share one read, and a reply to an older Job Cache request can no longer overwrite the results of a newer search (`local_cache` job-cache backend only)

## [0.6.3-rc11] - 2026-09-30

### Added

- Add `lookup_timeout_secs` under a new `[jobs]` section in `salt-radar.toml` / the Config Editor (default 300s) to bound how long a Job Detail lookup may run, replacing the hard-coded 60s agent / 65s UI limits

### Fixed

- Fix Job Detail failing to load huge jobs (e.g. `grains.items` across 4000 minions): run the colored and JSON `salt-run jobs.lookup_jid` calls concurrently instead of back to back, keep the JSON result even when the colored rendering misses the deadline, split the colored output into per-minion sections in one pass instead of one full-text search per minion, and show why a lookup failed in the popup instead of "No returns yet." or silently closing it

## [0.6.3-rc10] - 2026-09-30

### Fixed

- Fix TUI lag on busy masters (hundreds of events/sec): index live jobs by JID instead of scanning the whole job table on every job event, stop deep-cloning every event just to count it and the last 500 events on every alert tick, keep the minion index between drains instead of rebuilding it each time, release the agent queue lock before processing a batch, handle all queued keypresses before one redraw instead of redrawing per key, resync the sniffer's following selection once per batch, and filter the Live Jobs list once per frame instead of once per row (#104)

## [0.6.3-rc9] - 2026-09-27

### Added

- Every popup that already supports copy-to-clipboard (`y`/`Y`) now also supports saving that same content to a file (`s`/`S`) — proposes a filename from what's being saved, lets you edit it before confirming, and warns if it would overwrite an existing file. Default save directory is `~` (the launching user's home), configurable as `save_dir` under `[export]` in `salt-radar.toml` / the Config Editor (#103)
- Job Detail: `a`/`A` on the return list save every return in the job to a single file (Salt outputter format, or one JSON object keyed by minion id) — previously `y`/`Y`/`s`/`S` only covered the currently-selected return, so a job with hundreds of targeted minions had no single-window copy/save of the full result set (#103)

### Fixed

- Save-to-file's in-popup confirmation showed "✓ Copied" (the clipboard indicator) instead of "✓ Saved", since both actions shared one timestamp; save now has its own (#103)

## [0.6.3-rc8] - 2026-09-16

### Added

- Minions tab: the active list filter (`/`) can now also scope the Fleet Overview panel's OS/Salt-version/architecture breakdown to just the matching minions, instead of always aggregating the whole fleet — so filtering down to a single customer or host pattern shows that subset's distribution, with a "Filtered: N of M minions match /pattern/" label when scoped. Toggled live with `f` on the Minions tab (shown as a hint on the panel's bottom border), defaulting to on; the default is configurable via `follow_filter` under `[minions]` in `salt-radar.toml` / the config editor (#101)

## [0.6.3-rc7] - 2026-09-11

### Added

- Live Jobs list: the single "Progress" column (a bare `returned/target` count that read identically whether every minion succeeded or some of them failed) is now four spelled-out, fixed-width columns — `Tgt`/`OK`/`Fail`/`Wait` — so targeted/succeeded/failed/still-pending are each their own number, aligned top to bottom, with no mental arithmetic or symbol legend needed. Sourced from the live return event's own `success` field (the same authoritative value Salt itself sets), not re-derived client-side

### Fixed

- Live Jobs could permanently mistrack a broadcast job (e.g. `salt '*' ...`) as a one-off local execution targeting a single minion, showing e.g. `Tgt: 1` while further returns for the job's real targets kept incrementing `OK`/`Fail` past it. Root cause: the inbound agent-message queue is capped (to bound memory during a burst) and drops the *oldest* queued message once full — on a sustained high job rate, that oldest message could be a job's own `/new` (job-start) event, which is the only place the real target/count is ever learned; once dropped, every `/ret/` for that jid fell into the "no `/new` seen → must be a one-off local execution" fallback and stayed there for the job's whole life. Fixed on two levels: the queue now protects `/new` events from eviction, evicting the next-oldest non-`/new` message instead; and as a backstop regardless of cause, a job's tracked target count now always widens to match however many returns have actually come in, so the display can never show fewer targets than returns received
- Job Detail's per-minion return list could show `OK (retcode=0)` for a minion whose state run actually failed (e.g. a `state.apply` with no matching top file, or any state with `result: false`). The retcode/success derivation only checked for a top-level `result` or `retcode` key, but a state-return is a dict keyed by state ID with each state's own `result` nested one level down — finding neither at the top level, it silently defaulted to success. Now recognized as a state-return dict and considered failed if any state inside has `result: false`. Same latent bug fixed in the agent's parallel (currently unused by the Rust client) computation of the same values

## [0.6.3-rc6] - 2026-09-10

### Added

- Event Detail popup: `t` toggles between raw JSON and a **pretty view** for job-return events. Fetches the exact `salt-run jobs.lookup_jid --force-color --state-output=full --state-verbose=True` output — the same subprocess call Job Detail already uses — so formatting and colors are authoritative Salt output rather than an approximation; falls back to a client-side rendering only for non-job-return events or if the fetch fails/times out. While the fetch is in flight, the raw JSON stays on screen instead of a client-side approximation, so there's no "wrong colors, then right colors a moment later" flash once the real output lands (#99)
- Multi-term filtering (`/`) on Events/Minions/Jobs/Alerts/API: a comma-separated list of terms is OR'd together, and each term may be a glob (`web1,web2` or `web*`), mirroring Salt's own `-L`/glob targeting syntax rather than introducing regex. Matched terms are colored consistently — same term, same color, everywhere and every session — via a deterministic hash into a small, fully-saturated palette kept separate from the existing severity colors (green/red/yellow already mean success/fail/warn elsewhere in the UI). Applied to the Events tab list and the Event Sniffer's captured content (#100)
- Event Sniffer: the minion ID(s)/glob entered when launching it (`n`) are now colored wherever they occur in captured content (tag and JSON body) — previously only the capture filter used them, with no corresponding highlight (#100)

### Changed

- Filtering on Minions/Jobs/Alerts/API is now isolated per tab — each keeps its own filter text independently, so typing a filter on one tab (e.g. a host name on Events) no longer also silently filters an unrelated tab (e.g. request method/path/status on API) until manually cleared. Events already had its own isolated filter; this fixes the four tabs that were sharing one field
- Event Sniffer's `h` keyword field now also filters the displayed events, not just colors matches within them — narrows to only events containing at least one active keyword, consistent with what a filter box is expected to do. It's deliberately a *separate, optional* search on top of the sniffer's own minion-ID relevance filter (which stays broad — e.g. a wildcard-targeted job counts even without literally naming the minion — see Fixed below for why those two needed to stay decoupled)
- Event Detail popup: `y`/`Y` clipboard copy now matches whichever view is currently on screen (pretty or raw) instead of always copying raw JSON regardless of the toggle state

### Fixed

- Opening any detail popup (Minion/Job/Alert/API detail, Minion Events) on another tab while the Event Sniffer was running silently discarded it, since the sniffer shared the same single popup slot as every other, tab-scoped popup. The sniffer now lives independently of that slot, so it keeps capturing and reappears untouched when you come back to Events — while other popups on other tabs open normally instead of being blocked. A related stale-tab-tracking bug could also leave the sniffer completely unresponsive to input (including `q` to stop it) after any popup had been opened and closed on the Events tab, even though it kept rendering normally
- Job Detail / Event Detail pretty view could take many minutes (or effectively never finish, well past the popup's own 65s lookup timeout) to load a job with a very large number of minions — e.g. `grains.items` across 1000+ minions, ~230k lines of `salt-run jobs.lookup_jid` output — despite the underlying `salt-run` command itself returning in seconds. The agent's per-minion text-splitting (extracting each minion's own slice of the combined CLI output) scanned every *other* minion for each minion to find its section boundary — O(minions²) string searches, fine for a handful of minions but ~1,000,000 searches over a multi-megabyte string at 1000+ minions. Now computed in one pass per minion instead of one pass per minion-pair; a synthetic 1200-minion benchmark went from an estimated ~18 minutes to ~1.25 seconds with byte-identical output.
- Fix Events tab filtering causing progressive slowdown, sluggish tab-switching, and extended freezes on busy masters. The filtered event list is no longer re-scanned from the whole event buffer on every redraw, every keystroke, every incoming event, or even once per tick — it's now an incrementally maintained cache updated in O(1) per ingested/evicted event, with a full rebuild only when the filter itself changes (Enter/Esc). The filter is also applied on Enter instead of live per character (#98)
- Event Detail pretty view rendered a state.apply return's states alphabetically by state ID instead of actual execution order, because this crate's `serde_json` doesn't enable the `preserve_order` feature and so silently re-sorted the raw dict's keys. States are now explicitly sorted by `__run_num__` (the same field Salt's own highstate outputter uses) whenever a dict looks like a state-return — Job Detail's raw-JSON fallback path (used when the real fetch's `formatted_text` is empty) had the same latent bug and gets the same fix, since both share the renderer
- Event Sniffer: an early version tied the minion-ID coloring above directly to the keyword filter, which caused two bugs in sequence — coloring silently did nothing when the minion target was a glob (a glob never literally appears in rendered text to color), and, once that was fixed by seeding the filter from the minion target, wildcard-targeted jobs relevant to a sniffed minion were incorrectly hidden (their JSON doesn't literally contain that minion's name). The two concerns are now fully decoupled: minion-ID coloring is purely cosmetic and never filters, only `h` narrows the displayed view

## [0.6.2] - 2026-09-05

This release folds in `v0.6.2-rc1` through `v0.6.2-rc11`. Where a later
RC changed or fixed something an earlier RC in this same range
introduced, only the final end-state behavior is described below —
notably the salt-api `pillar` auth mode, which went through two design
iterations (a fixed placeholder pillar ID, then briefly `'*'`-targeted
pillar) before landing on resolving this master's own local minion ID,
the only one described here.

### Added
- Dashboard: `show_host_resources`/`show_services`/`show_key_status`/`show_event_types`/`show_job_pulse`/`show_salt_api` config settings, each independently hiding one of the Dashboard's fixed (non-sparkline) panels — same on/off toggle, in the Config Editor (`F3`), as the existing sparkline `show_*_graph` settings. Hiding one of Services/Minion Key Status/Event Types reflows the row to fill the freed width instead of leaving a gap.
- API tab: `hide_stats_polls` config setting (`[api]`, default `true`) filters salt-radar's own `GET /stats` poll out of the request log — left in, it added one entry per poll tick and drowned out real traffic. Set to `false` to see it anyway, or toggle live with `x` on the API tab — the request log's title shows `/stats hidden` whenever filtering is active, so the toggle state is never invisible. The filter applies at display time (like the existing text filter), so toggling `x` shows/hides already-captured `/stats` entries retroactively instead of only affecting ones captured from that point on.
- `stats_auth_mode` (`[api]`, default `"none"`): an authenticated alternative to `stats_disable_auth` for the salt-api panel/API tab. `stats_disable_auth` removes salt-api's own auth requirement for `/stats` specifically, so anyone who can reach salt-api's REST port can poll it unauthenticated (scoped to request-line/client/status/timing metadata only, never bodies or headers, but worth firewalling to trusted hosts if that matters) — `stats_auth_mode` avoids that by having the agent log into salt-api via eauth and send the resulting token as `X-Auth-Token` instead. Two credential sources: `"plaintext"` (`stats_auth_username`/`stats_auth_password`, read straight from `salt-radar.toml` — marked insecure) or `"pillar"` (`stats_auth_username_pillar_path`/`stats_auth_password_pillar_path`, two independent colon-delimited pillar paths resolved entirely master-side, for this master's own local minion ID — the same one `salt-call pillar.get` would use here, not something you configure — with no live round-trip and no credential ever touching `salt-radar.toml` or the Rust process; a `top.sls` entry keyed to that minion's own ID, not `'*'`, keeps it scoped to just this host). A rejected/stale token or stale pillar-resolved credentials trigger one automatic re-resolve-and-retry before the panel reports unreachable. README documents the minimal eauth ACL scope (an empty permission list) that can read `/stats` but run nothing else, since `/stats`'s own auth check doesn't consult `external_auth` at all.

### Changed
- Dashboard: Job Pulse and salt-api now share one row side by side instead of each taking a full-width row — both are a fixed KPI-line-plus-up-to-3-rows panel of the same height, and neither needed the full row width, so combining them frees a full row of vertical space for the sparklines below.

### Fixed
- Typing or pasting into the Jobs (or Minions/Events/Alerts/API) filter recomputed every tab's filtered-list length on every keystroke, not just the tab actually being filtered — most costly, it re-ran Events' fallback full-JSON-stringify match against the entire buffered event list regardless of which tab was open. On a busy master with a large event buffer, this made pasting a JID into the Jobs filter take 60s+ to become active, one keystroke at a time. The per-character clamp now only recomputes the active tab's own filtered length.
- Dashboard's Services panel could show memory usage over 100% (and wildly inflated "Salt RSS"/process counts): the agent's process scanner walked every PID on the host, not just Salt's own, and anything that didn't match a known salt-* cmdline pattern was still included as an "Unknown"-role process rather than excluded — on a host also running unrelated software, that summed the whole system's RSS into a figure labeled "Salt RSS". The agent now skips non-Salt processes entirely, and the Rust side excludes Unknown-role entries from the aggregate as a second layer of defense.

## [0.6.1] - 2026-09-05

This release folds in `v0.6.1-rc1` through `v0.6.1-rc8`, plus the
salt-api/API-tab config exposure, a detail-popup label-padding fix, and
the API/Pulse tab reorder that landed after rc8. Unlike the v0.6.0
range, this one is a straight linear build — no RC in this range
reworked or reverted something an earlier RC in the same range had
already shipped, so every item below is described once, as it stands
today.

### Added
- Dashboard: a `salt-api` panel, shown only when a `salt-api` process is detected — request rate, error rate, `POST /login` p50/p95 latency, in-flight/total request counts, and the most recent request per worker thread (path, status, timing), polled from salt-api's own built-in in-memory request-stats endpoint (CherryPy's `cpstats`, exposed at `GET /stats`) rather than any log file. Needs `rest_cherrypy: {collect_stats: True, stats_disable_auth: True}` in the master config; the panel explains that when the endpoint isn't reachable. This is a different mechanism from `log_level`/access logging — cpstats records only the request line (method+path), never the body, so nothing captures credentials sent in a POST body (e.g. `/login`), and nothing is ever written to disk. `stats_disable_auth` is required because salt-radar's agent never handles or stores salt-api auth tokens — see README for details. Request rate and error rate/login latency are tracked by salt-radar itself, since cpstats only exposes a lifetime-average rate and a live per-thread snapshot, not cumulative error/latency counters of its own.
- New alert rule: API auth latency — fires when salt-api's `POST /login` p95 crosses `api_auth_latency_warn_secs` (default 3.0s), a common early symptom of PAM/LDAP/eauth backend trouble. Total alert rule count: 14.
- New **API** tab (`6`): a live, in-memory request log (age, method, client, path, status, timing — filterable by `/`) and a per-endpoint breakdown (count, avg/p95 latency, error rate), both built by salt-radar itself from cpstats' per-thread snapshots — cpstats has no request log or per-endpoint history of its own. Best-effort: only the latest request per CherryPy worker thread is ever visible per poll, so a thread completing more than one request between polls means the earlier one is missed. Shows the same "not reachable" guidance as the Dashboard panel when `collect_stats`/`stats_disable_auth` aren't set. `Enter` opens a detail popup (full request line including query string, client, status, processing time, `y` to copy) — cpstats never gives us more than that: no request/response body, no headers, so no credentials, payload content, or Salt-level context (which minion/job/user) is ever visible.
- New `[api]` config section (and Config Editor entries): `stats_window` (rolling request count backing the salt-api panel's error-rate/login-latency figures, default 200), `log_cap` (API tab request log cap, default 1000), and `log_max_age_secs` (API tab request log age prune, default 3600). The request log is bounded by both: capped at `log_cap` entries, and entries older than `log_max_age_secs` are pruned every poll regardless of how full the log is, so a quiet API doesn't let it hold onto hours-old requests. Eviction correctly shifts the current selection so it keeps pointing at the same logical entry, the same correction already made for the Events buffer's own eviction.
- README screenshots refreshed for every tab, including a new one for the API tab.

### Changed
- Reordered tabs: **API** is now `6` and **Pulse** is `7` (previously the reverse).

### Fixed
- API Request detail popup: the "Full request line" label ran directly into its value with no gap — the shared detail-row label padding was fixed at 16 columns, one short of this label's own 17-character length, so ratatui's left-pad (which only pads shorter strings, never truncates longer ones) added nothing. Padding is now 20 columns.
- API tab's request log columns ran together once an entry's age grew past its fixed column width (an un-truncated, ever-growing field silently misaligning everything after it) — the same bug class already fixed for Event Types/Fleet Overview; every field in that row is now truncation-safe.
- salt-api panel's `in/out` bytes-per-second line showed a doubled `/s` suffix (`0 B/s/s`) — `format_bytes` already appends it.
- Typing (or pasting) into the Live Jobs/Events filter box could take ages to keep up on a master with a large accepted-key roster (e.g. thousands of leftover test minions never cleaned up by autoclean) — every keystroke redraws the filtered list, and each of those redraws rebuilt a fresh `HashSet` of every accepted minion ID from scratch, once per visible row, to check whether the filter text matched a known minion ID. On a master with thousands of keys, a pasted JID (delivered as a burst of individual keystrokes, since there's no bracketed-paste handling) could take well over a minute to finish filtering. The accepted-ID set is now kept up to date whenever the key list refreshes instead of being rebuilt on every filter check.
- Mouse mode (`F2`) could "randomly" switch tabs or toggle the sparkline bar/dots style with no corresponding click — `EnableMouseCapture` was requesting button-drag (1002) and any-motion (1003) tracking on top of click tracking (1000), which report an escape sequence for *every* mouse movement whether or not a button is held; salt-radar has no handler for Drag/Moved mouse events at all, so that traffic was pure waste sharing the same stdin stream as keyboard input, and more escape-sequence volume means more chances of a sequence getting split or corrupted on a lossy/high-latency link (SSH, a terminal multiplexer, a KVM's input handoff) and leaking stray bytes into the keyboard-event path. Mouse mode now only requests click tracking and extended SGR coordinates (1000 + 1006).
- Dashboard's CPU panel (and the CPU history feeding its sparkline/avg/peak) was actually reporting 1-minute load average divided by core count, not real CPU utilization — a stale sample after a load spike ended, `load_1` stays elevated for a while as an exponentially-decayed figure even once real, instantaneous usage has already dropped, which read as CPU stuck at 100% while a tool like `btop` (sampling `/proc/stat` directly) already showed it back down. The panel now computes real system-wide CPU% from `/proc/stat` cumulative jiffies deltas between polls, the same way the per-process CPU%s next to it already were; the "Host Resources" gauge, which is explicitly labeled `Load:`, is unaffected and still shows the real load average as intended.

## [0.6.0] - 2026-09-03

This release folds in `v0.6.0-rc1` through `v0.6.0-rc25`. Where a
later RC changed or fixed something an earlier RC in this same range
introduced, only the final end-state behavior is described below — the
intermediate iterations aren't (see the individual RC tags for that
history). Three such cases: rc13 added a `~/.config` fallback for
`salt-radar.toml`; rc14 then reworked the Config Editor's border hints
around it — the behavior actually shipping
in this release is the "Config file lookup" entry under Changed below.
rc16 made Job Detail lookups a single `--out=json` subprocess
call with the return rendered by a Rust-side outputter port; rc17 restored
a second `--force-color` call alongside it after finding real gaps in that
approach — see the Job Detail entry under Fixed below for the shipped
end state. rc17 also made `minion_cache_list` an in-process `salt.key.Key`
call only, for the CPU fix described below; rc20 restored a `salt-key -L`
subprocess fallback after finding the in-process call unreliable on at
least one real master (where it left the whole roster permanently empty),
but that fallback ran on every poll on that same master, reintroducing
the CPU cost rc17 fixed; rc21 caches the resolved key list on its own
30s timer so the fallback subprocess, when needed, only runs there
instead of every poll — see the roster entry under Fixed below for the
shipped end state.

### Added
- Status bar now shows mouse mode's actual on/off state (matching the existing admin mode/contrast indicators), since the `F2:mouse on` border hint only names the key, not whether pressing it turns mouse mode on or off.
- Job Detail popup header now shows a per-lookup timing breakdown right behind the JID (`salt-run Xs, parse Xs`), turning orange past 10s total — useful for tracking down a lookup that's slow here but fast when the same `salt-run jobs.lookup_jid` command is run by hand, without having to go dig through the agent's stderr log.
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
- Job Detail's `salt-run jobs.lookup_jid` lookup now always passes `--state-output=full --state-verbose=True`, overriding the master's configured `state_output`/`state_verbose` defaults — a "terse"/"changes" master default would otherwise hide unchanged/no-op states from this popup even though opening it is an explicit request to see everything about that one job.
- Shortcuts moved out of the bottom key-hint bar and into the border of the panel each one affects (btop-style `┤key:label├`), so a hint stays next to the behavior it switches; the bottom bar is now status-only (interval, agent connectivity, admin/contrast state, selection position). Every popup — previously a mix of hand-written footers, or none at all for the Config Editor — now uses the same border-hint style.
- Refresh interval (`+`/`-`) is shown as a live btop-style value (`┤-2000ms+├`) in the title frame and works from any tab, not only the Dashboard.
- Title bar and tab menu merged into one bordered frame (app name/version, master version, centered clock, alert count, host/uptime), replacing both the old single-row title bar and the per-tab Dashboard header that used to repeat the same information one row below it.
- Config file lookup: `./salt-radar.toml` in the launch directory wins if present; otherwise it falls back to `$XDG_CONFIG_HOME/salt-radar/salt-radar.toml` (usually `~/.config/salt-radar/salt-radar.toml`), creating the directory on first save. An existing per-directory config keeps working unchanged.
- Sparkline panels default to Dots instead of Bars on first run. CPU/Memory sparklines in Bars mode render on a fixed 0-100 scale instead of auto-scaling to the visible window's own max, which previously exaggerated small values to a full-height bar.
- Service memory reporting reads the systemd unit's cgroup `MemoryCurrent` (covers the whole process tree, shared pages counted once) instead of just the main process's own RSS, falling back to RSS when cgroup accounting is unavailable.
- Job Cache's live "N jobs read" loading line: only the count itself is now accent-colored, matching the title text color used elsewhere; the label and elapsed time around it render in a single dim color instead of mixing grey/yellow/grey.
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
- A stray/garbled mouse click landing on the tab bar row could silently jump to Pulse regardless of where it landed — the column-to-tab lookup defaulted anything outside the five known tab-label ranges to Pulse instead of doing nothing, so any click on the (usually large) blank space to the right of the tabs also switched tabs. This made a single spurious mouse event — e.g. from a KVM switch's input handoff — likely to swap the view even without landing anywhere near a real tab.
- Job Detail lookups (opening a specific job's return data) diverged from real `salt-run jobs.lookup_jid` output in three ways: the JSON call's parsed result was discarded immediately after parsing (a leftover `lookup_data = {}`), so every return's success/retcode was silently reported as a default success regardless of whether the job actually failed; the return preview colored a failed minion's whole output red, which the real command never does (it doesn't pass a per-minion retcode to Salt's nested outputter, so dict/list coloring is always the normal cyan/green); and state.apply/highstate jobs rendered as a raw, unformatted state-ID dict (`file_|-...`, `__id__`, `__run_num__`, ...) instead of the real highstate-formatted output, because forcing `--out=json` (to get structured per-minion data) makes Salt skip auto-selecting the highstate outputter, and `jobs.lookup_jid` itself wraps those returns in an `{"outputter": "highstate", "data": {...}}` envelope that was never being unwrapped before iterating per-minion. All three are fixed: the JSON call's result is parsed and unwrapped correctly for real retcode/success, and a second `--force-color` (non-`--out`-forced) call captures the exact CLI text — real outputter selection, real colors — sliced per minion for display, falling back to a line-for-line port of Salt's own `nested` outputter (`salt/output/nested.py`) only if that raw capture is unavailable.
- Job Detail return preview lost coloring partway through any multi-line colored block (e.g. a state's `Changes`/stdout text spanning several lines) — real colored CLI output often sets a color once for the whole block instead of repeating the escape code on every line, relying on the terminal to keep it active until the next code. The preview parsed each line's ANSI codes independently (splitting on `\n` before coloring), so only the first line of such a block kept its real color and every line after it fell back to the default color until the next state's own color code reset things. ANSI parsing now runs once over the whole return, carrying color across line boundaries the way a real terminal does.
- Fleet Overview's OS breakdown showed only a bare version number (e.g. `8.7`) instead of the distro name (`RedHat-8.7`, `OracleLinux-8.7`, ...) — the rc17 CPU fix's rewrite of `minion_cache_list`'s per-minion grain fields dropped `osfinger` (the distro+version grain the OS breakdown actually groups by), silently falling back to the bare `osrelease` version number for every minion instead.
- Agent's `minion_cache_list` (polled every `dashboard_secs`, 2s by default) unconditionally unpickled every minion's cache entry on every poll — on a fleet with a large number of permanently-dead minions (e.g. leftover test minions never cleaned up by autoclean, thousands on some masters), this pegged agent CPU indefinitely. Grains are now only re-fetched when a minion's cache mtime has actually moved since the last poll.
- Minions and Pulse could show an empty roster indefinitely, even after a fleet scan (which populates Fleet Overview independently via events, masking the problem there): `minion_cache_list` listed minion IDs via an in-process `salt.key.Key(opts).list_keys()` call, which isn't reliable in every environment — on at least one real master it always came back empty, and with nothing else to fall back to, `minion_info` (both views read from the same roster) had nothing to ever populate it with. Restored the `salt-key -L` subprocess fallback `cmd_list_keys` already had for exactly this. On a master where the in-process call never works, forking that fallback on every poll (every `dashboard_secs`, 2s by default) reintroduced the same per-tick subprocess cost the CPU fix above exists to avoid — the resolved key list (whichever path produced it) is now cached for 30s, since the accepted-key roster itself changes far less often than any single minion's grains.
- All minions could vanish from the Minions and Pulse views after a long-running session even when the roster-listing above did work: the `minion_cache_list` result handler replaced the *entire* roster with each poll's result unconditionally, without checking whether that poll actually succeeded — unlike every other agent-result handler. A single transient failure (which itself used to report `ok: true` with an empty list instead of a real failure, also fixed) wiped the whole fleet from view until the next successful poll instead of just skipping that refresh and keeping the last known roster.
- The Python agent had no crash recovery: an unhandled exception anywhere in the main dispatch loop (e.g. the OS refusing to start a new thread after a long, busy session hits a `ulimit`) killed the whole process, and the Rust side never restarted it — every view depending on agent data froze permanently for the rest of the session with no way to recover short of quitting. `dispatch()` failures are now caught and reported instead of propagating, and the Rust side now respawns the agent process (resending its startup commands) whenever it exits instead of abandoning the connection for good.
- Dashboard Event Types panel's bars lost alignment once any category's count reached 6+ digits — the count field was a hardcoded 5-character width, so a longer count ran into the bar column, and since each row's overflow varied with its own digit count, every row's bar started at a different column instead of lining up. The count column now sizes to the widest count actually present, like other Dashboard panels already do.
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
- Mouse-click row selection (Minions, Events, Alerts, Jobs, Job Cache — Pulse was unaffected) drifted again and selected one row *above* the one actually clicked: each panel's filter bar only renders when a filter is actually active or typing, but the click handler assumed it was always present, over-counting the header offset by one row whenever no filter was set (the common case). The row math is back in sync with each panel's real conditional layout.
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
- The agent's stdin dispatch loop only backgrounded the `jid_lookup`/`jobs_list` commands onto their own thread — every other command (`list_processes`, `get_zmq_connections`, `minion_cache_list`, etc., all sent on every tick) ran inline, blocking the loop from even reading the next stdin line until that command finished. On a busy master, a job-detail lookup could sit unread behind a backlog of those for tens of seconds despite finishing in a few seconds once actually dispatched. Every agent command now runs on its own thread, with the NDJSON stdout writer locked so concurrent writers can't interleave into an unparseable line.

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
