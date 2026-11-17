"""Job Detail cuts the colored `salt-run jobs.lookup_jid` output into one
section per minion. The sections must be where they always were (the old cut
went through a copy without color codes and a map of every character back),
without its memory: a lookup on thousands of minions prints tens of MB.

Run from the repository root: python3 -m unittest discover -s agent/tests
"""
import os
import re
import sys
import tracemalloc
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import salt_radar_agent as agent  # noqa: E402

GREEN, CYAN, RED, RESET = "\x1b[0;32m", "\x1b[0;36m", "\x1b[0;31m", "\x1b[0;0m"


def colored_lookup(minions, lines=8):
    """Output shaped like `salt-run jobs.lookup_jid --force-color`."""
    out = []
    for n in range(minions):
        out.append("{}web-{:05d}:{}".format(GREEN, n, RESET))
        out.append("{}    ----------{}".format(CYAN, RESET))
        for i in range(lines):
            out.append("{}    grain_{}:{}\n        {}value-é-{}{}".format(CYAN, i, RESET, GREEN, i, RESET))
    return "\n".join(out) + "\n"


def old_sections(raw_output, mids):
    """The cut as it was: headers found in a stripped copy, mapped back
    through the colored-text index of every kept character."""
    ansi = re.compile("\x1b\\[[0-9;]*m")
    parts, pos_map, last = [], [], 0
    for m in ansi.finditer(raw_output):
        parts.append(raw_output[last:m.start()])
        pos_map.extend(range(last, m.start()))
        last = m.end()
    parts.append(raw_output[last:])
    pos_map.extend(range(last, len(raw_output)))
    stripped = "".join(parts)

    def raw_pos(p):
        return len(raw_output) if p >= len(pos_map) else pos_map[p]

    mids, starts = set(mids), []
    for m in re.finditer(r"^(\S[^\n]*?):[ \t]*$", stripped, re.M):
        if m.group(1) in mids:
            mids.discard(m.group(1))
            starts.append((max(m.start() - 1, 0), m.start(), m.group(1)))
    return {
        mid: (raw_pos(start), raw_pos(starts[i + 1][0] if i + 1 < len(starts) else len(stripped)))
        for i, (_, start, mid) in enumerate(starts)
    }


class MinionSectionsTest(unittest.TestCase):
    def test_sections_are_where_the_old_cut_put_them(self):
        cases = {
            "plain grains": (colored_lookup(5, lines=2), ["web-00000", "web-00002", "web-00004"]),
            "no color": ("db1:\n    True\ndb2:\n    False\n", ["db1", "db2"]),
            "codes after the colon": (GREEN + "db1" + RESET + ":" + RESET + "  \n  x\n" + RED + "db2:" + RESET + "\n  y\n", ["db1", "db2"]),
            "nested key named like a minion": ("db1:\n    db2:\n        z\ndb2:\n    y\n", ["db1", "db2"]),
            "a minion that didn't return": ("db1:\n    x\n", ["db1", "db9"]),
            "empty": ("", ["db1"]),
            "only codes": (GREEN + RESET + "\n", ["db1"]),
            "id with a colon": ("[fe80::1]:\n    x\nfe80::2:\n    y\n", ["[fe80::1]", "fe80::2"]),
        }
        for name, (text, mids) in cases.items():
            with self.subTest(name):
                self.assertEqual(agent._minion_sections(text, mids), old_sections(text, mids))

    def test_a_section_keeps_its_colors_and_not_the_next_headers(self):
        text = colored_lookup(3, lines=1)
        start, end = agent._minion_sections(text, ["web-00001", "web-00002"])["web-00001"]
        section = text[start:end]
        self.assertTrue(section.startswith("web-00001:" + RESET), section)
        self.assertTrue(section.endswith(RESET), "ends before the next header's color code")
        self.assertNotIn("web-00002", section)

    def test_memory_does_not_grow_with_the_output(self):
        text = colored_lookup(4000)
        mids = ["web-{:05d}".format(n) for n in range(4000)]

        def peak(f):
            tracemalloc.start()
            f(text, mids)
            top = tracemalloc.get_traced_memory()[1]
            tracemalloc.stop()
            return top

        new, old = peak(agent._minion_sections), peak(old_sections)
        report = "{:.1f} MB of output: {:.2f} MB, the old cut {:.1f} MB".format(len(text) / 1e6, new / 1e6, old / 1e6)
        # What's left is a few hundred bytes per minion, not per character.
        self.assertLess(new, 400 * len(mids), report)
        self.assertLess(new * 20, old, report)


if __name__ == "__main__":
    unittest.main()
