# Security Policy

Kubiru stores credentials that can read and change Kubernetes clusters, so we
take security reports seriously and appreciate responsible disclosure.

## Supported versions

Security fixes are released for the latest minor version.

| Version | Supported |
| ------- | --------- |
| 0.1.x   | ✅        |
| < 0.1   | ❌ (legacy web version, no longer maintained) |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report privately through GitHub's
[private vulnerability reporting](https://github.com/phoscoder/kubiru/security/advisories/new)
(**Security → Report a vulnerability** on the repository).

Please include:

- a description of the issue and its impact;
- the steps or a proof of concept needed to reproduce it;
- the Kubiru version or commit, and your operating system;
- any suggested fix or mitigation.

Do not include real cluster tokens, kubeconfig files or other live secrets in
your report.

### What to expect

| Step | Target time |
| --- | --- |
| Acknowledgement of your report | 3 working days |
| Initial assessment and severity rating | 7 working days |
| Fix or mitigation for high and critical issues | 30 days |

We will keep you informed of progress, agree a disclosure date with you, and
credit you in the advisory and changelog unless you would rather stay
anonymous.

## Scope

In scope:

- leaking cluster tokens (to disk, logs, other hosts, the clipboard without user action, crash output);
- TLS verification bypasses that the user did not explicitly choose;
- requests sent to a different host or path than intended (for example through redirects or crafted names);
- edits or deletions applied to an object other than the one the user chose;
- weaknesses in how local data is stored or protected.

Out of scope:

- what a cluster permits a valid token to do (that is governed by Kubernetes RBAC);
- issues that need an attacker who already controls the user's account or machine;
- clusters configured with **Skip TLS certificate verification**, for attacks that this setting makes possible by design;
- vulnerabilities in third-party dependencies with no demonstrated impact on Kubiru (please report those upstream; we track advisories through RustSec).

## Security design

For how Kubiru protects credentials and requests, see the
[Security section of the README](README.md#security) and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
