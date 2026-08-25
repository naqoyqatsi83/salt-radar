#!/usr/bin/env python3
"""
salt-radar-agent — Python helper for salt-radar TUI.

Communicates with the Rust process via NDJSON over stdin/stdout.
Reads JSON-command lines from stdin, writes NDJSON responses to stdout.
"""

from __future__ import print_function

import json
import os
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

def ndjson(obj):
    """Write a JSON line to the saved output fd and flush immediately."""
    global _OUTPUT_FD
    fd = _OUTPUT_FD if _OUTPUT_FD is not None else 1
    data = json.dumps(obj, default=str) + "\n"
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

                    # Determine role
                    role = "Unknown"
                    if "MWorker-" in cmdline: role = "Worker"
                    elif "EventPublisher" in cmdline: role = "EventPublisher"
                    elif "ReqServer" in cmdline: role = "ReqServer"
                    elif "salt-master" in cmdline: role = "Master"
                    elif "salt-api" in cmdline: role = "API"
                    elif "salt-syndic" in cmdline: role = "Syndic"
                    elif "salt-minion" in cmdline: role = "Minion"

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
    }
    try:
        # Load
        with open("/proc/loadavg", "r") as f:
            load = f.read().split()
            info["load_1"] = float(load[0])
            info["load_5"] = float(load[1])
            info["load_15"] = float(load[2])

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


def cmd_minion_cache_list(opts, req):
    """Fetch minion cache data — salt-key for names, then Salt cache for grains."""
    minions = []
    seen = set()

    # Step 1: get the minion list from salt-key (reliable, always works)
    try:
        import subprocess as sp, json as json_mod
        output = _run_salt_cmd(["salt-key", "-L", "--out=json"], timeout=15)
        if output:
            key_data = json_mod.loads(output)
            for mid in key_data.get("minions", []):
                minions.append({"id": mid, "last_seen": 0, "cache_updated": 0})
                seen.add(mid)
    except Exception:
        pass

    # Step 2: enrich with grains data from the cache (if available)
    try:
        salt = _import_salt()
        cache = salt.cache.factory(opts)
        minion_ids = cache.list("minions")
        if minion_ids:
            for mid in minion_ids:
                entry = {"id": mid}
                grains = cache.fetch("minions", mid)
                if grains:
                    entry["os"] = grains.get("os", "")
                    entry["osrelease"] = grains.get("osrelease", "")
                    entry["saltversion"] = grains.get("saltversion", "")
                    entry["cpuarch"] = grains.get("cpuarch", "")
                entry["last_seen"] = cache.updated("minions", mid) or 0
                entry["cache_updated"] = cache.updated("minions", mid) or 0
                if mid in seen:
                    # Update existing entry with cache data
                    for e in minions:
                        if e["id"] == mid:
                            e.update(entry)
                            break
    except Exception:
        pass

    result(req["id"], "minion_cache_list", data={"minions": minions})


def cmd_zmq_ping(opts, req):
    """Respond immediately. Rust side measures full round-trip from send to
    response receipt, so no Salt runner call is needed here."""
    result(req["id"], "zmq_ping", data={"latency_ms": 0.0})


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
        # A minion is "up" if it has a cache file (means it has checked in)
        cachedir = opts.get("cachedir", "/var/cache/salt/master")
        minion_cache_dir = os.path.join(cachedir, "minions")
        for mid in all_minions:
            cache_file = os.path.join(minion_cache_dir, "{}.p".format(mid))
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
                    val_str = str(val) if not isinstance(val, str) else val
                    if len(val_str) > 500:
                        val_str = val_str[:500] + "..."
                    pairs.append({"key": key, "value": val_str})
    except Exception:
        pairs = []
    result(req["id"], "minion_grains", data={"minion_id": mid, "grains": pairs})


def cmd_jid_lookup(opts, req):
    jid = req.get("jid", "")
    if not jid:
        result(req["id"], "jid_lookup", ok=False, data={"found": False, "error": "jid required"})
        return

    import json as json_mod

    # Run raw CLI output for console format (y key / formatted_text)
    _color_env = dict(os.environ, CLICOLOR_FORCE="1", FORCE_COLOR="1", TERM="xterm-256color")
    cli_output = _run_salt_cmd(
        ["salt-run", "--force-color", "jobs.lookup_jid", jid],
        timeout=60, env=_color_env,
    )
    if not cli_output:
        result(req["id"], "jid_lookup", ok=False, data={"found": False})
        return

    # Run JSON output for structured data (Y key / full_return)
    json_output = _run_salt_cmd(
        ["salt-run", "jobs.lookup_jid", jid, "--out=json"],
        timeout=60,
    )
    lookup_data = {}
    if json_output:
        try:
            lookup_data = json_mod.loads(json_output)
        except Exception:
            lookup_data = {}
        lookup_data = {}

    def _derive_retcode_and_success(return_val):
        if isinstance(return_val, dict):
            result_val = return_val.get("result")
            if result_val is not None:
                if result_val is False:
                    return (1, False)
                if isinstance(result_val, bool):
                    return (0, True)
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

    # Parse CLI output into per-minion chunks (lines may contain ANSI codes).
    # Minion header = stripped line starting at col 0 ending with ':' (e.g. "ol9:")
    import re
    def _strip_ansi(s):
        return re.sub(r'\x1b\[[0-9;]*m', '', s)
    per_minion = {}
    lines = cli_output.splitlines()
    current_minion = None
    current_lines = []
    for line in lines:
        plain = _strip_ansi(line)
        if plain and not plain[0].isspace() and plain.endswith(":"):
            if current_minion is not None:
                per_minion[current_minion] = "\n".join(current_lines).rstrip("\n")
            current_minion = plain[:-1].strip()
            current_lines = [line]
        elif current_minion is not None:
            current_lines.append(line)
    if current_minion is not None:
        per_minion[current_minion] = "\n".join(current_lines).rstrip("\n")

    returns = []
    for mid, formatted_text in per_minion.items():
        ret_val = lookup_data.get(mid)
        retcode, success = _derive_retcode_and_success(ret_val) if ret_val is not None else (0, True)

        # Sanitise JSON return for full_return
        if ret_val is not None:
            try:
                clean_return = json_mod.loads(json_mod.dumps(ret_val, default=str))
            except Exception:
                clean_return = str(ret_val)
        else:
            clean_return = formatted_text

        summary = (formatted_text or "")[:200]

        returns.append({
            "minion_id": str(mid),
            "success": success,
            "retcode": retcode,
            "summary": summary,
            "full_return": clean_return,
            "formatted_text": formatted_text,
        })

    data = {
        "found": True,
        "fun": "",
        "tgt": "",
        "start_time": "",
        "returns": returns,
    }
    result(req["id"], "jid_lookup", data=data)


# ── Event subscription ──────────────────────────────────────────────────────

_event_thread = None
_event_stop = threading.Event()


def _parse_stamp(stamp):
    """Convert Salt's _stamp (ISO-8601 string or datetime) to a Unix timestamp float."""
    if stamp is None:
        return time.time()
    try:
        from datetime import datetime
        if isinstance(stamp, (int, float)):
            return float(stamp)
        if isinstance(stamp, datetime):
            return stamp.timestamp()
        if isinstance(stamp, str):
            s = stamp.strip()
            for fmt in ("%Y-%m-%dT%H:%M:%S.%f", "%Y-%m-%dT%H:%M:%S",
                        "%Y-%m-%d %H:%M:%S.%f", "%Y-%m-%d %H:%M:%S"):
                try:
                    return datetime.strptime(s[:26], fmt).timestamp()
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
        sock_dir = opts.get("sock_dir", "")
        if not sock_dir:
            sock_dir = os.path.join(opts.get("cachedir", "/var/cache/salt/master"), "master")
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


def cmd_jobs_list(opts, req):
    """Fetch recent jobs from the master job cache via salt-run subprocess.

    Uses subprocess (same as cmd_jid_lookup) to avoid RunnerClient threading
    issues inside the agent.

    Optional request fields:
      - filter_text (str): search across jid/fun/tgt/user on the full cache
      - max_results (int): max items to return (default 500)
      - offset (int): skip N newest entries (for pagination)
    """
    import json as json_mod

    output = _run_salt_cmd(["salt-run", "jobs.list_jobs", "--out=json"], timeout=90)
    if not output:
        result(req["id"], "jobs_list", ok=False, data={"error": "salt-run jobs.list_jobs failed"})
        return

    try:
        result_data = json_mod.loads(output)
    except Exception as exc:
        result(req["id"], "jobs_list", ok=False, data={"error": "failed to parse output: {}".format(str(exc))})
        return

    filter_text = req.get("filter_text", "")
    max_results = req.get("max_results", 500)
    offset = req.get("offset", 0)

    jobs = []
    for jid, info in result_data.items():
        if not isinstance(info, dict):
            continue
        fun = info.get("Function", info.get("fun", ""))
        tgt = info.get("Target", info.get("tgt", ""))
        tgt_type = info.get("tgt_type", info.get("tgt_type", "glob"))
        user = info.get("User", info.get("user", ""))
        # Use JID-derived epoch as primary — the JID always encodes the master
        # wall-clock time (YYYYMMDDHHMMSSuuuuuu) and is reliable even when the
        # cache lacks timestamp fields (common for runner jobs).
        start_epoch = _jid_to_epoch(jid)
        stamp = ""
        if start_epoch <= 0.0:
            stamp = info.get("StartTime", info.get("start_time", info.get("_stamp", "")))
            start_epoch = _parse_stamp(stamp)

        args_raw = info.get("Arguments", info.get("arg", []))
        args_list = args_raw if isinstance(args_raw, list) else []

        # Apply filter across full cache (server-side search)
        if filter_text:
            ft = filter_text.lower()
            fields_to_search = [str(jid).lower(), str(fun).lower(), str(tgt).lower(), str(user).lower()]
            if not any(ft in f for f in fields_to_search):
                continue
        jobs.append({
            "jid": str(jid),
            "fun": fun,
            "tgt": tgt,
            "tgt_type": tgt_type,
            "user": user,
            "start_time": str(stamp),
            "start_epoch": start_epoch,
            "arguments": args_list,
        })

    total_matching = len(jobs)
    jobs.sort(key=lambda j: j["start_epoch"], reverse=True)
    jobs = jobs[offset:]
    jobs = jobs[:max_results]

    result(req["id"], "jobs_list", data={
        "jobs": jobs,
        "total_count": total_matching,
        "returned_count": len(jobs),
    })


# ── Command dispatcher ──────────────────────────────────────────────────────

HANDLERS = {
    "version": cmd_version,
    "config_snapshot": cmd_config_snapshot,
    "minion_cache_list": cmd_minion_cache_list,
    "zmq_ping": cmd_zmq_ping,
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
    "alive_check": cmd_alive_check,
    "minion_ping": cmd_minion_ping,
    "minion_grains": cmd_minion_grains,
}

# Commands that may block for a long time (Salt runner calls, etc.).
# These run in a background thread so the agent stays responsive.
_LONG_RUNNING_CMDS = frozenset(["jid_lookup", "jobs_list"])


def dispatch(opts, req):
    cmd = req.get("cmd", "")
    handler = HANDLERS.get(cmd)
    if handler is None:
        result(req.get("id", 0), cmd, ok=False, data={"error": "unknown cmd: {}".format(cmd)})
        return

    if cmd in _LONG_RUNNING_CMDS:
        # Run in a background thread so the main loop stays responsive
        t = threading.Thread(target=_run_handler, args=(handler, opts, req), daemon=True)
        t.start()
    else:
        _run_handler(handler, opts, req)


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
        dispatch(opts, req)


if __name__ == "__main__":
    main()
