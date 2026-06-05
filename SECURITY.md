# Security Policy

## Supported Versions

Security fixes primarily target:

- the `main` branch;
- the version currently deployed to the official service, when applicable.

Official production infrastructure and secret handling are maintained outside
this public repository.

Older branches, unmaintained forks, and derived deployments are not guaranteed.

## Reporting a Vulnerability

Do not create a public issue to report a security flaw.

Recommended channel once the repository is public:

- GitHub Private Vulnerability Reporting, once enabled.

Until this mechanism is available:

- report the vulnerability to the maintainer through an already established private channel;
- explicitly request a secure exchange channel if you need to transmit a secret, sensitive PoC, or logs containing private data;
- avoid any public disclosure before the fix is validated.

## What to Include in the Report

Please include, if possible:

- the affected component;
- the expected impact;
- exploitation prerequisites;
- reproduction steps;
- a minimal PoC if you have one;
- the affected versions or commits.

## Disclosure Expectations

Maintenance goals:

- acknowledge receipt quickly;
- confirm whether the issue is a vulnerability;
- prepare a fix or mitigation;
- coordinate disclosure once the risk is reduced.

Once GitHub Private Vulnerability Reporting is enabled, use it as the preferred
reporting channel.
