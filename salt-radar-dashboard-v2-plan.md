# salt-radar — Dashboard enhancements (addendum to the original plan)

This is a focused follow-up brief for Claude Code, to be applied on top of the
existing v0.1.0 codebase (built from `salt-radar-design-plan.md`). It covers
three Dashboard-tab changes and one explicit removal.

---

## 0. Dropped: "Fleet & version drift" panel

Getting an accurate per-minion Salt version requires either a fresh grains
refresh or `test.version` against each minion — an active probe of the whole
fleet, which conflicts with the non-intrusive design goal. Cached grains data
(`saltversion`) can be stale for minions that were upgraded but haven't
re-authed/restarted since, so displaying it would be misleading rather than
just incomplete.

**Action**: remove any scaffolding for this panel if present. Leave a short
code comment noting it's deferred pending a possible future *opt-in*
active-probe mode (out of scope for now, and would need its own clearly
separate "this triggers fleet-wide commands" framing if ever added).

---

## 1. Dynamic salt-* systemd service discovery

Replace any hardcoded list of units (`salt-master`, `salt-api`,
`salt-syndic`, ...) in `collectors/services.rs` with a discover-then-poll
pattern, so the panel only ever shows services that actually exist on the
host.

### Discovery (low frequency — on startup, then every ~60s)
- Enumerate installed `salt-*.service` unit files via
  `systemctl list-unit-files --no-legend 'salt-*.service'` (verify exact
  flags/output columns against the systemd version on the target host —
  output format has shifted slightly across systemd releases).
- This lists units whose **unit files exist**, regardless of whether they're
  currently enabled or active — a host with `salt-syndic` never installed
  simply won't appear, which is the desired behavior.
- Strip the `.service` suffix to get unit names, e.g. `["salt-master",
  "salt-api"]`. Re-run this discovery periodically in case packages are
  added/removed while salt-radar is running — cheap and infrequent.

### Status polling (every dashboard refresh, e.g. ~2s)
- For each discovered unit, run
  `systemctl show <unit> --property=ActiveState,SubState,ActiveEnterTimestamp,InactiveEnterTimestamp`
  and parse the `Key=Value` lines (don't use `--value`, which drops the keys
  and makes multi-property output ambiguous to parse).
- Parse `ActiveEnterTimestamp` (systemd's human timestamp format, e.g. `Tue
  2026-06-15 10:21:33 UTC`) and compute `now - ActiveEnterTimestamp` as the
  "up for" duration when `ActiveState == "active"`.
- If not active, use `InactiveEnterTimestamp` for a "down for" duration where
  present; if neither timestamp is parseable (unit never transitioned this
  boot), show the state with no duration rather than failing.

### Data model
```rust
struct ServiceStatus {
    unit: String,            // "salt-master", "salt-api", ...
    active: bool,
    state_text: String,      // "active" / "inactive" / "failed" / ...
    since: Option<Duration>, // time in current state, if known
}
// collectors::services::poll() -> Vec<ServiceStatus>, length varies by host
```

### Rendering — Processes & Services panel (ui/dashboard.rs)
One inline segment per discovered service:
```
● salt-master  up 14d 02h    ● salt-api  up 14d 02h    ○ salt-minion  down 3h
```
- `●` (green) when active, `○` (gray/red depending on `state_text`) when not.
- A master-only host with no `salt-api`/`salt-syndic` installed shows just
  the one segment — no placeholders for absent services.
- If the segments don't fit on one line, wrap to a second line within the
  same panel rather than truncating service names.

---

## 2. Host uptime alongside application uptime

Two different "uptime" concepts, both worth showing, clearly labeled so
there's no ambiguity about which is which.

### Host (OS) uptime — collectors/system.rs
- Read `/proc/uptime` (first whitespace-separated field = seconds since
  boot). Pure filesystem read, no subprocess.
- Format as `Xd Yh Zm`, dropping leading-zero units (e.g. `14d 02h`, `3h
  12m`, `42m`).

### Application uptime
- Already implemented (currently shown as bare `uptime: 2s` in the header).
  No new data needed — just relabel for clarity alongside the host value.

### Header rendering
```
salt-radar v0.1.0 ● ● Salt 3006.16 | keys: A1 P0 | radar 2s | host 14d 03h
```
If the header gets too wide on narrow terminals, abbreviate further (e.g.
drop to `r 2s | h 14d`) rather than dropping one of the two values.

---

## 3. Minion activity (60s) — passive checkin sparkline

### Rationale
Per-minion "last seen" tracking is already planned for the Minions tab
(derived from minion-cache mtimes plus any event mentioning a minion id
during the session — see original plan, Minions Explorer / last-seen
sections). This panel is just a live, dashboard-level view of that same
underlying state. No new data source, no active probing — purely a
consequence of minions periodically generating events on their own (per your
~5 minute checkin cadence).

### Data model additions (shared app state)
```rust
struct MinionActivity {
    last_seen: HashMap<String, Instant>, // minion_id -> last event seen this session
    checkin_buckets: VecDeque<u32>,       // rolling 60s, e.g. 30 buckets x 2s
}
```
- On every event from the agent's event stream (the same subscription used
  for the Radar tab — this is an additional consumer of that stream, not a
  new one), extract a minion id where possible: `data['id']`,
  `data['minion_id']`, or the trailing segment of tags like
  `salt/job/*/ret/<minion>` or `salt/minion/<id>/start`.
- Update `last_seen[minion_id] = now` and increment the current time bucket
  in `checkin_buckets`.
- Roll the bucket window forward on the same ~2s cadence as the dashboard
  refresh (drop oldest bucket, push a new empty one).

### Breakdown counts
Against the full known-minion set (same union of key-directory + minion-cache
ids used by the Minions tab):
- **active** — `last_seen` within `active_secs` (default 300s, matching the
  known ~5 min checkin interval)
- **quiet** — older than `active_secs` but within `quiet_secs` (default
  1800s)
- **silent** — never seen this session, or older than `quiet_secs`

### Rendering — Minion activity panel (ui/dashboard.rs)
```
checkins/s  ▂▃▃▄▃▃▄▃▃▃▄▃▃▃▃▄▃▃▃▃▄▃     3792 active   16 quiet >5m   4 silent
```
- Sparkline from `checkin_buckets` — count of qualifying events per bucket
  (not distinct minions; simpler, and still visually meaningful as an
  activity indicator). If distinct-minions-per-bucket is wanted later, that
  needs a `HashSet<String>` per bucket instead of a counter — note this as a
  possible v3 refinement, not needed now.
- The three counts come from the breakdown above.

### Config additions (salt-radar.toml)
```toml
[minion_activity]
active_secs = 300
quiet_secs  = 1800
bucket_secs = 2
window_secs = 60
```

---

## Implementation order

1. `collectors/system.rs` — add host uptime from `/proc/uptime`.
2. Header rendering — show `radar <uptime> | host <uptime>`.
3. `collectors/services.rs` — rewrite to discover-then-poll; drop any
   hardcoded unit list.
4. `ui/dashboard.rs` — render the per-service line dynamically (1..N
   segments, wrapping as needed).
5. App state — add `MinionActivity`, wire the existing event-stream handler
   to also update `last_seen` + `checkin_buckets`.
6. `ui/dashboard.rs` — render the Minion activity panel (sparkline +
   breakdown).
7. Remove/comment-out any "Fleet & version drift" scaffolding per §0.
8. Add `[minion_activity]` section + defaults to `salt-radar.toml`.
