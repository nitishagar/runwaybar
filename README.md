# ▍RunwayBar

**Your AI runway, at a glance** — a featherweight Linux tray bar showing usage
limits for the AI coding tools you already have: **Claude Code**, **Codex**,
**z.ai / Zcode**, **OpenCode** and **Muse Code**.

One small Rust binary. No Electron, no Qt, no interpreter, no accounts, no
telemetry — Apache-2.0 licensed.

```
$ runwaybar status
Claude Code: ok
  session  62%, resets in 1h 59m
  weekly   31%
Codex: not installed
z.ai / ZCode: ok
  session  42%, resets in 12m
  weekly   10%
OpenCode: ok
  session  55%, resets in 59m
Muse Code: ok
  session  18%, resets in 3h 12m
  weekly   9%
```

## Highlights

- **Tiny and fast**: ~4 MB release binary, a few MB of daemon RSS, sub-10 ms
  `status` over the local socket. Pure-Rust D-Bus (SNI tray) and TLS stack; a
  musl-static build runs on any x86-64/aarch64 Linux.
- **Zero configuration**: if a tool is signed in, RunwayBar finds its local
  credentials. Absent tools show as *not installed* and stay quiet.
- **Read-only, provably**: RunwayBar never writes, refreshes or rewrites the
  tools' credential files — a CI guard runs the test suite under `strace` and
  fails on any write to those paths. Unknown data renders as *unknown*, never 0%.
- **Resilient**: failed polls keep the previous numbers marked stale; HTTP 429s
  put a provider on a five-minute cooldown; one provider failing never blanks
  the others.
- **Composable surfaces**: tray icon and menu (GNOME-with-appindicators, KDE,
  XFCE, Budgie, COSMIC, …), `--format waybar` for tiling bars, `--format json`
  for everything else, and a same-user Unix socket for scripts.

## Install

```sh
# .deb (Ubuntu 22.04+, Debian, ...)
curl -fL https://github.com/nitishagar/runwaybar/releases/latest/download/runwaybar_amd64.deb -o runwaybar.deb
sudo apt install ./runwaybar.deb

# tarball (any x86-64/aarch64 Linux; musl-static available)
curl -fsSL https://raw.githubusercontent.com/nitishagar/runwaybar/main/scripts/install.sh | sh

# from source
cargo install --git https://github.com/nitishagar/runwaybar
```

Then:

```sh
runwaybar serve              # start the tray daemon
runwaybar install            # add to ~/.config/autostart (login start)
```

On GNOME without the AppIndicator extension the daemon prints a notice and keeps
running — the CLI and socket still work.

## Usage

| Command | What it does |
|---|---|
| `runwaybar status` | Show usage (formats: `text`, `json`, `waybar`) |
| `runwaybar status --no-fetch` | Never touch the network (cache/daemon only) |
| `runwaybar refresh` | Ask the daemon (or do a one-shot) refresh now |
| `runwaybar serve` | Resident daemon: tray + IPC + scheduler |
| `runwaybar serve --dry-run` | Print the menu/tooltip tree (no D-Bus) |
| `runwaybar install` / `uninstall` | Manage the XDG autostart entry |
| `runwaybar smoke` | One-shot live check (opt-in via `RUNWAYBAR_LIVE_SMOKE=1`) |

### Waybar example

```json
"custom/runwaybar": {
  "exec": "runwaybar status --format waybar",
  "return-type": "json",
  "interval": 30
}
```

### Configuration

`~/.config/runwaybar/config.toml` (written with defaults on first run, `0600`,
read once at start):

```toml
interval = 300            # poll cadence in seconds, 60..=3600

[providers]
claude-code = true
codex = true
zai = true
opencode = true
muse = true

[notifications]
level = "warnings"        # off | warnings | all
```

## How it works (and what it will never do)

RunwayBar reads the same local files the CLIs own — `~/.claude/.credentials.json`,
`~/.codex/auth.json`, `~/.zcode/v2/config.json`,
`~/.local/share/opencode/auth.json`, `~/.config/muse/auth.json` — and queries
each vendor's own usage
endpoint with the token that vendor issued. Tokens are wrapped in a redacted
type that cannot render through logs, JSON or errors; they are never sent
anywhere except back to the issuing vendor over TLS. There is no login flow, no
credential storage, no telemetry and no backend — if you have no daemon
running, nothing talks to the network at all except an explicit `status` whose
cache has gone stale.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the hard invariants (read-only
credentials, no secrets in output, unknown-stays-unknown) and how they are
enforced in CI.

## Development

```sh
cargo test                                     # full suite
cargo clippy --all-targets -- -D warnings
scripts/strace-readonly-check.sh               # read-only credentials guard
scripts/e2e-daemon.sh                          # daemon lifecycle e2e (mock-backed)
python3 scripts/check-contrast.py              # landing-site contrast budget
```

Landing site: `site/` (deployed to GitHub Pages by `.github/workflows/pages.yml`;
custom-domain routing runbook for `runwaybar.applair.in` in
[docs/domain-routing-applair.md](docs/domain-routing-applair.md)).

## License

Apache-2.0 — see [LICENSE](LICENSE).
