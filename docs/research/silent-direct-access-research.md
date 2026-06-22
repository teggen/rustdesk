# Research: "Silent mode" for incoming direct‑IP connections

**Status:** Research / design notes (no code changed)
**Date:** 2026-06-22
**Goal:** Allow RustDesk to be configured so that, when somebody connects to this
computer over the local network (direct IP access), the machine stays *silent*:

1. No Connection Manager (CM) dialog opens on connect / disconnect / file transfer.
2. No RustDesk icon appears in the system tray.
3. The option is available in the Settings dialog.
4. The option can be overridden from the CLI.

This document maps the relevant code paths. The concrete step‑by‑step plan lives in
[`silent-direct-access-implementation-plan.md`](./silent-direct-access-implementation-plan.md).

---

## 1. How an incoming connection becomes a visible CM dialog

### 1.1 Where direct‑IP connections enter

Direct‑IP access has its **own listener**, separate from the ID/rendezvous and relay
paths:

- `src/rendezvous_mediator.rs:850` — `async fn direct_server(server)`
  - Gated by the `direct-server` option (`OPTION_DIRECT_SERVER`) and `stop-service`
    (`src/rendezvous_mediator.rs:854`).
  - Listens on `direct-access-port` (default `RENDEZVOUS_PORT + 2`, i.e. `21118`),
    see `get_direct_port()` at `src/rendezvous_mediator.rs:840`.
  - On accept it calls `create_tcp_connection(server, stream, addr, false, None)`
    (`src/rendezvous_mediator.rs:899`). **This is the single choke point that
    distinguishes a direct‑IP connection** — `secure = false`,
    `control_permissions = None`, and it is the only caller reached from
    `direct_server`.

Other entry points (for contrast):
- ID/punch‑hole/intranet: `src/rendezvous_mediator.rs:632` and `:715`
  (`crate::accept_connection(... secure=true ...)`).
- Relay: `src/server.rs:315` `create_relay_connection_` → `create_tcp_connection` at
  `src/server.rs:337`.

All paths converge on:
- `src/server.rs:190` `create_tcp_connection(...)` → `src/server.rs:263`
  `Connection::start(addr, stream, id, server, control_permissions)`.

> **Key takeaway:** to scope "silent" behaviour to *direct‑IP only*, a boolean
> (e.g. `from_direct_ip`) must be threaded from `direct_server` through
> `create_tcp_connection` → `Connection::start` → the `Connection` struct. Every
> other caller passes `false`.

### 1.2 From `Connection` to the CM window

The controlled side runs the CM as a **separate process** that talks to the server
process over IPC.

- `src/server/connection.rs:311` — the `Connection` struct (fields:
  `tx_to_cm`, `start_cm_ipc_para`, `ip`, `file_transfer`, `terminal`, …).
- On login the server decides whether to spawn / talk to the CM:
  - `try_start_cm_ipc()` — `src/server/connection.rs:2338`
  - `try_start_cm(peer_id, name, authorized)` — `src/server/connection.rs:1976`
    → `send_to_cm(ipc::Data::Login { … })` (`:2001`).
  - These are invoked from the login handler around
    `src/server/connection.rs:2468`, `:2544`, `:2559`, `:2606`, etc., based on
    `approve_mode` / password state.
- The CM process is actually launched in `run_cm_ipc` /`start_ipc`:
  - `src/server/connection.rs:5265`+ decides the launch arguments.
  - `let mut args = vec!["--cm"];` at `src/server/connection.rs:5295`.
  - On Linux headless it already switches to `args = vec!["--cm-no-ui"];`
    (`src/server/connection.rs:5342`), driven by
    `headless_cm = is_server() && is_headless_allowed() && is_headless()`
    (`:5280`). **This is the existing precedent for a windowless CM.**
  - Process is spawned via `crate::platform::run_as_user(args …)`
    (`:5366` / `:5371`).

### 1.3 The CM process and its window

- Entry dispatch: `src/core_main.rs:707`‑`718` handles `--cm` and `--cm-no-ui`.
  - `--cm` → returns `Some(args)` so the Flutter UI starts.
  - `--cm-no-ui` → starts the headless connection handler and returns `None`
    (no Flutter window).
- Flutter CM screen: `flutter/lib/main.dart:290` `runConnectionManagerScreen()`.
  - Reads `hide = await bind.cmGetConfig(name: "hide_cm") == 'true'`
    (`flutter/lib/main.dart:297`) and either `showCmWindow()` or
    `hideCmWindow(isStartup: true)`.
- IPC `Data::Login` is received in `src/ui_cm_interface.rs` (the
  `start_ipc` task loop, ~`:546`) and forwarded to the UI via
  `InvokeUiCM::add_connection`.
- Flutter side: `src/flutter.rs:1505` `add_connection()` pushes the
  `add_connection` event → `flutter/lib/models/model.dart:395` →
  `ServerModel.addConnection()` (`flutter/lib/models/server_model.dart:543`).
- The window is shown here:
  ```dart
  if (desktopType == DesktopType.cm && !hideCm) {
    showCmWindow();              // server_model.dart:577
  }
  ```

### 1.4 Connect / disconnect / file‑transfer notifications

- **Connect:** `InvokeUiCM::add_connection` (`src/ui_cm_interface.rs:184`,
  `src/flutter.rs:1505`).
- **Disconnect:** `on_close()` (`src/server/connection.rs:4620`) sends
  `ipc::Data::Disconnected` / `Data::Close` to the CM (`:4648`);
  `remove_connection()` (`src/ui_cm_interface.rs:~282`) → Flutter
  `on_client_remove` event.
- **File transfer:** CM receives `Data::FS` / `Data::FileTransferLog`
  (`src/ui_cm_interface.rs:~596`) → `file_transfer_log()`
  (`src/flutter.rs:1556` pushes `cm_file_transfer_log`) →
  `flutter/lib/models/cm_file_model.dart:29 onFileTransferLog`.

All three are surfaced through the **same CM window**. If the CM runs in
`--cm-no-ui` mode (or the window is permanently hidden), none of these produce a
visible dialog. The `--cm-no-ui` handler still processes the IPC stream (auth,
permissions, file jobs) but renders nothing.

---

## 2. The existing `allow-hide-cm` feature (and why it is not enough)

RustDesk already has a "hide connection manager" option, but it is narrow:

- Flutter UI toggle: `flutter/lib/desktop/pages/desktop_setting_page.dart:1454`
  `hide_cm()` writing `allow-hide-cm` (`:1463`). Tip string `hide_cm_tip`
  = *"Allow hiding only if accepting sessions via password and using a permanent
  password"*.
- Authoritative gate is in Rust:
  - `src/ipc.rs:863` — `cmGetConfig("hide_cm")` only returns a value when
    `is_pro() || is_custom_client()`; otherwise `None`.
  - The actual boolean is `hbb_common::password_security::hide_cm()` (lives in the
    `libs/hbb_common` submodule), which additionally requires
    `approve_mode == "password"` **and** a permanent password.
- Sciter equivalent: `src/ui/cm.rs:165` `hide_cm()` + `src/ui.rs:121`/`:186`.

**Limitations for our use‑case:**
- Gated behind **Pro / custom client** builds.
- Requires permanent‑password approve mode.
- Only hides the **window**; the CM process and the **tray** still run.
- Not scoped to direct‑IP / LAN connections.
- The Dart‑side dynamic re‑evaluation is currently commented out
  (`server_model.dart:138-148`, `:240-247`, `:282-294`), so it is effectively a
  startup‑only flag today.

We will reuse the *plumbing patterns* (a `cmGetConfig`‑style gate, the
`--cm-no-ui` launch path) but introduce a **new, ungated, direct‑IP‑scoped
option**.

---

## 3. The system tray

- `src/tray.rs:11` `start_tray()` returns early (no icon) when the **builtin**
  option `OPTION_HIDE_TRAY` is `"Y"`:
  ```rust
  if crate::ui_interface::get_builtin_option(keys::OPTION_HIDE_TRAY) == "Y" { return; }
  ```
- `make_tray()` (`src/tray.rs:25`) builds the icon and also re‑checks
  `OPTION_HIDE_TRAY` at `:141`. `OPTION_HIDE_STOP_SERVICE` (`:57`) hides the
  "Stop service" menu item only.
- The tray is a **global, per‑machine process**, not per‑connection. It is
  launched:
  - auto, on empty args, when a server is detected:
    `src/core_main.rs:82`‑`96` (`run_me(vec!["--tray"])`).
  - explicitly via `--tray`: `src/core_main.rs` `--tray` branch
    (calls `crate::tray::start_tray()`).
  - from `--server` startup on Linux/macOS.

> **Important consequence:** "no tray icon" is inherently a **global** setting —
> it cannot depend on whether a *particular* connection is direct‑IP, because the
> tray starts before any connection exists. `OPTION_HIDE_TRAY` is currently a
> *builtin* (admin/deploy) setting read via `get_builtin_option`, **not** a
> normal user‑editable option. Our feature needs `start_tray`/`make_tray` to also
> honour the new user option.

---

## 4. The options / settings system

(From `libs/hbb_common/src/config.rs` — the **`hbb_common` submodule**, currently
not checked out in this working copy — plus the Rust/Flutter glue in this repo.)

- **String→bool semantics** (`config.rs::option2bool`, mirrored in
  `flutter/lib/common.dart:1583` and Sciter):
  - `enable-*` → true unless value is `"N"`.
  - `allow-*`, `stop-service`, `direct-server`, `force-always-relay` → true only
    when value is `"Y"`.
  - otherwise → true unless `"N"`.
  - **An `allow-…` name therefore defaults to OFF**, which is what we want.
- **Get/Set:** `src/ui_interface.rs:163` `get_option`, `:422` `set_option`
  (in‑memory `OPTIONS` map persisted to the `--server` process via IPC).
- **Builtin/override tiers** (`src/common.rs:~2119`): `BUILTIN_SETTINGS`,
  `OVERWRITE_SETTINGS`, `DEFAULT_SETTINGS`, `HARD_SETTINGS`.
  - `src/ui_interface.rs:213` `is_option_fixed` → an option present in an
    `OVERWRITE_*` map is rendered read‑only in the UI.
  - `get_builtin_option` (used by the tray) reads `BUILTIN_SETTINGS`.
- **Settings UI:** `flutter/lib/desktop/pages/desktop_setting_page.dart`
  - `_OptionCheckBox(context, 'Label', kOptionKey, …)` (`:2539`) renders a toggle,
    reads via `mainGetBoolOptionSync`, writes via `mainSetBoolOption`
    (`flutter/lib/common.dart:1616`/`:1625`), and respects `isOptionFixed`.
  - The **Security ("safety") tab** (`:1086`+) is where connection‑permission
    toggles live (`enable-clipboard`, `enable-file-transfer`, `allow-hide-cm`,
    …) — the natural home for the new toggle.
- **Constants:** `flutter/lib/consts.dart:112`+ (`kOption…`).
- **Translations:** `src/lang/*.rs` (one `HashMap` per language; key →
  localized string), resolved by `src/lang.rs:161 translate_locale`, surfaced to
  Flutter via `translate()` (`flutter/lib/common.dart:1573`).

---

## 5. The CLI

`src/core_main.rs::core_main()` parses argv and dispatches modes. Relevant facts:

- Early flag extraction loop (`src/core_main.rs:50`‑`81`) pulls out
  `--elevate`, `--run-as-system`, `--quick_support`, `--no-server` before the
  main dispatch; everything else is collected into `args`.
- Modes return `Option<Vec<String>>`: `None` = exit after handling, `Some` =
  continue to the Flutter UI.
- Existing config‑from‑CLI mechanism:
  - `--option <key> [value]` (`src/core_main.rs:521`‑`536`) — get or set any
    option via IPC, **requires installed + root**, and is blocked when
    settings are disabled unless
    `OPTION_ALLOW_COMMAND_LINE_SETTINGS_WHEN_SETTINGS_DISABLED == "Y"`.
  - `--config`, `--import-config`, `--password`, `--set-id`, etc.
- There is **no** existing `--silent` / `--no-tray` flag.

> **Two complementary CLI surfaces exist:** (a) the generic, persistent
> `--option allow-silent-direct-access Y`; and (b) a dedicated, ergonomic
> override flag (e.g. `--silent` / `--no-silent`) that can set the option and/or
> influence the current process (e.g. the `--tray` process). The plan uses (a)
> as the backbone and adds (b) as a thin wrapper for discoverability.

---

## 6. Summary of integration points

| Behaviour to suppress | Code that produces it | Hook for silent mode |
|---|---|---|
| Direct‑IP detection | `direct_server` → `create_tcp_connection(.. None)` (`rendezvous_mediator.rs:899`) | thread `from_direct_ip` into `Connection` |
| CM window on connect | `server_model.dart:577 showCmWindow()` / launch `--cm` (`connection.rs:5295`) | launch `--cm-no-ui` for silent direct conns |
| Disconnect dialog | `on_close` (`connection.rs:4620`) → `remove_connection` | no‑op when CM is no‑UI |
| File‑transfer dialog | `file_transfer_log` (`flutter.rs:1556`) | no‑op when CM is no‑UI |
| Tray icon | `tray.rs:11 start_tray` / `:25 make_tray` | also honour new option (global) |
| Settings UI | `desktop_setting_page.dart` Security tab | add `_OptionCheckBox` |
| CLI | `core_main.rs` arg dispatch | `--option` + new `--silent`/`--no-silent` |
| Option storage/keys | `hbb_common` submodule `config.rs` | add `OPTION_ALLOW_SILENT_DIRECT_ACCESS` |
