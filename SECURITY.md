# Security Policy

## Supported versions

Security fixes are provided for the latest stable 2.x release. The 1.x line is
supported only for critical migration issues until 2026-11-14.

## Reporting a vulnerability

Please do not open a public issue for an unpatched vulnerability. Use GitHub's
private vulnerability reporting feature for this repository. Include affected
version, environment, reproduction steps, impact, and any proposed mitigation.

You should receive an acknowledgement within seven days. A coordinated fix and
disclosure timeline will be agreed after triage.

## Local command model

TopMonitoring can execute commands configured by the local user. Imported
configurations never trust or enable executable modules automatically. Review
every command before enabling its **Trusted** switch.
