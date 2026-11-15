"""Which processes count as Salt's: the daemons and their sub-processes,
not commands that merely name one.

Run from the repository root: python3 -m unittest discover -s agent/tests
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import salt_radar_agent as agent  # noqa: E402


class SaltProgramTest(unittest.TestCase):
    def test_daemons_and_their_sub_processes(self):
        for cmdline, program in [
            ("/usr/bin/salt-master", "salt-master"),
            ("/opt/saltstack/salt/bin/python3.10 /usr/bin/salt-master MainProcess", "salt-master"),
            ("/opt/saltstack/salt/bin/python3.10 /usr/bin/salt-master ReqServer MWorker-0", "salt-master"),
            ("/usr/bin/python3 /usr/bin/salt-api RunNetapi(salt.loaded.int.netapi.rest_cherrypy)", "salt-api"),
            ("salt-minion MultiMinionProcessManager MinionProcessManager", "salt-minion"),
            ("/usr/bin/salt-syndic -d", "salt-syndic"),
        ]:
            self.assertEqual(agent._salt_program(cmdline.split()), program, cmdline)

    def test_commands_that_only_name_a_daemon(self):
        for cmdline in [
            "systemctl status salt-master",
            "journalctl -u salt-master -f",
            "tail -f /var/log/salt/master",
            "vim /usr/bin/salt-master",
            "/usr/bin/python3 -c import time salt-master",
            "salt-run jobs.list_jobs",
            "salt-radar",
            "",
        ]:
            self.assertIsNone(agent._salt_program(cmdline.split()), cmdline)

    def test_salt_master_check_takes_raw_arguments(self):
        self.assertTrue(agent._is_salt_master([b"/opt/saltstack/salt/bin/python3.10", b"/usr/bin/salt-master", b"MainProcess"]))
        self.assertFalse(agent._is_salt_master([b"journalctl", b"-u", b"salt-master"]))


if __name__ == "__main__":
    unittest.main()
