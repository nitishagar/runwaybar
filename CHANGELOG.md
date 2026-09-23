# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/).

## [Unreleased]

### Added
- Provider core: Claude Code, Codex, z.ai/ZCode, OpenCode — read-only credential
  discovery, tolerant quota parsing, typed error taxonomy, per-provider isolation.
- `runwaybar status` CLI (json / text / waybar formats), cache-first one-shot polling.
- `runwaybar serve` daemon: SNI tray icon with per-provider menu and tooltips,
  same-user Unix-socket IPC, cooldown-honouring scheduler, transition notifications.
- `runwaybar refresh`, `runwaybar install --user` (XDG autostart), `runwaybar --version`.
- Packaging: .deb, glibc + musl-static tarballs, install.sh, CI with provenance
  attestations and a read-only-credentials strace guard.

[Unreleased]: https://github.com/nitishagar/runwaybar/compare/main...HEAD
