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
    salt = _import_salt()
    cache = salt.cache.factory(opts)
    minion_ids = cache.list("minions")
    if not minion_ids:
        result(req["id"], "minion_cache_list", data={"minions": []})
        return

    minions = []
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
        minions.append(entry)

    result(req["id"], "minion_cache_list", data={"minions": minions})


def cmd_zmq_ping(opts, req):
    """Respond immediately. Rust side measures full round-trip from send to
    response receipt, so no Salt runner call is needed here."""
    result(req["id"], "zmq_ping", data={"latency_ms": 0.0})


def cmd_jid_lookup(opts, req):
    jid = req.get("jid", "")
    if not jid:
        result(req["id"], "jid_lookup", ok=False, data={"found": False, "error": "jid required"})
        return

    import subprocess as sp
    import json as json_mod

    # Get structured JSON output from lookup_jid
    cp = sp.run(
        ["salt-run", "jobs.lookup_jid", jid, "--out=json"],
        capture_output=True, text=True, timeout=60,
    )
    if cp.returncode != 0 or not cp.stdout.strip():
        result(req["id"], "jid_lookup", ok=False, data={"found": False})
        return

    try:
        lookup_data = json_mod.loads(cp.stdout)
    except Exception as exc:
        result(req["id"], "jid_lookup", ok=False,
               data={"found": False, "error": "failed to parse lookup_jid output: {}".format(str(exc))})
        return

    if not isinstance(lookup_data, dict):
        result(req["id"], "jid_lookup", ok=False, data={"found": False})
        return

    def _derive_retcode_and_success(return_val):
        """Derive (retcode, success) from a minion's return value.

        Handles common Salt return formats:
          - state dicts: read the 'result' key
          - primitive bool: True=OK, False=FAIL
          - None: FAIL
          - anything truthy: OK
        """
        if isinstance(return_val, dict):
            # Check for state result key
            result_val = return_val.get("result")
            if result_val is not None:
                if result_val is False:
                    return (1, False)
                # result_val is True or truthy
                if isinstance(result_val, bool):
                    return (0, True)
            # Check for retcode in return
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

    returns = []
    for mid, ret_val in lookup_data.items():
        mid = str(mid)

        retcode, success = _derive_retcode_and_success(ret_val)

        # Sanitise return value to JSON-encodable types
        try:
            clean_return = json_mod.loads(json_mod.dumps(ret_val, default=str))
        except Exception:
            clean_return = str(ret_val)

        # Pretty-print for display
        try:
            formatted = json_mod.dumps(ret_val, indent=2, default=str)
        except Exception:
            formatted = str(ret_val)

        summary = (formatted or "")[:200]

        returns.append({
            "minion_id": mid,
            "success": success,
            "retcode": retcode,
            "summary": summary,
            "full_return": clean_return,
            "formatted_text": formatted,
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
        sk = salt.key.Key(opts)
        keys = sk.list_keys()
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
    import subprocess as sp
    import json as json_mod

    cp = sp.run(
        ["salt-run", "jobs.list_jobs", "--out=json"],
        capture_output=True, text=True, timeout=90,
    )
    if cp.returncode != 0 or not cp.stdout.strip():
        result(req["id"], "jobs_list", ok=False, data={"error": "salt-run jobs.list_jobs failed"})
        return

    try:
        result_data = json_mod.loads(cp.stdout)
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
            fields_to_search = [str(jid).lower(), fun.lower(), tgt.lower(), user.lower()]
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
    cmd_subscribe_events(opts, {"id": 0, "cmd": "subscribe_events"})

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
