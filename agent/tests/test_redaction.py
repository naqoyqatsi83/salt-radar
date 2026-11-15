"""What the agent blanks before a master config file reaches the TUI.

People who may run salt-radar through sudo, and nothing else, see the Master
Config tab: no credential may get past the redaction, lines must keep their
numbers (settings point at lines of their file), and what isn't a credential
must stay readable.

Run from the repository root: python3 -m unittest discover -s agent/tests
Needs PyYAML, not Salt.
"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import salt_radar_agent as agent  # noqa: E402

SECRETS = ["S3CR3T1", "S3CR3T2", "S3CR3T3", "S3CR3T4"]


class RedactionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.pki = os.path.join(cls.tmp.name, "pki")
        os.makedirs(cls.pki)
        open(os.path.join(cls.pki, "key.pem"), "w").close()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def check(self, text, keeps=(), secrets=SECRETS):
        """`text` redacted: none of `secrets` left, as many lines as before,
        every string of `keeps` still there."""
        out = agent._redact_text(text)
        for secret in secrets:
            self.assertNotIn(secret, out, "leaked in:\n{}".format(out))
        self.assertEqual(len(out.splitlines()), len(text.splitlines()), "line numbers moved:\n{}".format(out))
        for keep in keeps:
            self.assertIn(keep, out)
        return out

    def test_plain_and_quoted_values(self):
        self.check("password: S3CR3T1\nport: 8000\n", ["port: 8000"])
        self.check("mysql.pass: 'S3CR3T1'\ntoken: \"S3CR3T2\"\n")
        self.check("pin_pass: 123456\n", secrets=["123456"])

    def test_inline_mappings(self):
        self.check("rest_cherrypy: {port: 8000, password: S3CR3T1, host: 0.0.0.0}\n", ["port: 8000", "host: 0.0.0.0"])
        self.check("vault: {url: 'https://v:8200', token: 'S3CR3T1'}\n", ["https://v:8200"])
        self.check("auth: {pass: S3CR3T1, token: S3CR3T2, user: bob}\n", ["user: bob"])

    def test_multi_line_values(self):
        self.check("secret: S3CR3T1\n  S3CR3T2\n  S3CR3T3\nworker_threads: 5\n", ["worker_threads: 5"])
        self.check("passphrase: \"S3CR3T1\n  S3CR3T2\"\nkeep: 1\n", ["keep: 1"])

    def test_block_scalars(self):
        self.check("gpg_passphrase: |\n  S3CR3T1\n  S3CR3T2\ntimeout: 7\n", ["timeout: 7"])
        self.check("signing_key: |2\n    S3CR3T1\n    S3CR3T2\nnext: x\n", ["next: x"])
        self.check("api_token: >+\n  S3CR3T1\n\nnext: x\n", ["next: x"])

    def test_lists_and_mappings_under_a_credential_key(self):
        self.check("secret: [S3CR3T1, S3CR3T2]\ntokens:\n  - S3CR3T3\n  - S3CR3T4\n")
        self.check("credentials:\n  user: S3CR3T1\n  pass: S3CR3T2\n", ["user:", "pass:"])

    def test_nested_settings(self):
        self.check(
            "ext_pillar:\n  - vault:\n      conf: x\n      token: S3CR3T1\n"
            "  - git: [main https://u:S3CR3T2@git.example/repo]\n",
            ["conf: x", "git.example/repo"],
        )

    def test_a_secret_starting_with_a_slash_is_not_a_path(self):
        self.check("token: /S3CR3T1+aBcD==\n")
        self.check("secret: /S3CR3T1/aBcD==\n")

    def test_paths_and_references_stay_readable(self):
        self.check(
            "ssl_key: {0}/key.pem\nssl_crt: {0}/missing.crt\n".format(self.pki),
            ["key.pem", "missing.crt"],
        )
        self.check(
            "key_pass: sdb://masterkeyring/key_pass\n# signing_key_pass: sdb://kr/sp\n",
            ["sdb://masterkeyring/key_pass", "sdb://kr/sp"],
        )

    def test_flags_nulls_and_empty_values_stay(self):
        self.check(
            "key_pass: null\nrotate_aes_key: True\npassword: ''\n",
            ["key_pass: null", "rotate_aes_key: True", "password: ''"],
        )

    def test_comments(self):
        self.check(
            "# password: S3CR3T1\nport: 1 # old_token: S3CR3T2\n#  mysql.pass: \"S3CR3T3\"\n",
            ["port: 1"],
        )
        self.check("# see https://user:S3CR3T1@host/x\n", ["https://user:", "@host/x"])

    def test_urls(self):
        self.check("gitfs_remotes:\n  - https://deploy:S3CR3T1@git.example/repo.git\n", ["https://deploy:", "git.example"])

    def test_other_credential_names(self):
        self.check(
            "nacl.config:\n  sk: S3CR3T1\n  box_type: sealedbox\nmattermost.hook: S3CR3T2\n"
            "slack_webhook.webhook: T0/B0/S3CR3T3\ns3.keyid: S3CR3T4\n",
            ["box_type: sealedbox"],
        )
        self.check("headers:\n  Authorization: Bearer S3CR3T1\nwifi_psk: S3CR3T2\n", ["headers:"])
        self.check("external_auth:\n  pam:\n    bob:\n      - .*\n", ["bob:", "- .*"])

    def test_anchors_and_tags(self):
        self.check("vault: &v {token: S3CR3T1}\nvault2: *v\n")
        self.check("shared: &pw S3CR3T1\nmysql.pass: *pw\nport: 1\n", ["mysql.pass: *pw", "port: 1"])
        self.check("mysql.pass: &pw S3CR3T1\nother: *pw\n", ["other: *pw"])
        self.check("password: !!binary U0VDUkVU\nnote: x\n", ["note: x"], secrets=["U0VDUkVU"])

    def test_a_file_that_is_not_yaml(self):
        self.check("password: S3CR3T1\n  bad: [\n token: S3CR3T2\n")
        self.check("rest_cherrypy: {password: S3CR3T1\n: :\n")
        # Whatever stops it being YAML, a credential's value on the lines
        # after its key is still blanked.
        broken = "a: [\n"
        self.check(broken + "password: \"abc\n  S3CR3T1\"\nport: 1\n", ["port: 1"])
        self.check(broken + "tokens:\n  - S3CR3T1\n  - S3CR3T2\nport: 1\n", ["port: 1"])
        self.check(broken + "tokens:\n- S3CR3T1\n- S3CR3T2\nport: 1\n", ["port: 1"])
        self.check(broken + "vault:\n  token:\n    S3CR3T1\n  url: x\n", ["url: x"])
        self.check(broken + "key: |\n  S3CR3T1\n\n  S3CR3T2\nport: 1\n", ["port: 1"])
        self.check(broken + "- password: S3CR3T1\n  user: bob\n", ["user: bob"])
        self.check("a: \x01\npassword:\n  S3CR3T1\n")

    def test_windows_line_endings(self):
        self.check("password: S3CR3T1\r\nport: 8000\r\n", ["port: 8000"])

    def test_parsed_values(self):
        """`_redact` works on the loaded settings, shown in the list and popup."""
        self.assertEqual(
            agent._redact({"rest_cherrypy": {"port": 8000, "password": "S3CR3T1"}}),
            {"rest_cherrypy": {"port": 8000, "password": agent._REDACTED}},
        )
        self.assertEqual(agent._redact({"secret": ["S3CR3T1", "S3CR3T2"]}), {"secret": [agent._REDACTED] * 2})
        self.assertEqual(agent._redact({"token": "/S3CR3T1+aBcD=="}), {"token": agent._REDACTED})
        path = self.pki + "/key.pem"
        self.assertEqual(agent._redact({"ssl_key": path}), {"ssl_key": path})
        self.assertEqual(
            agent._redact({"gitfs_remotes": ["https://u:S3CR3T1@git.example/r"]}),
            {"gitfs_remotes": ["https://u:{}@git.example/r".format(agent._REDACTED)]},
        )
        flags = {"key_pass": None, "rotate_aes_key": True}
        self.assertEqual(agent._redact(flags), flags)

    def test_a_credential_is_blanked_under_any_name_once_known(self):
        """An anchor's value loads under both keys; a password may be repeated."""
        config = {"shared": "S3CR3T1-long", "mysql.pass": "S3CR3T1-long", "note": "uses S3CR3T1-long", "pin_pass": "1234", "port": "1234"}
        self.addCleanup(agent._known_credentials.clear)
        agent._note_credentials(config)
        self.assertEqual(agent._redact(config), {
            "shared": agent._REDACTED, "mysql.pass": agent._REDACTED, "note": "uses " + agent._REDACTED,
            "pin_pass": agent._REDACTED, "port": "1234",
        })
        self.check("# was S3CR3T1-long\nnote: S3CR3T1-long\nport: 1234\n", ["port: 1234"])

    def test_the_options_runner_return_is_redacted_in_its_event(self):
        self.addCleanup(agent._known_credentials.clear)
        """`test.get_opts` returns the master's whole configuration."""
        data = {"fun": "runner.test.get_opts", "jid": "20261115100000000000",
                "return": {"worker_threads": 5, "rest_cherrypy": {"port": 8000, "password": "S3CR3T1"}}}
        shown = agent._shown_event_data("salt/run/20261115100000000000/ret", data)
        self.assertEqual(shown["return"], {"worker_threads": 5, "rest_cherrypy": {"port": 8000, "password": agent._REDACTED}})
        self.assertEqual(shown["fun"], data["fun"])
        self.assertEqual(data["return"]["rest_cherrypy"]["password"], "S3CR3T1", "the event itself isn't changed")
        other = {"fun": "runner.jobs.list_jobs", "return": {"password": "x"}}
        self.assertIs(agent._shown_event_data("salt/run/1/ret", other), other)
        self.assertIs(agent._shown_event_data("salt/job/1/ret/m", data), data)


if __name__ == "__main__":
    unittest.main()
