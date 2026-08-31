# TopMonitoring Release Checklist

A package or GitHub release is allowed only after every applicable item is
complete. Record command output or attach logs to the release/PR.

## Documentation matrix

| Change | Required documentation |
|---|---|
| Any user-visible fix/feature | `CHANGELOG.md` |
| UI, controls, or behavior | `README.md`, `docs/TECHNICAL.md`, screenshots when material |
| Config fields or migration | `MIGRATION_V2.md`, `docs/TECHNICAL.md` |
| Dependencies or packaging | `README.md`, `Cargo.toml`, installer docs |
| Version release | `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, README package example, AppStream release |
| Security/trust behavior | `SECURITY.md`, `CHANGELOG.md`, technical docs |

## Automated no-Wayland gate

- [ ] Working tree is clean.
- [ ] `./scripts/test-no-wayland.sh check` passes.
- [ ] `./scripts/test-no-wayland.sh smoke` survives the configured timeout.
- [ ] CI no-default-features tests and build pass.
- [ ] Release documentation checker passes.

## Manual X11 gate

- [ ] Candidate was run from `target/release/topmonitoring`, not the installed binary.
- [ ] Test used isolated state first, then a copied migration profile.
- [ ] X11 dock struts reserve the selected edge.
- [ ] Settings transaction, modules, providers, history, and quick actions work.
- [ ] Logs show no crash loop, freeze, or recurring command failures.
- [ ] CPU/memory usage remains reasonable during idle and Settings interaction.
- [ ] Core/provider diagnostics remain fresh and redact host, interface, path, and metric values.
- [ ] Process, sensor, and hwmon-picker windows stay responsive and their workers stop after close.
- [ ] At 1366 px, priority tiers/More prevent overlap and keep Settings reachable.
- [ ] Every overflowed module appears once with a live value and restores cleanly.
- [ ] Global search, catalog filters, lazy editors, Ctrl+F/Esc, and narrow navigation pass.
- [ ] More opens an explanatory state when zero modules overflow.

## Debian package gate

- [ ] Manual testing is explicitly confirmed.
- [ ] `./scripts/package-no-wayland.sh --confirmed-manual` passes.
- [ ] `dpkg-deb --info` metadata is correct.
- [ ] Package contains binary, desktop entry, AppStream metadata, and icons.
- [ ] `Conflicts` and `Replaces` contain `topmonitoring-no-wayland`.
- [ ] No-Wayland `Depends` does not contain `libgtk4-layer-shell0`.
- [ ] Optional `lintian` findings are resolved or documented.
- [ ] Upgrade from the previous installed no-Wayland package succeeds.
- [ ] Fresh install, autostart, and uninstall/reinstall are tested.
- [ ] SHA-256 checksum is published with the `.deb`.

## GitHub publication gate

- [ ] Branch history is clean and release commit is tagged.
- [ ] GitHub CI is green for Wayland and no-Wayland matrices.
- [ ] Release notes match `CHANGELOG.md`.
- [ ] Previous known-good package is retained for rollback.
- [ ] Source archive and built package correspond to the same commit.
