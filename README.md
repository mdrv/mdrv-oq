# mdrv-oq

> [!CAUTION]
> **Personal software, maintained for the author only.** The repo is public so
> the CI and release pipeline have a home, not because it is a supported
> product. Development is heavily LLM-assisted: most commits are written with
> AI agents and verified only against the author's own devices (a Linux
> desktop with Quickshell, and a Poco F7). Expect breaking changes at any
> bump, no support, no stability guarantee, no release schedule. Read it as a
> reference for your own [nsh](https://github.com/longbridge/nsh) app; if you
> want a supported quota indicator, use `oc-quota` directly.

A GLM / Z.AI Coding Plan quota monitor, built as a native app on
[nsh](https://github.com/longbridge/nsh): a tray app on Linux and Windows
(colored percentage icon) and an Android app with a persistent
foreground-service notification.

```
GLM · Z.AI Coding Plan                    [Hide]

                 27%
                  ok
[████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░]
Resets      20:35:16 · 3h14m
Level       lite
Interval    5m
Checked     2m ago

Settings
Alert threshold                      −  +
90%
Reset time                        toggle
shown
Poll interval                       −  +
5m
Theme                             cycle
system
API key (overrides env / OpenCode) Save  Clear
```

## What it does

- Polls `GET https://api.z.ai/api/monitor/usage/quota/limit` with the raw
  key in the `Authorization` header and parses the `TOKENS_LIMIT` row
  (percentage used, next reset, plan level).
- **Linux tray**: icon renders the current percentage in pixels —
  green → amber (≥ 80%) → red (≥ threshold); full details in the tooltip;
  menu with Open / Refresh now / Quit; clicking opens the dashboard.
- **Windows tray**: the same colored percentage icon in the notification
  area; a primary click opens the dashboard, the menu is the secondary
  click.
- **Android**: tapping the launcher icon opens the dashboard; a
  foreground service (`specialUse`) keeps a silent ongoing notification —
  `GLM 27% · ♻️ 20:35:16` with a SystemUI chronometer for the last-poll
  age. The service is the **only poller** on Android; the UI mirrors its
  `state.json`, so the app and the notification always agree.
- **Alerts**: one silent notification when the percentage first crosses
  the threshold within a reset cycle, and one when a fetch fails after a
  previously-good poll (desktop; threshold notifications are desktop-side).
- **Fail-closed**: no key, network error, or API error renders `✖`/`…`
  with the message — never a stale green number. Failed fetches keep the
  last good snapshot and mark it stale.

## API key resolution

1. the key saved in the app (Settings → API key), then
2. the `ZAI_API_KEY` environment variable (desktop), then
3. OpenCode v2's credential database (`opencode.db`, read-only, WAL-safe;
   desktop only), then
4. OpenCode's legacy `auth.json` (desktop only).

The key is stored in the local settings database and is never logged or
rendered — only its origin is shown (`key: app settings`).

## Install

### Android

Grab `mdrv-oq-vX.Y.Z-android.apk` from
[Releases](https://github.com/mdrv/mdrv-oq/releases) (arm64-v8a, debug-key
signed for now), install it, open the app once, and grant the notification
permission. The persistent notification starts with the app; on Xiaomi /
HyperOS also enable _Autostart_ for the app and set battery saver to _No
restrictions_, or MIUI will freeze the service.

### Linux (desktop)

Build from source — the tray needs a StatusNotifierItem host (Quickshell,
Plasma, Waybar, GNOME with AppIndicator):

```sh
git clone https://github.com/mdrv/mdrv-oq
cd mdrv-oq
cargo run --release     # deps: GTK 4 ≥ 4.14 dev packages + the nsh CLI
```

The nsh CLI (only needed for packaging, not for `cargo run`) installs with
`curl -fsSL https://nsh.dev/install.sh | sh`.

Release tarballs from CI are built with `nsh package --target linux` and
need `gtk4` + `pango` at runtime.

### Windows

Build from source (no system packages needed; Rust 1.99.1+):

```sh
git clone https://github.com/mdrv/mdrv-oq
cd mdrv-oq
cargo run --release
```

The icon keeps its colors in the notification area; the tooltip carries the
details. Releases currently ship Linux and Android artifacts only.

## Settings

| Setting         | Values                   | Notes                                              |
| --------------- | ------------------------ | -------------------------------------------------- |
| Alert threshold | 50–99%                   | fires once per reset cycle                         |
| Reset time      | shown / hidden           | also controls the notification's `♻️` clock        |
| Poll interval   | 1m / 5m / 10m / 30m / 1h | Android applies it on the next cycle, no reinstall |
| Theme           | system / light / dark    |                                                    |
| API key         | masked entry             | Save / Clear; overrides env / OpenCode             |

## State

- Linux: `~/.local/state/mdrv-oq/settings.db` (Turso/SQLite) — override
  with `MDRV_OQ_STATE_DIR`.
- Windows: `%LOCALAPPDATA%\mdrv-oq\settings.db` — same override.
- Android: `files/state/` inside the app's data dir — `settings.db` shared
  between the Rust UI and the Java service, plus `state.json` written by
  the service for the UI to mirror.

## How it's built

One Rust core (`crates/mdrv-oq-core`: fetch, parse, key resolution,
thresholds, rendering) with a shared nsh UI on desktop; on Android the
packaged app runs the same UI and a small framework-only Java foreground
service (`android/`) polls on top of the same settings database — assembled
without Gradle by `android/build-app.sh` (`aapt2` / `d8` / `zipalign` /
`apksigner`).

The desktop tray uses a patched nsh
([`mdrv-oq/tray-api`
branch](https://github.com/mdrv/nsh/tree/mdrv-oq/tray-api) of a public
fork, wired in via `[patch.crates-io]`): upstream treats every SVG tray
icon as a monochrome template; the patch keeps colored SVGs colored, on
Linux and Windows. It has been
[offered upstream](https://github.com/longbridge/nsh).

## License

MIT — see [LICENSE](LICENSE).
