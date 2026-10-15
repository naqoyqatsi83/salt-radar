# Security policy

salt-radar runs as root on a Salt master and reads its keys, job cache and
configuration, so a flaw in it can matter. Please report one privately:
use **Report a vulnerability** on the repository's
[Security tab](https://github.com/naqoyqatsi83/salt-radar/security/advisories/new)
instead of opening a public issue.

Include the salt-radar version (`What's new` shows the running one), the Salt
version, and the steps or input that trigger it. Only the latest release is
fixed; there are no backports.

Vulnerabilities in Salt itself belong to the
[Salt project](https://github.com/saltstack/salt/security/policy).
