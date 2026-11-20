"""Reading what tells a starving master from a busy one out of /proc and
/sys: CPU steal and iowait, the run queue, pressure stall, swapping, dropped
connections, open files and cgroup CPU throttling.

Run from the repository root: python3 -m unittest discover -s agent/tests
"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import salt_radar_agent as agent  # noqa: E402


class ProcStatTest(unittest.TestCase):
    def test_iowait_steal_and_run_queue(self):
        text = (
            "cpu  4705 150 1120 16250 520 0 30 77 0 0\n"
            "cpu0 2000 70 500 8000 260 0 15 40 0 0\n"
            "intr 1234 0 0\n"
            "procs_running 3\n"
            "procs_blocked 1\n"
        )
        self.assertEqual(agent._parse_proc_stat(text), {
            "cpu_iowait_jiffies": 520, "cpu_steal_jiffies": 77, "procs_running": 3, "procs_blocked": 1,
        })

    def test_old_kernels_without_steal(self):
        self.assertEqual(agent._parse_proc_stat("cpu  1 2 3 4\n")["cpu_steal_jiffies"], 0)


class PressureTest(unittest.TestCase):
    def test_some_total(self):
        text = (
            "some avg10=1.53 avg60=0.87 avg300=0.40 total=123456789\n"
            "full avg10=0.20 avg60=0.10 avg300=0.05 total=2345678\n"
        )
        self.assertEqual(agent._parse_psi(text), 123456789)

    def test_cpu_pressure_on_older_kernels_has_no_full_line(self):
        self.assertEqual(agent._parse_psi("some avg10=0.00 avg60=0.00 avg300=0.00 total=42\n"), 42)

    def test_nothing_to_read(self):
        self.assertIsNone(agent._parse_psi(""))


class VmstatNetstatTest(unittest.TestCase):
    def test_swap_faults_and_oom_kills(self):
        text = "nr_free_pages 1000\npswpin 12\npswpout 34\npgmajfault 56\noom_kill 1\n"
        self.assertEqual(agent._parse_vmstat(text), {
            "swap_in_pages": 12, "swap_out_pages": 34, "major_faults": 56, "oom_kills": 1,
        })

    def test_listen_queue_drops(self):
        text = (
            "TcpExt: SyncookiesSent SyncookiesRecv ListenOverflows ListenDrops TCPLostRetransmit\n"
            "TcpExt: 0 0 7 9 3\n"
            "IpExt: InNoRoutes InTruncatedPkts\n"
            "IpExt: 0 0\n"
        )
        self.assertEqual(agent._parse_netstat(text), {"listen_overflows": 7, "listen_drops": 9})
        self.assertEqual(agent._parse_netstat("IpExt: InNoRoutes\nIpExt: 0\n"), {})


class OpenFilesTest(unittest.TestCase):
    LIMITS = (
        "Limit                     Soft Limit           Hard Limit           Units     \n"
        "Max cpu time              unlimited            unlimited            seconds   \n"
        "Max open files            100000               100000               files     \n"
    )

    def test_soft_limit(self):
        self.assertEqual(agent._parse_fd_limit(self.LIMITS), 100000)

    def test_unlimited_or_missing(self):
        self.assertIsNone(agent._parse_fd_limit(self.LIMITS.replace("100000               100000", "unlimited            unlimited")))
        self.assertIsNone(agent._parse_fd_limit(""))

    def test_own_process(self):
        fds, limit = agent._master_files(os.getpid())
        self.assertGreater(fds, 0)
        self.assertTrue(limit is None or limit >= fds)


class CgroupThrottlingTest(unittest.TestCase):
    def test_v1_cpu_controller(self):
        cgroup = "12:memory:/system.slice/salt-master.service\n4:cpu,cpuacct:/system.slice/salt-master.service\n"
        self.assertEqual(agent._cpu_stat_path(cgroup), ("/sys/fs/cgroup/cpu,cpuacct/system.slice/salt-master.service/cpu.stat", True))

    def test_v2_unified_hierarchy_when_present(self):
        self.assertIsNone(agent._cpu_stat_path("0::/no/such/salt-radar-test.service\n"))

    def test_quota_counts(self):
        v2 = "usage_usec 9000000\nuser_usec 6000000\nsystem_usec 3000000\nnr_periods 1200\nnr_throttled 300\nthrottled_usec 4500000\n"
        self.assertEqual(agent._parse_cpu_stat(v2, False), {"periods": 1200, "throttled": 300, "throttled_usec": 4500000})
        v1 = "nr_periods 10\nnr_throttled 2\nthrottled_time 5000000\n"
        self.assertEqual(agent._parse_cpu_stat(v1, True), {"periods": 10, "throttled": 2, "throttled_usec": 5000})

    def test_no_quota_no_periods(self):
        self.assertIsNone(agent._parse_cpu_stat("usage_usec 9000000\nuser_usec 6000000\n", False))
        self.assertIsNone(agent._parse_cpu_stat("usage_usec 9000000\nnr_periods 0\nnr_throttled 0\nthrottled_usec 0\n", False))


class SaturationTest(unittest.TestCase):
    def test_reads_what_this_host_has(self):
        out = agent._saturation()
        if os.path.exists("/proc/stat"):
            self.assertIn("procs_running", out)
            self.assertIn("cpu_steal_jiffies", out)
        for value in out.get("psi_us", {}).values():
            self.assertGreaterEqual(value, 0)


if __name__ == "__main__":
    unittest.main()
