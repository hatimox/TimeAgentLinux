# TimeAgent (Linux native)

A **native Linux (Rust + GTK4 + ksni tray)** rewrite of TimeAgent — a tray app
that logs meeting and task time to **TargetProcess**. Native to avoid the
Electron event-loop / per-poll `pactl`-spawn fragility that caused the
meeting-detection freezes: here, microphone detection is an **in-process
PulseAudio/PipeWire query** and the loop runs on a `tokio` interval.

Reuses the same config + token as the Electron app
(`~/.config/TimeAgent/settings.json` + the libsecret entry
`net.omnevo.timeagent`/`tp-token`), so an existing setup is picked up.

## Install (prebuilt packages)

Each tagged release publishes packages built by GitHub Actions
([`.github/workflows/release.yml`](.github/workflows/release.yml)) for x86_64:

| Artifact | Install |
|----------|---------|
| `timeagent_<ver>_amd64.deb`       | `sudo apt install ./timeagent_<ver>_amd64.deb` |
| `timeagent-<ver>.x86_64.rpm`       | `sudo dnf install ./timeagent-<ver>.x86_64.rpm` |
| `TimeAgent-<ver>-x86_64.AppImage`  | `chmod +x TimeAgent-*.AppImage && ./TimeAgent-*.AppImage` |
| `timeagent-<ver>-x86_64-linux.tar.gz` | extract and run `./timeagent` (needs GTK4 + libpulse/libsecret installed) |

Packages require **GTK 4.10+** (so a reasonably recent distro). To cut a release,
push a tag: `git tag v0.1.0 && git push origin v0.1.0`.

A CI workflow ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) builds and
lints every push/PR to `main`.

## Build (on Ubuntu)

System dependencies:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config \
  libgtk-4-dev \
  libpulse-dev \
  libsecret-1-dev \
  libssl-dev
# Rust toolchain (if not installed):
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then:

```bash
cargo build --release
./target/release/timeagent
```

> **GNOME tray note:** the tray uses StatusNotifierItem (the modern standard).
> On vanilla GNOME you need the *AppIndicator and KStatusNotifierItem Support*
> shell extension for the icon to appear (KDE/Plasma shows it natively). Same
> requirement the Electron build had.

## Module map — all features written

| File | Role |
|------|------|
| `src/models.rs`     | data types |
| `src/settings.rs`   | JSON config + libsecret token |
| `src/tpclient.rs`   | async TP REST client, noon-anchored dates, pagination, status change, time edit/delete |
| `src/mic.rs`        | in-process PulseAudio/PipeWire mic detection (no `pactl` spawn) |
| `src/watcher.rs`    | tokio meeting loop + Split / Stop-tracking / bounded suppression |
| `src/holidays.rs`   | Morocco civil holidays + day-off check |
| `src/store.rs`      | shared state, all async TP ops, recurring auto-log, glib notifications |
| `src/ui_tasks.rs`   | tasks/bugs window: search, filter, scope, status DropDown, US link, hours, direct log, edit/delete entries |
| `src/ui_settings.rs`| settings tabs: account, meetings, recurring, days off |
| `src/ui_prompt.rs`  | end-of-meeting dialog + task picker + defined-meeting picker |
| `src/main.rs`       | GTK app + tokio runtime + ksni tray + glib bridge |

## IMPORTANT: unverified — build on Ubuntu first

Every feature is written, but **none of this was compiled** — it was authored on
macOS, which has no Rust/GTK4 toolchain. Expect `cargo build` to surface errors
to fix (the gtk4-rs bindings are strict). Highest-risk spots, in order:

1. **`mic.rs` libpulse threaded-mainloop** — the introspect calls likely need
   explicit `mainloop.lock()`/`unlock()` guards and an operation-state check
   rather than the poll-until-done pattern used. Most likely first failure.
2. **gtk4-rs API drift** — method names / builder signatures (e.g.
   `DropDown::new`, `glib::MainContext::channel`, `AlertDialog::choose`) vary by
   crate version. Pin versions or adjust to the installed gtk4 crate's API.
3. **glib `Sender` Send-ness** — `glib::Sender` is used to push updates from the
   tokio thread; confirm it crosses threads cleanly (it should — it's `Send`).
4. **ksni `MenuItem`/`StandardItem` API** — adjust to the ksni 0.2 API if names differ.

Validate after it builds: `mic_in_use()` true in a real call; logging round-trips
to TP; status change and time edit/delete work; the meeting prompt appears.

The macOS sibling lives at `../TimeAgentMac` (compiles) and the original Electron
app at `../TimeAgentElectron` — reference both for exact behavior parity.
