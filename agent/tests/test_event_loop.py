"""The agent's event listener keeps going and says when events stop.

Salt's listener is replaced by a fake one that fails to start, breaks
mid-stream, then loses the event bus and finds it again.

Run from the repository root: python3 -m unittest discover -s agent/tests
Needs neither Salt nor a master.
"""
import os
import sys
import threading
import time
import types
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import salt_radar_agent as agent  # noqa: E402

attempts = []
destroyed = []


class FakeBus:
    def __init__(self, sock_dir, opts, listen=True):
        attempts.append(len(attempts) + 1)
        self.n = attempts[-1]
        self.calls = 0
        self.cpub = True
        if self.n == 1:
            raise OSError("publisher socket missing")

    def get_event(self, wait=5, tag="", full=False):
        self.calls += 1
        time.sleep(0.01)
        if self.n == 2:
            if self.calls == 1:
                return {"tag": "salt/test/2", "data": {"_stamp": "2026-11-15T08:00:00.000000"}}
            raise RuntimeError("stream closed")
        # Third listener: the bus goes away for a while, then comes back.
        self.cpub = not 3 <= self.calls <= 5
        if self.calls in (1, 7):
            return {"tag": "salt/test/3.{}".format(self.calls), "data": {}}
        return None

    def destroy(self):
        destroyed.append(self.n)


class EventLoopTest(unittest.TestCase):
    def setUp(self):
        # `_event_loop` imports salt.utils.event itself: give it the fake.
        self.modules = {name: sys.modules.get(name) for name in ("salt", "salt.utils", "salt.utils.event")}
        salt, utils, event = (types.ModuleType(name) for name in self.modules)
        event.MasterEvent = FakeBus
        utils.event = event
        salt.utils = utils
        sys.modules.update({"salt": salt, "salt.utils": utils, "salt.utils.event": event})
        self.sent = []
        self.saved = (agent.ndjson, agent._EVENT_RETRY_SECS)
        self.listening = agent._publisher_listening
        agent._publisher_listening = lambda opts: True
        agent.ndjson = self.sent.append
        agent._EVENT_RETRY_SECS = 0.05
        agent._event_stop.clear()

    def tearDown(self):
        agent._event_stop.set()
        agent.ndjson, agent._EVENT_RETRY_SECS = self.saved
        agent._publisher_listening = self.listening
        for name, module in self.modules.items():
            if module is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = module

    def test_listener_restarts_and_reports_the_bus(self):
        thread = threading.Thread(target=agent._event_loop, args=({"sock_dir": "/x"},), daemon=True)
        thread.start()
        deadline = time.time() + 5
        while time.time() < deadline and not any(m.get("tag") == "salt/test/3.7" for m in self.sent):
            time.sleep(0.02)
        agent._event_stop.set()
        thread.join(2)
        self.assertFalse(thread.is_alive(), "the listener stops when unsubscribed")

        got = [
            (m["type"], m["tag"]) if m["type"] == "event" else (m["type"], m["ok"], m.get("error"))
            for m in self.sent
        ]
        down = "no event bus in /x (is salt-master running?)"
        self.assertEqual(got, [
            ("event_bus", False, "publisher socket missing"),
            ("event_bus", True, None), ("event", "salt/test/2"), ("event_bus", False, "stream closed"),
            ("event_bus", True, None), ("event", "salt/test/3.1"), ("event_bus", False, down),
            ("event_bus", True, None), ("event", "salt/test/3.7"),
        ])
        self.assertEqual(destroyed, [2, 3], "every listener that was made is closed")

    @unittest.skipUnless(hasattr(__import__("socket"), "AF_UNIX"), "needs Unix sockets")
    def test_the_publisher_socket_is_asked_whether_it_listens(self):
        """Salt's listener keeps saying it's connected while the publisher is gone."""
        import socket
        import tempfile
        with tempfile.TemporaryDirectory() as sock_dir:
            opts = {"sock_dir": sock_dir}
            self.assertFalse(self.listening(opts), "no socket")
            server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            server.bind(os.path.join(sock_dir, "master_event_pub.ipc"))
            server.listen(1)
            self.assertTrue(self.listening(opts))
            server.close()
            self.assertFalse(self.listening(opts), "the socket file is left behind by a publisher that died")


if __name__ == "__main__":
    unittest.main()
