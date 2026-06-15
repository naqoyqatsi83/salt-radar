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

# ── Salt imports (these must succeed — run with Salt's Python) ──────────────

def _import_salt():
    import salt.config
    import salt.version
    import salt.utils.event
    import salt.cache
    import salt.runner
    return salt


# ── Helpers ─────────────────────────────────────────────────────────────────

def ndjson(obj):
    """Write a JSON line to stdout and flush immediately."""
    sys.stdout.write(json.dumps(obj, default=str) + "\n")
    sys.stdout.flush()


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
    salt = _import_salt()
    jid = req.get("jid", "")
    if not jid:
        result(req["id"], "jid_lookup", ok=False, data={"found": False, "error": "jid required"})
        return
    runner = salt.runner.RunnerClient(opts)
    try:
        lookup = runner.cmd("jobs.lookup_jid", [jid])
    except Exception as exc:
        result(req["id"], "jid_lookup", ok=False, data={
            "found": False, "error": str(exc),
        })
        return

    if not lookup:
        result(req["id"], "jid_lookup", data={"found": False})
        return

    # Normalize: lookup returns {minion_id: result_data, ...}
    returns = []
    for mid, retdata in lookup.items():
        if isinstance(retdata, dict):
            retcode = retdata.get("retcode", 0)
            success = not retcode
            summary = str(retdata.get("return", retdata))[:200]
        else:
            retcode = 0
            success = True
            summary = str(retdata)[:200]
        full = retdata if isinstance(retdata, (dict, list)) else str(retdata)
        returns.append({
            "minion_id": str(mid),
            "success": success,
            "retcode": retcode,
            "summary": summary,
            "full_return": full,
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
    """Convert Salt's _stamp (ISO-8601 string) to a Unix timestamp float."""
    if stamp is None:
        return time.time()
    try:
        from datetime import datetime
        if isinstance(stamp, (int, float)):
            return float(stamp)
        if isinstance(stamp, str):
            dt = datetime.strptime(stamp[:26], "%Y-%m-%dT%H:%M:%S.%f")
            return dt.timestamp()
    except (ValueError, OSError):
        pass
    return time.time()


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


# ── Command dispatcher ──────────────────────────────────────────────────────

HANDLERS = {
    "version": cmd_version,
    "config_snapshot": cmd_config_snapshot,
    "minion_cache_list": cmd_minion_cache_list,
    "zmq_ping": cmd_zmq_ping,
    "jid_lookup": cmd_jid_lookup,
    "subscribe_events": cmd_subscribe_events,
    "unsubscribe_events": cmd_unsubscribe_events,
}


def dispatch(opts, req):
    cmd = req.get("cmd", "")
    handler = HANDLERS.get(cmd)
    if handler is None:
        result(req.get("id", 0), cmd, ok=False, data={"error": "unknown cmd: {}".format(cmd)})
        return
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
