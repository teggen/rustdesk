# Handoff: applying the hbb_common submodule changes in a new session

The silent-direct-access feature (plus the per-connection override / close-prevention
controls) needs changes in the **hbb_common submodule** (`teggen/hbb_common`).
Those changes are committed locally in this session but **could not be pushed**
because the session was scoped to `teggen/rustdesk` only. The container is
ephemeral, so the local submodule commits will be lost — these patches are the
durable copy.

- Patch series: [`hbb_common-patches/`](./hbb_common-patches/)
  - `0001-config-add-OPTION_ALLOW_SILENT_DIRECT_ACCESS-key.patch`
  - `0002-proto-add-per-connection-silent-mode-close-controls-.patch`
- Parent-repo branch (already pushed): `claude/pensive-einstein-m38yn4`
  - `.gitmodules` already points `libs/hbb_common` at `https://github.com/teggen/hbb_common`.
  - The submodule **pointer is intentionally NOT bumped yet** (still the upstream
    pinned commit `e50ac3c…`), because the new commits aren't on the remote.

## What the patches do

1. `config.rs`: add `OPTION_ALLOW_SILENT_DIRECT_ACCESS` (`allow-silent-direct-access`)
   and register it in `KEYS_SETTINGS`.
2. `protos/message.proto`: add three `LoginRequest` fields signalled by the
   connecting client:
   - `override_silent_mode = 18` — show CM window + tray for this session despite silent mode.
   - `auto_hide_tray = 19` — hide the tray again when this session ends.
   - `prevent_close = 20` — disable server-side close/disconnect for this session.

   The proto is compiled at build time (build.rs), so no generated code is
   checked in — the new accessors appear after `cargo build`.

## Steps in the new session (with `teggen/hbb_common` access enabled)

```bash
cd <repo>                       # the rustdesk checkout
git checkout claude/pensive-einstein-m38yn4
git pull origin claude/pensive-einstein-m38yn4

git submodule sync libs/hbb_common
git submodule update --init libs/hbb_common

cd libs/hbb_common
git checkout -b claude/pensive-einstein-m38yn4
git config user.email noreply@anthropic.com
git config user.name Claude
git am ../../docs/research/hbb_common-patches/*.patch
#   (fallback if `git am` is fussy:
#    for p in ../../docs/research/hbb_common-patches/*.patch; do git apply "$p"; done
#    git commit -am "hbb_common: silent-direct-access option + login controls")

git push -u origin claude/pensive-einstein-m38yn4
cd ../..

# Bump the parent submodule pointer to the new fork commit and push
git add libs/hbb_common
git commit -m "build: bump hbb_common to silent-direct-access + login controls"
git push origin claude/pensive-einstein-m38yn4
```

## Then verify the build

The previous session could **not** compile (the full build fetches a dependency
from `chromium.googlesource.com`, blocked by the network policy). Once the
submodule is wired up and the network allows, run:

```bash
cargo check
```

Pay attention to:
- `OPTION_ALLOW_SILENT_DIRECT_ACCESS` resolving in `src/server/connection.rs`,
  `src/tray.rs`.
- `LoginRequest::override_silent_mode / auto_hide_tray / prevent_close` accessors
  in `src/client.rs` and `src/server/connection.rs`.
- `tray-icon` API: this code uses `TrayIcon::set_visible(bool)` (crate
  `tray-icon` 0.21.x). If that method name differs in the pinned version, adjust
  `src/tray.rs` (the only user).

## What the parent branch already contains

All non-submodule work is committed on `claude/pensive-einstein-m38yn4`:

Silent direct access (first feature):
- `src/server/connection.rs`, `src/server.rs`, `src/rendezvous_mediator.rs`,
  `src/tray.rs`, `src/core_main.rs`, Flutter settings, `src/lang/*`.

Per-connection override / close-prevention (this feature):
- `src/ipc.rs` — `prevent_close` in `Data::Login`; `force_show_tray` config query.
- `src/server/connection.rs` — tray force-show counters; honor `override_silent_mode`
  in the CM-launch decision; `prevent_close` enforcement; propagate to CM.
- `src/ui_cm_interface.rs` — `Client.prevent_close` + `add_connection` plumbing.
- `src/tray.rs` — dynamic icon visibility via `set_visible`, polling the server's
  `force_show_tray` signal.
- `src/client.rs` — send the three fields from per-peer options
  (`override-silent-mode`, `auto-hide-tray`, `prevent-server-close`).
- `flutter/lib/models/server_model.dart` — `Client.preventClose`.
- `flutter/lib/desktop/pages/server_page.dart` — hide the Disconnect button when
  `preventClose`.
- `flutter/lib/consts.dart` — option-key constants.
- `src/lang/*` — `close_prevented_by_peer_tip` (+ silent-mode strings).

## Remaining integration point (client UI)

The client *sends* the per-peer flags (read at connect time from the peer
config), but there is **no pre-connect UI toggle yet** to set
`override-silent-mode` / `auto-hide-tray` / `prevent-server-close`. They can be
set programmatically on the peer config today. A small per-peer settings UI
(e.g. in the address-book/peer-card edit dialog) is the recommended follow-up;
the option keys are already defined in `flutter/lib/consts.dart`.
