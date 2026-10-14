#!/usr/bin/env python3
"""
salt-radar-agent — Python helper for salt-radar TUI.

Communicates with the Rust process via NDJSON over stdin/stdout.
Reads JSON-command lines from stdin, writes NDJSON responses to stdout.
"""

from __future__ import print_function

import fnmatch
import json
import os
import re
import sys
import threading
import time
import traceback
import socket

# ── Salt imports (these must succeed — run with Salt's Python) ──────────────

def _import_salt():
    import salt.config
    import salt.version
    import salt.utils.event
    import salt.cache
    import salt.runner
    import salt.client
    import salt.pillar
    return salt


# ── Saved stdout fd (immune to fd-level redirects) ─────────────────────────
# Initialized in main() via os.dup(1).  ndjson() writes through this fd so
# that the event subscriber thread is unaffected by any fd-1 redirect (e.g.
# fleet scan suppressing Salt warnings).
_OUTPUT_FD = None


def _init_output():
    global _OUTPUT_FD
    _OUTPUT_FD = os.dup(1)


# ── Helpers ─────────────────────────────────────────────────────────────────

_NDJSON_LOCK = threading.Lock()


def ndjson(obj):
    """Write a JSON line to the saved output fd and flush immediately.

    Every command now dispatches on its own thread (see `dispatch`), so
    concurrent writers are the norm rather than the exception — a single
    `os.write()` call is atomic only up to PIPE_BUF (4096 bytes on Linux);
    anything larger (a big job return, a large grains dump, ...) written
    by two threads at once could otherwise interleave into a line neither
    side can parse. The lock keeps each message's bytes contiguous.
    """
    global _OUTPUT_FD
    fd = _OUTPUT_FD if _OUTPUT_FD is not None else 1
    data = json.dumps(obj, default=str) + "\n"
    with _NDJSON_LOCK:
        os.write(fd, data.encode())


def result(id_val, cmd, ok=True, data=None):
    if data is None:
        data = {}
    ndjson({"id": id_val, "type": "result", "cmd": cmd, "ok": ok, "data": data})


def error(id_val, message):
    ndjson({"type": "error", "id": id_val, "message": message})


def event(tag, data, ts=None):
    if ts is None:
        ts = time.time()
    ndjson({"type": "event", "tag": tag, "data": data, "ts": ts})


def get_opts(master_config):
    """Load and return Salt master opts."""
    salt = _import_salt()
    opts = salt.config.master_config(master_config)
    return opts


def _run_salt_cmd(args, timeout=30, env=None):
    """Run a salt command directly (baremetal/VM — agent has local filesystem access)."""
    import subprocess as sp
    try:
        cp = sp.run(args, capture_output=True, text=True, timeout=timeout, env=env)
        if cp.returncode == 0 and cp.stdout.strip():
            return cp.stdout
    except Exception:
        pass
    return None


def _key_mtime(opts, subdir, mid):
    try:
        path = os.path.join(opts.get("pki_dir", "/etc/salt/pki"), "master", subdir, mid)
        return int(os.path.getmtime(path))
    except OSError:
        return 0


def cmd_list_keys(opts, req):
    """Fetch minion key data — salt.key.Key first, then salt-key -L fallback."""
    keys = []
    try:
        salt = _import_salt()
        sk = salt.key.Key(opts)
        key_dict = sk.list_keys()
        status_map = {
            "minions": "Accepted",
            "minions_pre": "Pending",
            "minions_rejected": "Rejected",
            "minions_denied": "Denied",
        }
        for subdir, status in status_map.items():
            for mid in key_dict.get(subdir, []):
                keys.append({
                    "id": mid,
                    "status": status,
                    "mtime_epoch": _key_mtime(opts, subdir, mid),
                })
    except Exception:
        pass

    # Fallback to salt-key CLI if Python API failed
    if not keys:
        import subprocess as sp, json as json_mod
        try:
            output = _run_salt_cmd(["salt-key", "-L", "--out=json"], timeout=15)
            if output:
                key_data = json_mod.loads(output)
                for mid in key_data.get("minions", []):
                    keys.append({"id": mid, "status": "Accepted", "mtime_epoch": _key_mtime(opts, "minions", mid)})
                for mid in key_data.get("minions_pre", []):
                    keys.append({"id": mid, "status": "Pending", "mtime_epoch": _key_mtime(opts, "minions_pre", mid)})
                for mid in key_data.get("minions_rejected", []):
                    keys.append({"id": mid, "status": "Rejected", "mtime_epoch": _key_mtime(opts, "minions_rejected", mid)})
                for mid in key_data.get("minions_denied", []):
                    keys.append({"id": mid, "status": "Denied", "mtime_epoch": _key_mtime(opts, "minions_denied", mid)})
        except Exception:
            pass

    result(req["id"], "list_keys", data={"keys": keys})


def cmd_list_processes(opts, req):
    processes = []
    # Simple /proc scanner to match Rust implementation
    try:
        for pid_dir in os.listdir("/proc"):
            if pid_dir.isdigit():
                pid = int(pid_dir)
                try:
                    # /proc/<pid>/comm is just the truncated interpreter name
                    # (e.g. "/opt/saltstack/") for onedir-packaged Salt — the
                    # real process title Salt sets (MWorker-0, EventPublisher,
                    # ...) only shows up in cmdline.
                    with open(os.path.join("/proc", pid_dir, "cmdline"), "rb") as f:
                        cmdline = f.read().replace(b"\0", b" ").decode("utf-8", "replace")
                    with open(os.path.join("/proc", pid_dir, "stat"), "r") as f:
                        content = f.read()
                        # comm (field 2) is parenthesized and can itself contain
                        # spaces or parens (e.g. "(sd-pam)"), so locate it by the
                        # last ')' rather than naive whitespace splitting -- a
                        # fixed-index split silently misaligns every field after
                        # it for any process whose name has embedded whitespace.
                        rest = content[content.rindex(")") + 2:].split()
                        # rest[0] is field 3 (state); rss is field 24 -> index 21
                        rss_kb = int(rest[21]) * 4
                        # utime (field 14) + stime (field 15) -> indices 11, 12
                        cpu_ticks = int(rest[11]) + int(rest[12])

                    # Determine role. Every real salt-* process (including
                    # its sub-workers/threads: MWorker-N, EventPublisher,
                    # ReqServer, FileserverUpdate, Maintenance, ...) still
                    # carries "salt-master"/"salt-minion"/"salt-api"/
                    # "salt-syndic" somewhere in its cmdline, so a process
                    # matching none of these isn't an unclassified Salt
                    # process -- it's an unrelated process on the host
                    # (this scan walks all of /proc, not just Salt's PIDs)
                    # and must be skipped, not counted as "Unknown".
                    if "MWorker-" in cmdline: role = "Worker"
                    elif "EventPublisher" in cmdline: role = "EventPublisher"
                    elif "ReqServer" in cmdline: role = "ReqServer"
                    elif "salt-master" in cmdline: role = "Master"
                    elif "salt-api" in cmdline: role = "API"
                    elif "salt-syndic" in cmdline: role = "Syndic"
                    elif "salt-minion" in cmdline: role = "Minion"
                    else:
                        continue

                    processes.append({
                        "pid": pid,
                        "role": role,
                        "rss_kb": rss_kb,
                        "cpu_ticks": cpu_ticks,
                    })
                except (OSError, IndexError, ValueError):
                    continue
    except OSError:
        pass
    result(req["id"], "list_processes", data={"processes": processes, "clk_tck": os.sysconf("SC_CLK_TCK")})


def cmd_list_services(opts, req):
    services = []
    try:
        import subprocess as sp
        import os

        # Get all salt-* service properties from systemd in one call
        cp = sp.run(
            ["systemctl", "show", "salt-*.service",
             "--property=Id,ActiveState,SubState,MainPID,ActiveEnterTimestamp,MemoryCurrent"],
            capture_output=True, text=True, timeout=10
        )

        if cp.returncode == 0 and cp.stdout.strip():
            # Each service block is separated by a blank line
            for block in cp.stdout.strip().split("\n\n"):
                props = {}
                for line in block.strip().split("\n"):
                    if "=" in line:
                        k, v = line.split("=", 1)
                        props[k] = v

                unit = props.get("Id", "")
                if not unit:
                    continue

                active_state = props.get("ActiveState", "inactive")
                sub_state = props.get("SubState", "")
                main_pid = int(props.get("MainPID", "0"))

                active = active_state == "active"

                state_map = {
                    "running": "running",
                    "exited": "exited",
                    "dead": "dead",
                    "failed": "failed",
                    "auto-restart": "restarting",
                }
                state = state_map.get(sub_state, sub_state)

                # Memory: prefer the unit's cgroup accounting (MemoryCurrent),
                # which reflects the whole service tree (e.g. salt-master plus
                # all its forked Worker/ReqServer/EventPublisher children) with
                # shared pages counted once -- unlike summing each process's own
                # /proc/<pid>/stat RSS independently (as the process-role panel
                # does), which double-counts memory those forked children share
                # with their parent via copy-on-write. Falls back to the main
                # process's own VmRSS if cgroup accounting isn't available.
                memory_kb = 0
                uptime_seconds = 0
                mem_current = props.get("MemoryCurrent", "")
                if mem_current.isdigit():
                    memory_kb = int(mem_current) // 1024
                elif main_pid > 0:
                    try:
                        with open("/proc/{}/status".format(main_pid), "r") as f:
                            for line in f:
                                if line.startswith("VmRSS:"):
                                    memory_kb = int(line.split()[1])
                                    break
                    except Exception:
                        pass
                if main_pid > 0:
                    try:
                        with open("/proc/{}/stat".format(main_pid), "r") as f:
                            stat_line = f.read()
                        start_ticks = int(stat_line.split(")")[1].strip().split()[19])
                        with open("/proc/uptime", "r") as f:
                            uptime = float(f.read().split()[0])
                        clk_tck = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
                        uptime_seconds = int(uptime - start_ticks / clk_tck)
                    except Exception:
                        pass

                services.append({
                    "unit": unit,
                    "active": active,
                    "state": state,
                    "uptime_seconds": uptime_seconds,
                    "active_enter_timestamp": props.get("ActiveEnterTimestamp", ""),
                    "memory_kb": memory_kb,
                })
    except Exception:
        pass

    # Fallback: /proc scan for salt processes when systemctl unavailable or found nothing
    if not services:
        import os, time
        now = time.time()
        for entry in os.listdir("/proc"):
            if not entry.isdigit():
                continue
            pid = entry
            try:
                with open("/proc/{}/cmdline".format(pid), "r") as f:
                    raw = f.read().replace("\0", " ").strip()
                if not raw or "salt" not in raw:
                    continue
                parts = raw.split()
                binary = parts[0].split("/")[-1] if parts else "?"
                unit = "{}.service".format(binary)
                with open("/proc/{}/stat".format(pid), "r") as f:
                    stat_line = f.read()
                state_char = stat_line.split(")")[1].strip().split()[0] if ")" in stat_line else "?"
                active = state_char not in ("Z", "T", "X")
                state = {"R": "running", "S": "sleeping", "D": "waiting",
                         "Z": "zombie", "T": "stopped", "X": "dead"}.get(state_char, "?")
                rss_kb = 0
                try:
                    with open("/proc/{}/status".format(pid), "r") as f:
                        for line in f:
                            if line.startswith("VmRSS:"):
                                rss_kb = int(line.split()[1])
                                break
                except Exception:
                    pass
                start_ticks = 0
                try:
                    start_ticks = int(stat_line.split(")")[1].strip().split()[19])
                except (IndexError, ValueError):
                    pass
                uptime_seconds = 0
                if start_ticks > 0:
                    try:
                        with open("/proc/uptime", "r") as f:
                            uptime = float(f.read().split()[0])
                        clk_tck = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
                        uptime_seconds = int(uptime - start_ticks / clk_tck)
                    except Exception:
                        pass
                existing = next((s for s in services if s["unit"] == unit), None)
                if existing is None:
                    services.append({
                        "unit": unit,
                        "active": active,
                        "state": state,
                        "uptime_seconds": uptime_seconds,
                        "active_enter_timestamp": "",
                        "memory_kb": rss_kb,
                    })
                elif "MWorker" in raw or "ReqServer" in raw:
                    services.append({
                        "unit": unit,
                        "active": active,
                        "state": state,
                        "uptime_seconds": uptime_seconds,
                        "active_enter_timestamp": "",
                        "memory_kb": rss_kb,
                    })
            except (IOError, OSError, ValueError, IndexError):
                continue

    result(req["id"], "list_services", data={"services": services})


def cmd_get_system_info(opts, req):
    info = {
        "load_1": 0.0, "load_5": 0.0, "load_15": 0.0,
        "mem_total_kb": 0, "mem_avail_kb": 0,
        "cache_disk_usage_pct": 0.0, "pki_disk_usage_pct": 0.0,
        "uptime_secs": 0, "network_rx_bytes": 0, "network_tx_bytes": 0,
        "network_rx_rate": 0.0, "network_tx_rate": 0.0,
        "cpu_total_jiffies": 0, "cpu_idle_jiffies": 0,
    }
    try:
        # Load
        with open("/proc/loadavg", "r") as f:
            load = f.read().split()
            info["load_1"] = float(load[0])
            info["load_5"] = float(load[1])
            info["load_15"] = float(load[2])

        # CPU: cumulative jiffies from the system-wide "cpu " line -- the Rust
        # side derives instantaneous % from the delta between two polls, the
        # same way load average (above) is decayed but nothing here is.
        with open("/proc/stat", "r") as f:
            for line in f:
                if line.startswith("cpu "):
                    fields = [int(x) for x in line.split()[1:]]
                    # user, nice, system, idle, iowait, irq, softirq, steal, ...
                    idle = fields[3] + (fields[4] if len(fields) > 4 else 0)
                    info["cpu_total_jiffies"] = sum(fields)
                    info["cpu_idle_jiffies"] = idle
                    break

        # Mem
        with open("/proc/meminfo", "r") as f:
            for line in f:
                if line.startswith("MemTotal:"):
                    info["mem_total_kb"] = int(line.split()[1])
                elif line.startswith("MemAvailable:"):
                    info["mem_avail_kb"] = int(line.split()[1])

        # Uptime
        with open("/proc/uptime", "r") as f:
            info["uptime_secs"] = int(float(f.read().split()[0]))

        # Disk usage (simple shutil.disk_usage)
        import shutil
        pki_dir = opts.get("pki_dir", "/etc/salt/pki/master")
        cachedir = opts.get("cachedir", "/var/cache/salt/master")

        for path, key in [(cachedir, "cache_disk_usage_pct"), (pki_dir, "pki_disk_usage_pct")]:
            try:
                usage = shutil.disk_usage(path)
                info[key] = (usage.used / usage.total) * 100.0
            except OSError:
                pass

        # Network traffic (cumulative bytes since boot)
        try:
            rx_total = 0
            tx_total = 0
            with open("/proc/net/dev", "r") as f:
                for line in f.readlines()[2:]:
                    line = line.strip()
                    if not line or line.startswith("lo:"):
                        continue
                    parts = line.split()
                    if len(parts) >= 10:
                        rx_total += int(parts[1])
                        tx_total += int(parts[9])
            info["network_rx_bytes"] = rx_total
            info["network_tx_bytes"] = tx_total
        except Exception:
            pass

    except Exception:
        pass
    result(req["id"], "get_system_info", data=info)


# Cached {token: str, eauth: str, username: str} per (scheme, port, eauth,
# username) -- re-login only happens on first use or after the cached token
# is rejected, not on every poll.
_api_token_cache = {}

# Cached compiled pillar dict for "pillar" mode -- compiling pillar is real
# work (rendering SLS through the top file), so this is only redone when
# there's no cached value yet or a login using values resolved from it has
# just failed (credentials rotated in pillar), not on every poll tick
# alongside the (much cheaper) token cache above.
_api_pillar_compiled_cache = None


def _minion_data_bank(mid):
    """The master's minion data cache keeps each minion's grains and pillar
    under key "data" in bank "minions/<id>" (salt/utils/minions.py)."""
    return "minions/{}".format(mid)


def _pillar_get(compiled, path):
    """Walk a colon-delimited path into a compiled pillar dict, the same
    shape `salt-call pillar.get` resolves. Returns None if any segment is
    missing.
    """
    node = compiled
    for key in path.split(":"):
        if not isinstance(node, dict) or key not in node:
            return None
        node = node[key]
    return node


def _compile_stats_pillar(opts, force_refresh=False):
    """Compile pillar master-side for this master's own local minion ID --
    the same one `salt-call pillar.get` would use on this box, resolved the
    same way Salt itself resolves it (cached /etc/salt/minion_id, else the
    configured/guessed id) -- not a live round-trip, and not a minion the
    user has to name in salt-radar.toml: it's whichever minion this master
    already is. Scoped naturally to just this host, matching how a
    top.sls entry keyed to this master's own minion ID would only ever be
    compiled for requests made on this box. If this master has no local
    minion at all, id resolution falls back to a guessed FQDN/IP that
    almost certainly matches nothing in your top.sls -- pillar mode
    requires a minion co-located with the master.

    Cached across polls; force_refresh re-renders (e.g. a login with
    previously-resolved values just failed, so they may have rotated in
    pillar since).
    """
    global _api_pillar_compiled_cache
    if not force_refresh and _api_pillar_compiled_cache is not None:
        return _api_pillar_compiled_cache, None
    try:
        salt = _import_salt()
        minion_id, _ = salt.config.get_id(opts)
        cache = salt.cache.factory(opts)
        grains = (cache.fetch(_minion_data_bank(minion_id), "data") or {}).get("grains", {})
        pil = salt.pillar.get_pillar(opts, grains, minion_id, saltenv=opts.get("saltenv") or "base")
        compiled = pil.compile_pillar()
        _api_pillar_compiled_cache = compiled
        return compiled, None
    except Exception as exc:
        return None, "pillar compilation failed: {}".format(exc)


def _resolve_api_stats_auth(opts, req, force_refresh=False):
    """Resolve (username, password) for authenticated /stats access, per
    api_stats_auth_mode ("plaintext" reads them straight off the request;
    "pillar" compiles pillar master-side -- no live round-trip to any
    minion -- and reads two independent paths out of it, cached until a
    login with the resolved values fails). Returns None on "none" mode or
    on any resolution failure (bad pillar path, etc.), logging why via the
    returned error string.
    """
    mode = req.get("auth_mode") or "none"
    if mode == "none":
        return None, None, None
    if mode == "plaintext":
        username = req.get("auth_username") or ""
        password = req.get("auth_password") or ""
        if not username or not password:
            return None, None, "stats_auth_username/stats_auth_password not set"
        return username, password, None

    if mode == "pillar":
        username_path = req.get("auth_username_pillar_path") or ""
        password_path = req.get("auth_password_pillar_path") or ""
        if not username_path or not password_path:
            return None, None, "stats_auth_username_pillar_path/stats_auth_password_pillar_path not set"
        compiled, err = _compile_stats_pillar(opts, force_refresh=force_refresh)
        if err:
            return None, None, err
        username = _pillar_get(compiled, username_path)
        password = _pillar_get(compiled, password_path)
        if not username or not password:
            return None, None, "pillar paths {!r}/{!r} did not both resolve to values".format(username_path, password_path)
        return username, password, None

    return None, None, "unknown stats_auth_mode {!r}".format(mode)


def _login_api_token(scheme, port, eauth, username, password):
    """POST /login and return a fresh token, or None on failure."""
    import urllib.request
    import urllib.error
    import ssl
    import json as json_mod

    ctx = None
    if scheme == "https":
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE

    url = "{}://127.0.0.1:{}/login".format(scheme, port)
    body = json_mod.dumps({"username": username, "password": password, "eauth": eauth}).encode("utf-8")
    http_req = urllib.request.Request(
        url, data=body,
        headers={"Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(http_req, timeout=5, context=ctx) as resp:
            login_body = resp.read().decode("utf-8", "replace")
        parsed = json_mod.loads(login_body)
        return parsed["return"][0]["token"]
    except Exception:
        return None


def _get_api_token(scheme, port, eauth, username, password, force_relogin=False):
    """Cached token for (scheme, port, eauth, username), logging in only
    when there's no cached token yet, the credentials changed, or the
    caller asks for a forced re-login (a previous request with the cached
    token came back 401 -- Salt tokens expire server-side on their own
    schedule that this agent has no visibility into).
    """
    key = (scheme, port, eauth, username)
    cached = _api_token_cache.get(key)
    if cached and not force_relogin:
        return cached
    token = _login_api_token(scheme, port, eauth, username, password)
    if token:
        _api_token_cache[key] = token
        return token
    _api_token_cache.pop(key, None)
    return None


def _ensure_api_token(opts, req, scheme, port, eauth, force_relogin):
    """Resolve credentials and get a usable token, one attempt with
    whatever's cached plus one retry with freshly-resolved credentials if
    that fails and the mode is "pillar" (a rejected login there could mean
    the cached pillar-resolved password is stale, not just that the token
    expired) -- covers both the first-ever login and the post-401 retry in
    cmd_get_api_stats with the same fallback behavior. Returns (token, None)
    or (None, error_message).
    """
    username, password, auth_err = _resolve_api_stats_auth(opts, req)
    if auth_err:
        return None, "auth: {}".format(auth_err)
    token = _get_api_token(scheme, port, eauth, username, password, force_relogin=force_relogin)
    if not token and req.get("auth_mode") == "pillar":
        username, password, auth_err = _resolve_api_stats_auth(opts, req, force_refresh=True)
        if auth_err:
            return None, "auth: {}".format(auth_err)
        token = _get_api_token(scheme, port, eauth, username, password, force_relogin=True)
    if not token:
        return None, "salt-api login failed (check stats_auth_eauth/credentials)"
    return token, None


def cmd_get_api_stats(opts, req):
    """Poll salt-api's built-in in-memory request stats (rest_cherrypy's
    `collect_stats` option, backed by CherryPy's cpstats tool) over its own
    /stats endpoint.

    This is deliberately not a log file: cpstats records request line
    (method+path, never the body), client, response status, and timing
    entirely in the running salt-api process's memory (`logging.statistics`,
    just reusing the `logging` module as a data container -- nothing is
    written to disk), independent of Salt's own log_level. A request body
    can carry credentials (e.g. POST /login); the request *line* CherryPy
    records here never does.

    Two ways to access /stats, per api_stats_auth_mode (see AppConfig):
    "none" (default) relies on `rest_cherrypy: {collect_stats: True,
    stats_disable_auth: True}` in the master config -- /stats needs no
    credentials at all, see the README's security note on that tradeoff.
    "plaintext"/"pillar" instead log in via eauth (stats_disable_auth can
    stay unset/False) and send the resulting token as X-Auth-Token --
    slower per the extra request, but no unauthenticated endpoint.
    """
    rest_cherrypy = opts.get("rest_cherrypy") or {}
    if not rest_cherrypy:
        result(req["id"], "get_api_stats", data={"configured": False})
        return

    port = rest_cherrypy.get("port", 8000)
    scheme = "http" if rest_cherrypy.get("disable_ssl", False) else "https"
    url = "{}://127.0.0.1:{}/stats".format(scheme, port)

    import urllib.request
    import urllib.error
    import ssl
    import json as json_mod

    auth_mode = req.get("auth_mode") or "none"
    eauth = req.get("auth_eauth") or "pam"
    auth_headers = {"Accept": "application/json"}
    if auth_mode != "none":
        token, auth_err = _ensure_api_token(opts, req, scheme, port, eauth, force_relogin=False)
        if auth_err:
            result(req["id"], "get_api_stats", data={
                "configured": True, "reachable": False, "port": port, "scheme": scheme,
                "error": auth_err,
            })
            return
        auth_headers["X-Auth-Token"] = token

    ctx = None
    if scheme == "https":
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE

    def _fetch(headers):
        http_req = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(http_req, timeout=5, context=ctx) as resp:
            return resp.read().decode("utf-8", "replace")

    try:
        body = _fetch(auth_headers)
    except urllib.error.HTTPError as exc:
        # A cached token can go stale server-side (Salt's own token_expire,
        # or the master having restarted) with no way for this agent to
        # know ahead of time -- one re-login-and-retry before giving up.
        if exc.code == 401 and auth_mode != "none":
            token, auth_err = _ensure_api_token(opts, req, scheme, port, eauth, force_relogin=True)
            if auth_err:
                result(req["id"], "get_api_stats", data={
                    "configured": True, "reachable": False, "port": port, "scheme": scheme,
                    "error": "{} (after 401)".format(auth_err),
                })
                return
            auth_headers["X-Auth-Token"] = token
            try:
                body = _fetch(auth_headers)
            except Exception as exc2:
                result(req["id"], "get_api_stats", data={
                    "configured": True, "reachable": False, "port": port, "scheme": scheme,
                    "error": str(exc2),
                })
                return
        else:
            result(req["id"], "get_api_stats", data={
                "configured": True, "reachable": False, "port": port, "scheme": scheme,
                "error": str(exc),
            })
            return
    except Exception as exc:
        result(req["id"], "get_api_stats", data={
            "configured": True, "reachable": False, "port": port, "scheme": scheme,
            "error": str(exc),
        })
        return

    stats = json_mod.loads(body)

    app_stats = stats.get("CherryPy Applications", {}) if isinstance(stats, dict) else {}
    requests = []
    for tid, rec in app_stats.get("Requests", {}).items():
        requests.append({
            "thread_id": str(tid),
            "client": rec.get("Client", ""),
            "request_line": rec.get("Request-Line", ""),
            "response_status": rec.get("Response Status") or "",
            "processing_time_secs": rec.get("Processing Time") or 0.0,
            # Epoch seconds this request finished -- lets the Rust side tell
            # whether a given worker thread has completed a *new* request
            # since the last poll (cpstats only ever keeps the latest
            # request per thread, so this is how we dedupe across polls
            # instead of re-counting the same still-latest one every time).
            "end_time": rec.get("End Time") or 0.0,
        })

    result(req["id"], "get_api_stats", data={
        "configured": True,
        "reachable": True,
        "port": port,
        "scheme": scheme,
        "current_requests": app_stats.get("Current Requests", 0),
        "total_requests": app_stats.get("Total Requests", 0),
        "requests_per_sec": app_stats.get("Requests/Second", 0.0),
        "bytes_read_per_sec": app_stats.get("Bytes Read/Second", 0.0),
        "bytes_written_per_sec": app_stats.get("Bytes Written/Second", 0.0),
        "uptime_secs": app_stats.get("Uptime", 0.0),
        "requests": requests,
    })


def cmd_get_zmq_connections(opts, req):
    connections = []
    try:
        with open("/proc/net/tcp", "r") as f:
            lines = f.readlines()[1:]
            for line in lines:
                parts = line.split()
                if len(parts) < 4: continue

                # Local address: hex format IP:port
                local_raw = parts[1].split(':')
                if len(local_raw) != 2: continue

                local_port = int(local_raw[1], 16)
                remote_raw = parts[2].split(':')
                if len(remote_raw) != 2: continue

                remote_ip_hex = remote_raw[0]
                remote_port = int(remote_raw[1], 16)

                # Convert hex IP to dotted quad
                import socket, struct
                ip_bytes = struct.pack("<L", int(remote_ip_hex, 16))
                remote_addr = socket.inet_ntoa(ip_bytes)

                state = parts[3]
                # Map hex state to string
                state_map = {"01": "ESTABLISHED", "02": "SYN_SENT", "03": "SYN_RECV", "04": "FIN_WAIT1"}
                state_str = state_map.get(state, state)

                connections.append({
                    "local_port": local_port,
                    "remote_addr": remote_addr,
                    "remote_port": remote_port,
                    "state": state_str,
                })
    except Exception:
        pass
    result(req["id"], "get_zmq_connections", data={"connections": connections})


# ── Command handlers ────────────────────────────────────────────────────────



def cmd_version(opts, req):
    salt = _import_salt()
    ver = getattr(salt.version, "__version__", "?")
    pyver = "{}.{}.{}".format(sys.version_info.major, sys.version_info.minor, sys.version_info.micro)
    result(req["id"], "version", data={
        "salt_version": ver,
        "python_version": pyver,
    })


def _detect_api_port(opts):
    """Try to find salt-api listen port from config files."""
    import os, re
    paths = ["/etc/salt/master", "/etc/salt/salt-api"]
    if os.path.isdir("/etc/salt/master.d"):
        try:
            for fn in sorted(os.listdir("/etc/salt/master.d")):
                if fn.endswith(".conf"):
                    paths.append(os.path.join("/etc/salt/master.d", fn))
        except OSError:
            pass

    for path in paths:
        try:
            with open(path) as f:
                in_rest = False
                for line in f:
                    stripped = line.strip()
                    if re.match(r"^\s*rest_\w+:", stripped):
                        in_rest = True
                    elif stripped and not stripped.startswith("#") and not stripped[0].isspace() and ":" in stripped:
                        # A top-level key ended the rest section
                        in_rest = False
                    if in_rest:
                        m = re.match(r"^\s*port:\s*(\d+)\s*$", stripped)
                        if m:
                            return int(m.group(1))
        except (OSError, IOError):
            continue
    return 0

def cmd_config_snapshot(opts, req):
    # Get actual hostname instead of just interface
    try:
        hostname = socket.gethostname()
    except Exception:
        hostname = opts.get("interface", "0.0.0.0")

    data = {
        "salt_version": str(getattr(_import_salt().version, "__version__", "?")),
        "python_version": "{}.{}.{}".format(
            sys.version_info.major, sys.version_info.minor, sys.version_info.micro
        ),
        "worker_threads": opts.get("worker_threads"),
        "minimum_auth_version": opts.get("minimum_auth_version", 0),
        "transport": opts.get("transport", "zeromq"),
        "sock_dir": opts.get("sock_dir", ""),
        "reactor_count": len(opts.get("reactor", []) if isinstance(opts.get("reactor"), list) else []),
        "fileserver_backends": list(opts.get("fileserver_backend", [])),
        "gitfs_remote_count": len(opts.get("gitfs_remotes", [])),
        "ext_pillar_names": (
            list(opts.get("ext_pillar", {}).keys())
            if isinstance(opts.get("ext_pillar"), dict)
            else []
        ),
        "master_job_cache": opts.get("master_job_cache", "local"),
        "pillar_cache": bool(opts.get("pillar_cache", False)),
        "presence_events": bool(opts.get("presence_events", False)),
        "interface": opts.get("interface", "0.0.0.0"),
        "publish_port": opts.get("publish_port", 4505),
        "ret_port": opts.get("ret_port", 4506),
        "api_port": _detect_api_port(opts),
        "pki_dir": opts.get("pki_dir", "/etc/salt/pki/master"),
        "cachedir": opts.get("cachedir", "/var/cache/salt/master"),
        "cpu_count": opts.get("cpu_count", os.cpu_count() or 0),
        "hostname": hostname,
    }
    result(req["id"], "config_snapshot", data=data)


# ── Master configuration browser ────────────────────────────────────────────

# Last parts of a setting's name that mark its value as a credential
# (`mysql.pass`, `gitfs_password`, `vault: {token: ...}`, `secret_key`).
_SECRET_LAST_PARTS = {
    "pass", "passwd", "password", "passphrase", "pw", "bindpw", "secret",
    "token", "apikey", "key", "credential", "credentials", "creds", "priv",
}
_SECRET_ANYWHERE = ("password", "passwd", "secret")
_REDACTED = "<redacted>"
_UNREADABLE = "<unreadable: the file isn't valid YAML>"


def _is_secret_name(name):
    name = str(name).lower()
    if any(s in name for s in _SECRET_ANYWHERE):
        return True
    return re.split(r"[._\-]", name)[-1] in _SECRET_LAST_PARTS


def _is_secret_value(value):
    """A value worth hiding: text or a number, but not a flag, an empty value
    or a filesystem path (`ssl_key: /etc/pki/key.pem` names a file, it isn't
    the key)."""
    if isinstance(value, bool) or value is None:
        return False
    if isinstance(value, (int, float)):
        return True
    if isinstance(value, str):
        return bool(value.strip()) and not value.startswith(("/", "~"))
    return False


def _plain(value):
    """Salt's defaults are frozen (immutabletypes), which neither compare equal
    to the plain lists and dicts master_config() builds nor dump as YAML."""
    from collections.abc import Mapping
    if isinstance(value, Mapping):
        return {k: _plain(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)) or type(value).__name__ == "ImmutableList":
        return [_plain(v) for v in value]
    return value


def _redact(value):
    """Copy of a config value with credential-looking entries replaced, at any
    depth (ext_pillar lists, rest_cherrypy dicts, ...)."""
    if isinstance(value, dict):
        return {
            k: (_REDACTED if _is_secret_name(k) and _is_secret_value(v) else _redact(v))
            for k, v in value.items()
        }
    if isinstance(value, (list, tuple)):
        return [_redact(v) for v in value]
    return value


_YAML_ENTRY = re.compile(r"^(\s*(?:-\s+)?)([\"']?)([^\s:\"'#][^:\"'#]*)\2(\s*:\s*)(.*)$")


def _redact_text(text):
    """A config file's text with credential values blanked line by line,
    including the indented lines of a block scalar (`password: |`)."""
    out = []
    block_indent = None
    for line in text.splitlines():
        if block_indent is not None:
            indent = len(line) - len(line.lstrip())
            if line.strip() and indent > block_indent:
                out.append(" " * indent + _REDACTED)
                continue
            block_indent = None
        m = _YAML_ENTRY.match(line)
        if m and _is_secret_name(m.group(3)):
            value = m.group(5).split(" #")[0].strip()
            if value in ("|", ">", "|-", ">-", "|+", ">+"):
                block_indent = len(m.group(1))
                out.append(line)
                continue
            unquoted = value.strip("\"'")
            if unquoted and unquoted.lower() not in ("true", "false", "yes", "no", "on", "off", "null", "~") \
                    and not unquoted.startswith(("/", "~", "{", "[")):
                out.append("{}{}{}{}{}{}".format(m.group(1), m.group(2), m.group(3), m.group(2), m.group(4), _REDACTED))
                continue
        out.append(line)
    return "\n".join(out)


def _config_text(value, flow):
    import salt.utils.yaml
    try:
        if flow:
            text = salt.utils.yaml.safe_dump(value, default_flow_style=True, width=1 << 30)
        else:
            text = salt.utils.yaml.safe_dump(value, default_flow_style=False)
        text = text.strip()
        if text.endswith("\n..."):
            text = text[:-4].strip()
        elif text.endswith("..."):
            text = text[:-3].strip()
        return text
    except Exception:
        return repr(value)


def _top_level_blocks(text):
    """{key: (start, end)} line ranges (1-based, end exclusive) of a YAML
    file's top-level entries; trailing blank and comment lines aren't part
    of a block."""
    lines = text.splitlines()
    starts = []
    for i, line in enumerate(lines):
        if not line or line[0] in " \t#-" or line.startswith("..."):
            continue
        m = re.match(r"^([\"']?)([^:\"']+)\1\s*:", line)
        if m:
            starts.append((m.group(2).strip(), i))
    blocks = {}
    for n, (key, start) in enumerate(starts):
        end = starts[n + 1][1] if n + 1 < len(starts) else len(lines)
        while end > start + 1 and (not lines[end - 1].strip() or lines[end - 1].lstrip().startswith("#")):
            end -= 1
        blocks[key] = (start + 1, end + 1)
    return blocks


def _config_files(path):
    """Salt's load order (salt.config.master_config) as (path, data, error,
    group): the main file, then `default_include` (master.d/*.conf, sorted),
    then the main file's `include`; an included file's own `include` follows
    it in its group. A file Salt rejects (e.g. a key twice) contributes
    nothing to it, so its data here is a lenient read, for showing what was
    ignored."""
    import glob
    import yaml
    salt = _import_salt()
    order = []

    def read(fn):
        try:
            return salt.config._read_conf_file(fn) or {}, None
        except Exception as exc:
            try:
                with open(fn) as f:
                    text = f.read()
            except OSError:
                return {}, str(exc)
            try:
                data = yaml.safe_load(text) or {}
            except Exception:
                # Not YAML at all: still name the settings it tries to set.
                data = {key: _UNREADABLE for key in _top_level_blocks(text)}
            return (data if isinstance(data, dict) else {}), str(exc)

    def add_includes(patterns, base, group):
        if isinstance(patterns, str):
            patterns = [patterns]
        for pattern in patterns or []:
            pattern = os.path.expanduser(pattern)
            if not os.path.isabs(pattern):
                pattern = os.path.join(os.path.dirname(base), pattern)
            for fn in sorted(glob.glob(pattern)):
                if fn in [o[0] for o in order]:
                    continue
                data, err = read(fn)
                order.append((fn, data, err, group))
                if not err:
                    add_includes(data.get("include"), fn, group)

    main, err = read(path)
    order.append((path, main, err, "main"))
    if not err:
        add_includes(main.get("default_include", salt.config.DEFAULT_MASTER_OPTS["default_include"]), path, "default_include")
        add_includes(main.get("include"), path, "include")
    return order


def _combine(base, value):
    """How Salt combines one included file's value into the ones before it
    in the same group (salt.utils.dictupdate.update with merge_lists)."""
    import copy
    if isinstance(base, dict) and isinstance(value, dict):
        merged = copy.deepcopy(base)
        for k, v in value.items():
            merged[k] = _combine(merged[k], v) if k in merged else copy.deepcopy(v)
        return merged
    if isinstance(base, list) and isinstance(value, list):
        return base + value
    return value


def _source_relations(key, sources, order):
    """Each file's part in a setting, in load order: `first`; `overrides`
    (a later file's scalar wins); `merged` (dicts and lists from files of one
    include group combine); `replaces` (a later group, e.g. master.d over the
    main file, replaces the whole value: nested keys it doesn't set are
    dropped, and listed); `ignored` (in a file Salt rejected)."""
    out = []
    group, group_value, prev_group_value = None, None, None
    for src in sources:
        fn, data, err, file_group = order[src["file"]]
        value = _plain(data.get(key))
        if err:
            src["relation"] = "ignored"
        elif group is None:
            src["relation"] = "first"
            group, group_value = file_group, value
        elif file_group != group:
            src["relation"] = "replaces"
            src["replaced"] = len(out)
            prev_group_value, group, group_value = group_value, file_group, value
        elif isinstance(group_value, (dict, list)) and type(group_value) is type(value):
            src["relation"] = "merged"
            group_value = _combine(group_value, value)
        else:
            src["relation"] = "overrides"
            group_value = value
        shown = _redact({key: value})[key]
        src["value"] = shown if shown in (_UNREADABLE, _REDACTED) else _config_text(shown, False)
        out.append(src)
    # What a replacing group dropped, once the group's own files are combined.
    for src in out:
        if src.get("relation") == "replaces" and isinstance(prev_group_value, dict) and isinstance(group_value, dict):
            src["dropped"] = sorted(str(k) for k in prev_group_value if k not in group_value)
        src.pop("replaced", None)
    return out


def _salt_master_started():
    """Epoch seconds the oldest salt-master process started, 0 if none."""
    try:
        with open("/proc/stat") as f:
            btime = next(int(l.split()[1]) for l in f if l.startswith("btime"))
        hz = os.sysconf("SC_CLK_TCK")
        starts = []
        for pid in os.listdir("/proc"):
            if not pid.isdigit():
                continue
            try:
                with open("/proc/{}/cmdline".format(pid), "rb") as f:
                    cmd = f.read().replace(b"\0", b" ")
                if b"salt-master" not in cmd:
                    continue
                with open("/proc/{}/stat".format(pid)) as f:
                    fields = f.read().rsplit(")", 1)[1].split()
                starts.append(btime + int(fields[19]) / hz)
            except (OSError, IndexError, ValueError):
                continue
        return int(min(starts)) if starts else 0
    except Exception:
        return 0


def cmd_master_config(opts, req):
    """Every master setting: where it's set (file and line range, in Salt's
    load order) and how each file's value combines, its effective value and
    Salt's default, plus the files Salt rejected. Credentials are redacted
    here, so they never reach the TUI."""
    import difflib
    salt = _import_salt()
    path = opts.get("conf_file") or "/etc/salt/master"
    defaults = salt.config.DEFAULT_MASTER_OPTS
    effective = salt.config.master_config(path)
    order = _config_files(path)

    files = []
    sources = {}
    for idx, (fn, data, err, _group) in enumerate(order):
        try:
            with open(fn) as f:
                text = f.read()
            mtime = int(os.path.getmtime(fn))
        except OSError as exc:
            text, mtime, err = "", 0, err or str(exc)
        # A YAML error names where the mapping starts first, then the line
        # at fault; its message lines are interleaved with "in <file>" ones.
        lines = re.findall(r"line (\d+)", err or "")
        reason = [l.strip() for l in (err or "").splitlines()[1:] if l.strip() and not l.strip().startswith("in ")]
        files.append({
            "path": fn, "text": _redact_text(text), "mtime": mtime,
            "error": (reason[-1] if reason else err), "error_line": int(lines[-1]) if lines else 0,
        })
        blocks = _top_level_blocks(text)
        for key in data:
            start, end = blocks.get(str(key), (0, 0))
            sources.setdefault(str(key), []).append({"file": idx, "start": start, "end": end})

    settings = []
    for key in sorted(set(effective) | set(sources), key=str):
        if str(key).startswith("__"):
            continue
        srcs = _source_relations(key, sources.get(str(key), []), order)
        in_files = any(s["relation"] != "ignored" for s in srcs)
        known = key in defaults
        actual = _plain(effective.get(key))
        default = _plain(defaults[key]) if known else None
        value = _redact({key: actual})[key]
        if srcs and not in_files:
            kind = "ignored"
        elif in_files and not known:
            kind = "unknown"
        elif in_files:
            kind = "explicit"
        elif known and actual == default:
            kind = "default"
        else:
            kind = "derived"
        entry = {
            "key": str(key),
            "kind": kind,
            "sources": srcs,
            "line": _config_text(value, True),
            "full": _config_text(value, False),
            "default": _config_text(_redact({key: default})[key], False) if known else None,
            "redacted": value != actual,
        }
        if kind == "unknown":
            close = difflib.get_close_matches(str(key), [str(k) for k in defaults], n=1, cutoff=0.8)
            if close:
                entry["suggest"] = close[0]
        settings.append(entry)

    result(req["id"], "master_config", data={
        "conf_file": path,
        "files": files,
        "master_started": _salt_master_started(),
        "settings": settings,
    })


# mid -> {"mtime": int, "grains": {"os": ..., "osrelease": ..., ...}} — see
# cmd_minion_cache_list for why this exists.
_minion_grains_cache = {}

# Cached accepted-minion-key list -- see cmd_minion_cache_list for why this
# is resolved on its own timer instead of every poll.
_key_list_cache = {"key_ids": [], "fetched_at": 0.0}
_KEY_LIST_REFRESH_SECS = 30


def _resolve_key_ids(opts):
    """Return the accepted-minion-key list, refreshed at most once every
    `_KEY_LIST_REFRESH_SECS` rather than on every `minion_cache_list` poll
    (every `dashboard_secs`, 2s by default).

    Tries `salt.key.Key` in-process first (like `cmd_list_keys`), falling
    back to a `salt-key -L` subprocess if that raises or comes back empty —
    `salt.key.Key(opts).list_keys()` is not reliable in every environment
    (this exact fallback already existed in `cmd_list_keys` for the same
    reason). But on a master where the in-process call never works, that
    fallback subprocess forks (a full Python + Salt interpreter startup)
    on *every* poll instead of the one-off it's meant to be — on a fleet
    with thousands of minions this alone reintroduces the CPU cost this
    whole cache exists to avoid. The resolved list is cached for
    `_KEY_LIST_REFRESH_SECS` regardless of which path produced it, since
    the accepted-key roster itself changes far less often than any single
    minion's grains do.
    """
    salt = _import_salt()
    now = time.time()
    if _key_list_cache["key_ids"] and (now - _key_list_cache["fetched_at"]) < _KEY_LIST_REFRESH_SECS:
        return _key_list_cache["key_ids"]

    key_ids = []
    try:
        key_ids = salt.key.Key(opts).list_keys().get("minions", [])
    except Exception:
        pass

    if not key_ids:
        import json as json_mod
        output = _run_salt_cmd(["salt-key", "-L", "--out=json"], timeout=15)
        if output:
            try:
                key_ids = json_mod.loads(output).get("minions", [])
            except Exception:
                key_ids = []

    if key_ids:
        _key_list_cache["key_ids"] = key_ids
        _key_list_cache["fetched_at"] = now
        return key_ids

    # Both paths came up empty this round -- keep serving the last known
    # good list (if any) rather than treating a transient hiccup in both
    # paths at once as "the fleet is now empty".
    return _key_list_cache["key_ids"]


def cmd_minion_cache_list(opts, req):
    """Fetch minion cache data — salt.key for names, then Salt cache for grains.

    `cache.fetch()` unpickles a file per minion — real work. On a fleet
    with a large number of permanently-dead minions (created for testing
    and never cleaned up by autoclean; some masters carry thousands of
    these) doing that unconditionally on every poll (every `dashboard_secs`,
    2s by default) means thousands of unpickles every couple of seconds,
    forever, for minions whose grains can never change again. `cache.updated()`
    is a cheap stat-only call, so grains are only re-fetched when a minion's
    cache mtime has actually moved since the last poll — dead minions get
    stat'd, not unpickled, on every subsequent tick.
    """
    minions = []
    key_ids = _resolve_key_ids(opts)

    if not key_ids:
        # Never resolved a single accepted key, ever -- report failure
        # rather than a "successful" empty roster, so the Rust side (which
        # treats a successful minion_cache_list as authoritative and
        # replaces its whole roster with it) keeps showing the last
        # known-good roster instead of wiping every minion from view.
        result(req["id"], "minion_cache_list", ok=False, data={"error": "no minion keys found via salt.key.Key or salt-key -L"})
        return

    try:
        cache = _import_salt().cache.factory(opts)
    except Exception:
        cache = None

    for mid in key_ids:
        entry = {"id": mid}
        if cache is not None:
            try:
                # Not a "last seen": Salt rewrites the entry only when it
                # compiles the minion's pillar, so an idle, healthy minion
                # keeps an old mtime.
                mtime = cache.updated(_minion_data_bank(mid), "data") or 0

                cached = _minion_grains_cache.get(mid)
                if cached is not None and cached["mtime"] == mtime:
                    entry.update(cached["grains"])
                else:
                    grains = (cache.fetch(_minion_data_bank(mid), "data") or {}).get("grains")
                    grain_fields = {}
                    if grains:
                        grain_fields = {
                            "os": grains.get("os", ""),
                            "osrelease": grains.get("osrelease", ""),
                            "osfinger": grains.get("osfinger", ""),
                            "saltversion": grains.get("saltversion", ""),
                            "cpuarch": grains.get("cpuarch", ""),
                        }
                    entry.update(grain_fields)
                    _minion_grains_cache[mid] = {"mtime": mtime, "grains": grain_fields}
            except Exception:
                pass
        minions.append(entry)

    # Drop entries for minions no longer in salt-key (deleted/rejected)
    # so this doesn't grow without bound over a long-running agent.
    if key_ids:
        key_id_set = set(key_ids)
        for stale in [m for m in _minion_grains_cache if m not in key_id_set]:
            del _minion_grains_cache[stale]

    result(req["id"], "minion_cache_list", data={"minions": minions})


def cmd_zmq_ping(opts, req):
    """Respond immediately. Rust side measures full round-trip from send to
    response receipt, so no Salt runner call is needed here. This is a local
    agent-liveness check, not a measurement of Salt's own ZMQ transport."""
    result(req["id"], "zmq_ping", data={})


def cmd_job_ping(opts, req):
    """Fire an async test.ping so the Rust side can time the real dispatch ->
    first return round-trip through Salt's actual publish/return transport.
    Uses LocalClient.cmd_async() directly (same as fleet_scan) rather than
    shelling out to the salt CLI, so a fresh Python-interpreter/CLI startup
    isn't added to every sample on top of the real transport latency.
    Returns the JID immediately; completion is observed by the caller via
    the normal event stream, not by waiting on this command."""
    jid = None
    try:
        salt = _import_salt()
        local = salt.client.LocalClient(opts["conf_file"])
        jid = local.cmd_async("*", "test.ping", [], tgt_type="glob")
        if isinstance(jid, bytes):
            jid = jid.decode()
    except Exception:
        jid = None
    result(req["id"], "job_ping", data={"jid": jid})


def cmd_alive_check(opts, req):
    """Check which minions are connected — uses salt-key + cache presence (passive, no pings)."""
    up = []
    down = []
    try:
        import subprocess as sp, json as json_mod, os
        # Get all minion IDs
        output = _run_salt_cmd(["salt-key", "-L", "--out=json"], timeout=15)
        if not output:
            result(req["id"], "alive_check", data={"up": [], "down": []})
            return
        key_data = json_mod.loads(output)
        all_minions = key_data.get("minions", [])
        # A minion is "up" if it has a cache entry (means it has checked in).
        # Layout is a directory per minion holding data.p, not a flat
        # "<mid>.p" file — e.g. /var/cache/salt/master/minions/<mid>/data.p.
        cachedir = opts.get("cachedir", "/var/cache/salt/master")
        minion_cache_dir = os.path.join(cachedir, "minions")
        for mid in all_minions:
            cache_file = os.path.join(minion_cache_dir, mid, "data.p")
            if os.path.exists(cache_file):
                up.append(mid)
            else:
                down.append(mid)
    except Exception:
        pass
    result(req["id"], "alive_check", data={"up": up, "down": down})


def cmd_minion_ping(opts, req):
    """Ping a single minion via test.ping — used by admin mode detail popup."""
    mid = req.get("minion_id", "")
    if not mid:
        result(req["id"], "minion_ping", data={"minion_id": mid, "connected": False})
        return
    try:
        import subprocess as sp, json as json_mod
        output = _run_salt_cmd(["salt", mid, "test.ping", "--out=json", "--timeout=10"], timeout=15)
        connected = False
        if output:
            data = json_mod.loads(output)
            connected = data.get(mid) is True or data.get("return", {}).get(mid) is True
    except Exception:
        connected = False
    result(req["id"], "minion_ping", data={"minion_id": mid, "connected": connected})


def cmd_minion_grains(opts, req):
    """Fetch all grains from a single minion via salt {mid} grains.items --out=json."""
    mid = req.get("minion_id", "")
    if not mid:
        result(req["id"], "minion_grains", data={"minion_id": mid, "grains": []})
        return
    try:
        import subprocess as sp, json as json_mod
        output = _run_salt_cmd(["salt", mid, "grains.items", "--out=json", "--timeout=15"], timeout=20)
        pairs = []
        if output:
            data = json_mod.loads(output)
            grains_data = data.get(mid, {})
            if not isinstance(grains_data, dict):
                grains_data = data.get("return", grains_data)
            if isinstance(grains_data, dict):
                for key in sorted(grains_data.keys()):
                    val = grains_data[key]
                    if isinstance(val, str):
                        val_str = val
                    elif isinstance(val, (list, dict)):
                        # Pretty-print nested structures (interfaces, disks,
                        # pythonpath, etc.) instead of a single-line repr —
                        # the TUI renders each line indented under the key.
                        val_str = json_mod.dumps(val, indent=2, sort_keys=True)
                    else:
                        val_str = str(val)
                    if len(val_str) > 2000:
                        val_str = val_str[:2000] + "\n..."
                    pairs.append({"key": key, "value": val_str})
    except Exception:
        pairs = []
    result(req["id"], "minion_grains", data={"minion_id": mid, "grains": pairs})


def _cached_job_targets(opts, jid):
    """The minions a job was published to, from the local_cache job load
    (empty for other job-cache backends or when it can't be read)."""
    returner = opts.get("ext_job_cache") or opts.get("master_job_cache") or "local_cache"
    if returner != "local_cache":
        return []
    try:
        import salt.returners.local_cache as local_cache_mod

        local_cache_mod.__opts__ = opts
        return [str(m) for m in (local_cache_mod.get_load(jid) or {}).get("Minions", [])]
    except Exception:
        traceback.print_exc(file=sys.stderr)
        return []


def _cached_job_outcomes(opts, jid):
    """Each minion's stored `(retcode, success)` for a job, read from the
    local_cache job directory. `jobs.lookup_jid` returns only the return
    data, so a command that printed its error and exited non-zero (e.g.
    `/bin/sh: uptime: not found`, rc 127) would otherwise read as a success.
    Empty for other job-cache backends or when the files can't be read."""
    returner = opts.get("ext_job_cache") or opts.get("master_job_cache") or "local_cache"
    if returner != "local_cache":
        return {}
    try:
        import salt.payload
        import salt.returners.local_cache as local_cache_mod
        import salt.utils.jid as jid_mod

        local_cache_mod.__opts__ = opts
        jid_dir = jid_mod.jid_dir(jid, local_cache_mod._job_dir(), opts["hash_type"])
        outcomes = {}
        for mid in os.listdir(jid_dir):
            path = os.path.join(jid_dir, mid, local_cache_mod.RETURN_P)
            if not os.path.isfile(path):
                continue
            with open(path, "rb") as fh:
                stored = salt.payload.load(fh)
            if isinstance(stored, dict):
                outcomes[mid] = (stored.get("retcode"), stored.get("success"))
        return outcomes
    except Exception:
        traceback.print_exc(file=sys.stderr)
        return {}


def cmd_jid_lookup(opts, req):
    """Look up a specific job's per-minion returns.

    Runs `salt-run jobs.lookup_jid JID` twice:

    - `--force-color` (no `--out` override): captures the *exact* text
      `salt-run jobs.lookup_jid JID` would print in a real terminal,
      including which outputter Salt auto-selects. This matters because
      forcing `--out=json` (as this used to do exclusively) makes Salt
      skip that auto-selection entirely — for a state.apply/highstate job,
      `jobs.lookup_jid` actually returns `{"outputter": "highstate", "data":
      {...}}`, and only the *unforced* CLI path unwraps that and renders it
      through the highstate outputter (Name/Result/Comment/Duration per
      state ID). `--out=json` just serializes that wrapper dict as-is,
      which read like raw, unformatted state IDs (`file_|-...`, `__id__`,
      `__run_num__`, ...) instead of real `salt-run` output.
    - `--out=json`: structured data used for the left-panel host list,
      filtering, and the derived success/retcode per minion (still needed
      even though the JSON payload for a state job needs the same
      `{"outputter": ..., "data": ...}` unwrap as above before it can be
      iterated per-minion).

    Both run concurrently, so a lookup costs the slower of the two rather
    than their sum -- on a 4000-minion `grains.items` job each one alone can
    take 20s+. The colored run is best-effort: if it fails or misses the
    deadline, the popup falls back to rendering `full_return` itself.
    `subprocess_secs`/`postprocess_secs` are returned so the Rust side can
    show the breakdown in the popup header.
    """
    jid = req.get("jid", "")
    if not jid:
        result(req["id"], "jid_lookup", ok=False, data={"found": False, "error": "jid required"})
        return

    import json as json_mod
    import subprocess as sp

    timeout = req.get("timeout") or 300
    outputs = {}
    errors = {}

    def _lookup(key, args):
        try:
            cp = sp.run(args, capture_output=True, text=True, timeout=timeout)
            if cp.returncode == 0 and cp.stdout.strip():
                outputs[key] = cp.stdout
            else:
                lines = (cp.stderr or "").strip().splitlines()
                errors[key] = lines[-1] if lines else "salt-run exited with code {}".format(cp.returncode)
        except sp.TimeoutExpired:
            errors[key] = "salt-run jobs.lookup_jid timed out after {}s".format(timeout)
        except Exception as exc:
            errors[key] = str(exc)

    t0 = time.time()
    threads = [
        threading.Thread(target=_lookup, args=("raw", [
            "salt-run", "jobs.lookup_jid", jid, "--force-color",
            # Always show full state detail regardless of the master's
            # configured state_output/state_verbose -- a "terse"/"changes"
            # master default would otherwise hide unchanged/no-op states
            # from this popup even though the user explicitly opened it to
            # see everything about this one job.
            "--state-output=full", "--state-verbose=True",
        ]), daemon=True),
        threading.Thread(target=_lookup, args=("json", [
            "salt-run", "jobs.lookup_jid", jid, "--out=json",
        ]), daemon=True),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    t1 = time.time()
    raw_output = outputs.get("raw", "")
    json_output = outputs.get("json")
    if not json_output:
        result(req["id"], "jid_lookup", ok=False, data={
            "found": False,
            "error": errors.get("json", "salt-run returned no output"),
            "subprocess_secs": round(t1 - t0, 2),
        })
        return

    try:
        lookup_data = json_mod.loads(json_output)
    except Exception:
        result(req["id"], "jid_lookup", ok=False, data={"found": False, "error": "failed to parse salt-run output"})
        return

    # A state.apply/highstate/orchestrate job's lookup_jid return isn't
    # {minion: return} directly — it's wrapped so the CLI knows to render
    # it through the highstate outputter instead of the default nested one.
    # See salt.runner.Runner.run()'s own identical check.
    if isinstance(lookup_data, dict) and set(lookup_data.keys()) == {"outputter", "data"}:
        lookup_data = lookup_data["data"]

    def _derive_retcode_and_success(return_val):
        if isinstance(return_val, dict):
            result_val = return_val.get("result")
            if result_val is not None:
                if result_val is False:
                    return (1, False)
                if isinstance(result_val, bool):
                    return (0, True)
            elif return_val and all(
                isinstance(v, dict) and ("result" in v or "__run_num__" in v)
                for v in return_val.values()
            ):
                # state.apply-style return: a dict keyed by state ID, each
                # value its own per-state result dict -- there's no
                # top-level "result"/"retcode" to find above, so without
                # this the whole job silently defaulted to "success" (rc=0)
                # even when every state inside had result=False.
                any_failed = any(v.get("result") is False for v in return_val.values())
                return (1, False) if any_failed else (0, True)
            rc = return_val.get("retcode", 0)
            if not isinstance(rc, int):
                try:
                    rc = int(rc)
                except (ValueError, TypeError):
                    rc = 0
            return (rc, rc == 0)
        if return_val is None:
            return (1, False)
        if isinstance(return_val, bool):
            return (0, return_val)
        return (0, True)

    import re
    _ansi_re = re.compile("\x1b\\[[0-9;]*m")

    def _strip_ansi_with_map(text):
        """Strip ANSI escapes while tracking, for each character kept, its
        index in the original string — so a position found in the stripped
        text (used for reliably matching a bare "mid:" header even though
        `--force-color` puts escape codes right before it) can be mapped
        back to a slice boundary in the original (still-colored) text.

        Walks escape-code matches only (`finditer`), copying each run of
        plain text between them in one slice, rather than a Python-level
        loop doing a regex match at every single character position — for
        a lookup spanning thousands of minions (hundreds of thousands of
        lines) the per-character version added a real, visible delay on its
        own before the O(n^2) bug below was even reached."""
        out_parts = []
        pos_map = []
        last = 0
        for m in _ansi_re.finditer(text):
            if m.start() > last:
                out_parts.append(text[last:m.start()])
                pos_map.extend(range(last, m.start()))
            last = m.end()
        if last < len(text):
            out_parts.append(text[last:])
            pos_map.extend(range(last, len(text)))
        return "".join(out_parts), pos_map

    _stripped, _pos_map = _strip_ansi_with_map(raw_output)

    def _to_raw_pos(stripped_pos):
        if stripped_pos >= len(_pos_map):
            return len(raw_output)
        return _pos_map[stripped_pos]

    # Precompute every minion's [start, end) span in the stripped text in a
    # single regex pass over column-0 "mid:" header lines. A `.find()` per
    # minion is O(minions x text) -- for a 4000-minion `grains.items` job
    # (tens of MB of output) that alone could outlast the lookup timeout.
    # Each section's end is simply the next header's start.
    if _stripped.strip():
        # Each entry is (newline_pos, content_start, mid) -- content_start
        # (right after "mid:") is where THIS minion's own slice begins;
        # newline_pos (the position of the newline preceding "mid:") is what
        # closes the PREVIOUS minion's slice. Keeping these distinct matters:
        # the raw text between a newline and the next visible character can
        # contain that next minion's own leading ANSI codes (`--force-color`
        # puts them immediately before the name), and ending a slice at
        # "next minion's content_start" instead of "next minion's newline_pos"
        # would pull those codes into the wrong (preceding) minion's section.
        _mids = {str(k) for k in lookup_data}
        _mid_starts = []
        for m in re.finditer(r"^(\S[^\n]*?):[ \t]*$", _stripped, re.M):
            mid_str = m.group(1)
            if mid_str in _mids:
                _mids.discard(mid_str)
                start = m.start()
                _mid_starts.append((max(start - 1, 0), start, mid_str))
        _mid_spans = {}
        for idx, (_newline_pos, content_start, mid_str) in enumerate(_mid_starts):
            end = _mid_starts[idx + 1][0] if idx + 1 < len(_mid_starts) else len(_stripped)
            _mid_spans[mid_str] = (_to_raw_pos(content_start), _to_raw_pos(end))
    else:
        _mid_spans = {}

    def _extract_section(mid):
        """Pull this minion's own section out of the raw multi-minion CLI
        text — everything from its "mid:" header up to the next minion's
        header (or EOF) — preserving the original ANSI color codes."""
        bounds = _mid_spans.get(mid)
        if not bounds:
            return ""
        raw_start, raw_end = bounds
        return raw_output[raw_start:raw_end].strip()

    outcomes = _cached_job_outcomes(opts, jid)
    returns = []
    for mid, ret_val in lookup_data.items():
        mid = str(mid)
        retcode, success = _derive_retcode_and_success(ret_val)
        stored_rc, stored_success = outcomes.get(mid, (None, None))
        if isinstance(stored_rc, int) and stored_rc != 0:
            retcode, success = stored_rc, False
        elif stored_success is False:
            retcode, success = retcode or 1, False

        # Already plain JSON types -- it came straight from json.loads above.
        clean_return = ret_val

        summary = json_mod.dumps(clean_return, default=str)[:200]

        returns.append({
            "minion_id": mid,
            "success": success,
            "retcode": retcode,
            "summary": summary,
            "full_return": clean_return,
            # This minion's exact slice of the real `salt-run jobs.lookup_jid`
            # CLI output (correct outputter, real ANSI colors). Falls back to
            # the JSON-reconstructed nested view (see nested_return_lines in
            # src/ui/detail_popup.rs) only if the raw text couldn't be split.
            "formatted_text": _extract_section(mid),
        })

    returned = {r["minion_id"] for r in returns}
    data = {
        "found": True,
        "fun": "",
        "tgt": "",
        "start_time": "",
        "returns": returns,
        # Targeted minions with no return (yet): still running, down, or
        # never received the job.
        "missing": sorted(m for m in set(_cached_job_targets(opts, jid)) if m not in returned),
        "subprocess_secs": round(t1 - t0, 2),
        "postprocess_secs": round(time.time() - t1, 2),
    }
    result(req["id"], "jid_lookup", data=data)


# ── Event subscription ──────────────────────────────────────────────────────

_event_thread = None
_event_stop = threading.Event()


def _parse_stamp(stamp):
    """Convert Salt's _stamp (ISO-8601 string or datetime) to a Unix timestamp float.

    Salt's _stamp is always UTC but carries no tzinfo — Python's naive
    .timestamp() silently assumes the *local* timezone instead, which
    shifted every event's timestamp by the local UTC offset (matches
    _jid_to_epoch's own tzinfo=utc handling below).
    """
    if stamp is None:
        return time.time()
    try:
        from datetime import datetime, timezone
        if isinstance(stamp, (int, float)):
            return float(stamp)
        if isinstance(stamp, datetime):
            if stamp.tzinfo is None:
                stamp = stamp.replace(tzinfo=timezone.utc)
            return stamp.timestamp()
        if isinstance(stamp, str):
            s = stamp.strip()
            for fmt in ("%Y-%m-%dT%H:%M:%S.%f", "%Y-%m-%dT%H:%M:%S",
                        "%Y-%m-%d %H:%M:%S.%f", "%Y-%m-%d %H:%M:%S"):
                try:
                    return datetime.strptime(s[:26], fmt).replace(tzinfo=timezone.utc).timestamp()
                except ValueError:
                    continue
    except (ValueError, OSError):
        pass
    return time.time()


def _jid_to_epoch(jid):
    """Parse a Salt JID (YYYYMMDDHHMMSSuuuuuu) into a Unix timestamp float.

    Salt encodes the master wall clock time as the first 14 digits of the JID.
    JIDs are always UTC.
    """
    try:
        s = str(jid).strip()[:14]
        if len(s) == 14 and s.isdigit():
            from datetime import datetime, timezone
            dt = datetime.strptime(s, "%Y%m%d%H%M%S").replace(tzinfo=timezone.utc)
            return dt.timestamp()
    except (ValueError, OSError):
        pass
    return 0.0


def _event_loop(opts):
    """Run in a background thread: listen on event bus, forward events to stdout."""
    import salt.utils.event
    try:
        # opts comes from salt.config.master_config(), which always
        # resolves sock_dir from Salt's own defaults (typically
        # /var/run/salt/master, not under cachedir) — no need to guess it.
        sock_dir = opts["sock_dir"]
        me = salt.utils.event.MasterEvent(sock_dir, opts, listen=True)
        for evt in me.iter_events(tag="", full=True):
            if _event_stop.is_set():
                break
            if evt is None:
                continue
            tag = evt.get("tag", "")
            evt_data = evt.get("data", {})
            ts = _parse_stamp(evt_data.get("_stamp"))
            event(tag, evt_data, ts)
    except Exception as exc:
        error(None, "event_bus: {}".format(str(exc)))


def cmd_subscribe_events(opts, req):
    global _event_thread, _event_stop
    if _event_thread and _event_thread.is_alive():
        result(req["id"], "subscribe_events", data={"status": "already subscribed"})
        return
    _event_stop.clear()
    _event_thread = threading.Thread(target=_event_loop, args=(opts,), daemon=True)
    _event_thread.start()
    result(req["id"], "subscribe_events", data={"status": "subscribed"})


def cmd_unsubscribe_events(opts, req):
    global _event_stop, _event_thread
    _event_stop.set()
    _event_thread = None
    result(req["id"], "unsubscribe_events", data={"status": "unsubscribed"})


def cmd_fleet_scan(opts, req):
    """Run grains.items on all minions via LocalClient and return fleet data."""
    salt = _import_salt()
    local = salt.client.LocalClient(opts["conf_file"])

    # Get all accepted minion keys so we know which minions to expect
    expected_minions = []
    try:
        import json as json_mod
        output = _run_salt_cmd(["salt-key", "-L", "--out=json"], timeout=15)
        if output:
            keys = json_mod.loads(output)
            expected_minions = keys.get("minions", [])
    except Exception:
        pass

    # Fire async command — returns JID immediately without blocking
    try:
        jid = local.cmd_async("*", "grains.items", [],
                              tgt_type="glob")
    except Exception as exc:
        result(req["id"], "fleet_scan", ok=False, data={"error": str(exc)})
        return

    if isinstance(jid, bytes):
        jid = jid.decode()

    result(req["id"], "fleet_scan", data={
        "jid": str(jid),
        "expected_minions": expected_minions,
    })


_STATE_APPLY_FUNS = ("state.apply", "state.sls", "state.highstate", "state.top")


def _format_job_args(fun, args):
    """Mirror of `format_job_args` in src/types.rs: renders a job's raw Salt
    `arg` list for the Args column -- for state runs the requested SLS list
    (or "highstate" when none was named), with `test=True` returned as a
    separate flag. Done here for the Job Cache so the server-side search and
    sort see the same text the TUI shows. Keys are walked sorted to match
    serde_json's (BTreeMap) object order on the Rust side.
    """
    def text(v):
        if isinstance(v, str):
            return v
        if isinstance(v, (list, tuple)):
            return ",".join(text(i) for i in v)
        return json.dumps(v, separators=(",", ":"), sort_keys=True, default=str)

    positional, kwargs, test_run = [], [], False
    for v in args if isinstance(args, list) else []:
        if isinstance(v, dict) and v.get("__kwarg__") is True:
            for k in sorted(v):
                val = v[k]
                if k == "test":
                    test_run = val is True or (isinstance(val, str) and val.lower() == "true")
                elif not k.startswith("__"):
                    kwargs.append((k, val))
        else:
            positional.append(text(v))
    if fun in _STATE_APPLY_FUNS:
        for i, (k, val) in enumerate(kwargs):
            if k == "mods":
                positional.insert(0, text(val))
                del kwargs[i]
                break
        if not positional and fun != "state.sls":
            positional.append("highstate")
    parts = positional + ["{}={}".format(k, text(val)) for k, val in kwargs]
    return " ".join(parts), test_run


# str() since py3 can't order a None (e.g. a cache entry missing its user)
# against a str.
_JOB_SORT_KEYS = {
    "jid": lambda j: j["jid"],
    "user": lambda j: str(j["user"]),
    "started": lambda j: (j["start_epoch"], j["jid"]),
    "target": lambda j: str(j["tgt"]),
    "fun": lambda j: str(j["fun"]),
    "args": lambda j: (j["args"], j["test_run"]),
}


# Job-cache summaries already read from disk, keyed by the job's
# (top, final) directory pair under `<cachedir>/jobs/`. A busy master's cache
# holds tens of thousands of jobs, and unpickling every `.load.p` again for
# each search/page/refresh is what made the Job Cache panel re-load from
# scratch every time -- only directories not seen before are read now.
_JOB_INDEX = {}
_JOB_INDEX_LOCK = threading.Lock()


def _fast_local_job_walk(opts, on_progress):
    """Read the local_cache job directory ourselves via Salt's own (private)
    local_cache layout -- rather than the atomic `salt-run jobs.list_jobs`
    call, which only returns once it has read the entire cache with no
    visibility into how far along it is. `on_progress(count)` is invoked
    periodically as jobs are read, so a slow first read (tens of thousands
    of jobs) shows live progress instead of an opaque freeze.

    Incremental across calls: directory listings are cheap, so every call
    re-lists the cache, reads `.load.p` only for job directories missing
    from `_JOB_INDEX`, and forgets directories Salt has since expired
    (`keep_jobs_seconds`). Jobs whose `.load.p` isn't written yet or can't
    be read are left out of the index so the next call retries them.

    Only correct for the `local_cache` job-cache backend -- callers must
    check `master_job_cache`/`ext_job_cache` first and fall back to the
    slower salt-run-based path for any other backend (redis, mysql, etcd).
    Reaches into `local_cache`'s private `_job_dir`/`LOAD_P`, so any
    failure (e.g. a future Salt version renaming them) raises and the
    caller falls back to the safe path rather than surfacing a broken cache
    view.
    """
    import salt.payload
    import salt.returners.local_cache as local_cache_mod
    import salt.utils.jid as jid_mod

    # These modules normally get `__opts__` injected by Salt's loader when
    # loaded as a returner plugin; called directly like this, it has to be
    # set by hand before `_job_dir()` (which reads it) works.
    local_cache_mod.__opts__ = opts
    job_dir = local_cache_mod._job_dir()

    # Serialized so a search typed while the first read is still running
    # waits for it and then reuses it, instead of starting a second full read.
    with _JOB_INDEX_LOCK:
        seen = set()
        new = 0
        for top in os.listdir(job_dir):
            t_path = os.path.join(job_dir, top)
            try:
                finals = os.listdir(t_path)
            except OSError:
                continue
            for final in finals:
                key = (top, final)
                seen.add(key)
                if key in _JOB_INDEX:
                    continue
                try:
                    with open(os.path.join(t_path, final, local_cache_mod.LOAD_P), "rb") as rfh:
                        job = salt.payload.load(rfh)
                except Exception:
                    continue
                if not job or "jid" not in job:
                    continue
                jid = job["jid"]
                info = jid_mod.format_jid_instance(jid, job)
                info.pop("Metadata", None)
                _JOB_INDEX[key] = (jid, info)
                new += 1
                if new % 250 == 0:
                    on_progress(len(_JOB_INDEX))
        for key in [k for k in _JOB_INDEX if k not in seen]:
            del _JOB_INDEX[key]
        on_progress(len(_JOB_INDEX))
        return dict(_JOB_INDEX.values())


def cmd_jobs_list(opts, req):
    """Fetch recent jobs from the master job cache.

    Uses a direct, incremental read of the job cache directory (reporting
    progress as it goes — see `_fast_local_job_walk`) when the configured
    job-cache backend is Salt's default `local_cache`; falls back to the
    same atomic `salt-run jobs.list_jobs` subprocess call (as cmd_jid_lookup
    uses, to avoid RunnerClient threading issues inside the agent) for any
    other backend, or if the fast path fails for any reason.

    Optional request fields:
      - filter_text (str): search the full cache by comma-separated terms
        (see `_filter_terms` / `_term_matcher`), any of which may match
      - max_results (int): max items to return (default 500)
      - offset (int): skip N entries in sort order (for pagination)
      - sort_key (str): jid/user/started/target/fun/args -- sort the full
        cache by this column before paging (default: newest first)
      - sort_desc (bool): reverse sort_key's order
    """
    import json as json_mod

    def on_progress(count):
        ndjson({"type": "progress", "id": req["id"], "cmd": "jobs_list", "data": {"loaded": count}})

    result_data = None
    returner = opts.get("ext_job_cache") or opts.get("master_job_cache") or "local_cache"
    if returner == "local_cache":
        try:
            result_data = _fast_local_job_walk(opts, on_progress)
        except Exception:
            traceback.print_exc(file=sys.stderr)
            result_data = None

    if result_data is None:
        output = _run_salt_cmd(["salt-run", "jobs.list_jobs", "--out=json"], timeout=90)
        if not output:
            result(req["id"], "jobs_list", ok=False, data={"error": "salt-run jobs.list_jobs failed"})
            return

        try:
            result_data = json_mod.loads(output)
        except Exception as exc:
            result(req["id"], "jobs_list", ok=False, data={"error": "failed to parse output: {}".format(str(exc))})
            return

    matchers = [_term_matcher(t) for t in _filter_terms(req.get("filter_text", ""))]
    max_results = req.get("max_results", 500)
    offset = req.get("offset", 0)

    jobs = []
    for jid, info in result_data.items():
        if not isinstance(info, dict):
            continue
        fun = info.get("Function", info.get("fun", ""))
        tgt = info.get("Target", info.get("tgt", ""))
        if isinstance(tgt, (list, tuple)):
            tgt = ",".join(str(t) for t in tgt)
        tgt_type = info.get("Target-type", info.get("tgt_type", "glob"))
        user = info.get("User", info.get("user", ""))
        # Use JID-derived epoch as primary — the JID always encodes the master
        # wall-clock time (YYYYMMDDHHMMSSuuuuuu) and is reliable even when the
        # cache lacks timestamp fields (common for runner jobs).
        start_epoch = _jid_to_epoch(jid)
        stamp = ""
        if start_epoch <= 0.0:
            stamp = info.get("StartTime", info.get("start_time", info.get("_stamp", "")))
            start_epoch = _parse_stamp(stamp)

        args, test_run = _format_job_args(fun, info.get("Arguments", info.get("arg", [])))

        # Apply filter across full cache (server-side search)
        if matchers:
            fields = [str(jid), str(fun), str(tgt), str(user), args]
            if not any(m(fields, str(tgt)) for m in matchers):
                continue
        jobs.append({
            "jid": str(jid),
            "fun": fun,
            "tgt": tgt,
            "tgt_type": tgt_type,
            "user": user,
            "start_time": str(stamp),
            "start_epoch": start_epoch,
            "args": args,
            "test_run": test_run,
        })

    total_matching = len(jobs)
    # start_epoch only has whole-second precision; the JID breaks ties so
    # jobs started within the same second still come out newest first.
    jobs.sort(key=lambda j: (j["start_epoch"], j["jid"]), reverse=True)
    sort_key = _JOB_SORT_KEYS.get(req.get("sort_key", ""))
    if sort_key:
        jobs.sort(key=sort_key, reverse=bool(req.get("sort_desc")))
    jobs = jobs[offset:]
    jobs = jobs[:max_results]

    result(req["id"], "jobs_list", data={
        "jobs": jobs,
        "total_count": total_matching,
        "returned_count": len(jobs),
    })


def _filter_terms(text):
    """Split a filter into its comma-separated terms the way salt-radar's own
    filters do: a term wrapped in slashes is a regex (a comma inside it
    belongs to it), and a `/` that never closes is plain text."""
    terms, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if c == "," or c.isspace():
            i += 1
            continue
        start = i
        if c == "/":
            j, end = i + 1, None
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == "/" and _ends_regex(text[j + 1:]):
                    end = j + 1
                    break
                j += 1
            if end is not None:
                if end < n and text[end] == "i":
                    end += 1
                terms.append(text[start:end])
                i = end
                continue
        end = text.find(",", start)
        end = n if end < 0 else end
        term = text[start:end].strip()
        if term:
            terms.append(term)
        i = end
    return terms


def _ends_regex(rest):
    """Whether what follows a `/` closes a regex: an optional `i` flag, then
    only whitespace up to a comma or the end."""
    if rest.startswith("i"):
        rest = rest[1:]
    rest = rest.lstrip()
    return rest == "" or rest.startswith(",")


def _term_matcher(term):
    """A predicate `(fields, tgt) -> bool` for one filter term: a `/regex/`
    (`/.../i` ignores case) searched in every field, a glob (`*`, `?`,
    `[...]`) matched against the whole target like Salt's targeting, or a
    case-insensitive substring of any field. A regex that doesn't compile is
    plain text."""
    if len(term) >= 3 and term.startswith("/"):
        body, flags = None, 0
        if term.endswith("/i") and len(term) > 3:
            body, flags = term[1:-2], re.IGNORECASE
        elif term.endswith("/"):
            body = term[1:-1]
        if body:
            try:
                rx = re.compile(body.replace("\\/", "/"), flags)
                return lambda fields, tgt: any(rx.search(f) for f in fields)
            except re.error:
                pass
    low = term.lower()
    glob = any(ch in term for ch in "*?[")

    def match(fields, tgt):
        if glob and fnmatch.fnmatchcase(tgt.lower(), low):
            return True
        return any(low in f.lower() for f in fields)

    return match


# ── Command dispatcher ──────────────────────────────────────────────────────

HANDLERS = {
    "version": cmd_version,
    "config_snapshot": cmd_config_snapshot,
    "master_config": cmd_master_config,
    "minion_cache_list": cmd_minion_cache_list,
    "zmq_ping": cmd_zmq_ping,
    "job_ping": cmd_job_ping,
    "jid_lookup": cmd_jid_lookup,
    "subscribe_events": cmd_subscribe_events,
    "unsubscribe_events": cmd_unsubscribe_events,
    "fleet_scan": cmd_fleet_scan,
    "jobs_list": cmd_jobs_list,
    "list_keys": cmd_list_keys,
    "list_processes": cmd_list_processes,
    "list_services": cmd_list_services,
    "get_system_info": cmd_get_system_info,
    "get_zmq_connections": cmd_get_zmq_connections,
    "get_api_stats": cmd_get_api_stats,
    "alive_check": cmd_alive_check,
    "minion_ping": cmd_minion_ping,
    "minion_grains": cmd_minion_grains,
}

def dispatch(opts, req):
    """Hand a request off to its handler on its own thread.

    Every command runs threaded now, not just the ones known to be slow
    (`jobs_list`, `jid_lookup`) — `main()`'s `for line in sys.stdin` loop
    previously ran every *other* handler inline, so on a busy master a
    slow synchronous command (e.g. `list_processes`/`get_zmq_connections`
    taking a while under load) blocked the loop from even reading the
    next stdin line, let alone dispatching it — a `jid_lookup` queued
    behind one could sit unread for tens of seconds despite finishing in
    a few once it actually started. See `ndjson`'s lock for the write
    side of the same change.
    """
    cmd = req.get("cmd", "")
    handler = HANDLERS.get(cmd)
    if handler is None:
        result(req.get("id", 0), cmd, ok=False, data={"error": "unknown cmd: {}".format(cmd)})
        return

    t = threading.Thread(target=_run_handler, args=(handler, opts, req), daemon=True)
    t.start()


def _run_handler(handler, opts, req):
    cmd = req.get("cmd", "")
    try:
        handler(opts, req)
    except Exception as exc:
        result(
            req.get("id", 0), cmd, ok=False,
            data={"error": "{}: {}".format(cmd, str(exc))},
        )
        traceback.print_exc(file=sys.stderr)


# ── Main loop ───────────────────────────────────────────────────────────────

def main():
    master_config = "/etc/salt/master"
    args = list(sys.argv[1:])
    while args:
        if args[0] == "--master-config" and len(args) >= 2:
            master_config = args[1]
            args = args[2:]
        else:
            args = args[1:]

    # Save a reference to the real stdout fd so ndjson() / event thread
    # are immune to fd-1 redirects (used by fleet scan).
    _init_output()

    # Load opts once at startup
    salt = _import_salt()
    opts = get_opts(master_config)

    # Subscribe to events automatically on startup
    try:
        cmd_subscribe_events(opts, {"id": 0, "cmd": "subscribe_events"})
    except Exception:
        pass

    # Read commands from stdin
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as exc:
            error(None, "parse error: {}".format(str(exc)))
            continue
        try:
            dispatch(opts, req)
        except Exception as exc:
            # dispatch() itself only starts a thread -- failures here are
            # things like the OS refusing a new thread (e.g. a ulimit hit
            # after a long, busy session), not handler bugs (those are
            # already caught inside _run_handler). Letting this propagate
            # would kill the whole agent process and leave the Rust side
            # in a permanently dead connection until the app is restarted.
            error(req.get("id", 0), "dispatch failed: {}".format(str(exc)))
            traceback.print_exc(file=sys.stderr)


if __name__ == "__main__":
    main()
