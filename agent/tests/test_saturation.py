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


class DiskAndTcpTest(unittest.TestCase):
    DISKSTATS = (
        "   8       0 sda 1200 30 96000 2400 5400 120 432000 27000 0 9100 29400 0 0 0 0 0 0\n"
        "   8       2 sda2 1100 30 88000 2200 5300 120 430000 26800 0 9000 29000 0 0 0 0 0 0\n"
        " 253       0 dm-0 1050 0 86000 2300 5600 0 431000 31000 2 9050 33300 0 0 0 0 0 0\n"
    )

    def test_counters_of_one_device(self):
        self.assertEqual(agent._parse_diskstats(self.DISKSTATS, (253, 0)), {
            "device": "dm-0", "ios": 1050 + 5600, "io_ms": 2300 + 31000, "busy_ms": 9050, "sectors": 86000 + 431000,
        })
        self.assertIsNone(agent._parse_diskstats(self.DISKSTATS, (8, 1)))

    def test_old_kernels_with_fewer_fields(self):
        self.assertEqual(agent._parse_diskstats("   8       0 sda 1 0 8 4 2 0 16 6 0 9 10\n", (8, 0))["busy_ms"], 9)

    def test_mount_source_of_the_longest_mount_point(self):
        mountinfo = (
            "22 1 0:21 / / rw,relatime shared:1 - btrfs /dev/sda2 rw,subvol=/@\n"
            "40 22 0:35 / /var rw,relatime shared:2 - btrfs /dev/sda3 rw,subvol=/@/var\n"
            "41 22 0:36 / /var/cache/salt\\040master rw shared:3 - xfs /dev/mapper/vg-salt rw\n"
            "50 22 0:40 / /run rw - tmpfs tmpfs rw\n"
        )
        self.assertEqual(agent._mount_source("/var/cache/salt/master", mountinfo), "/dev/sda3")
        self.assertEqual(agent._mount_source("/var/cache/salt master/jobs", mountinfo), "/dev/mapper/vg-salt")
        self.assertEqual(agent._mount_source("/etc/salt", mountinfo), "/dev/sda2")
        self.assertEqual(agent._mount_source("/run/salt", mountinfo), "tmpfs")

    def test_tcp_retransmits(self):
        text = (
            "Tcp: RtoAlgorithm RtoMin RtoMax MaxConn ActiveOpens PassiveOpens AttemptFails EstabResets CurrEstab InSegs OutSegs RetransSegs InErrs OutRsts\n"
            "Tcp: 1 200 120000 -1 7773 40053 1 1 96 758490 651376 290 0 4\n"
            "Udp: InDatagrams NoPorts\n"
            "Udp: 10 0\n"
        )
        self.assertEqual(agent._parse_snmp(text), {"tcp_out_segs": 651376, "tcp_retrans_segs": 290})
        self.assertEqual(agent._parse_snmp("Udp: InDatagrams\nUdp: 1\n"), {})


class SaturationTest(unittest.TestCase):
    def test_reads_what_this_host_has(self):
        out = agent._saturation("/")
        if os.path.exists("/proc/stat"):
            self.assertIn("procs_running", out)
            self.assertIn("cpu_steal_jiffies", out)
        for value in out.get("psi_us", {}).values():
            self.assertGreaterEqual(value, 0)


if __name__ == "__main__":
    unittest.main()
