# Implementation Plan: "Silent mode" for incoming direct‑IP connections

**Companion document:** [`silent-direct-access-research.md`](./silent-direct-access-research.md)
**Date:** 2026-06-22

## 1. Objective

Add a configurable **silent mode** so that when a peer connects over the local
network via **direct IP access**, the controlled machine shows nothing:

1. No Connection Manager dialog on connect / disconnect / file transfer.
2. No tray icon.
3. A toggle in the Settings dialog.
4. A CLI override.

## 2. Design overview

Introduce **one new user option**:

```
allow-silent-direct-access      # "Y" = on, "" = off (allow-* ⇒ defaults OFF)
```

Chosen prefix `allow-` so the existing `option2bool`/`bool2option` rules treat it
as a boolean that defaults to **off** (see research §4) — no new parsing logic.

The option drives **two independent behaviours**:

- **(A) Per‑connection (CM):** when an incoming connection arrives through the
  **direct‑IP listener** *and* the option is on, the server starts/uses the CM in
  **headless `--cm-no-ui` mode**, reusing the existing windowless path. No window,
  no connect/disconnect/file‑transfer dialogs.
- **(B) Global (tray):** `start_tray()` / `make_tray()` additionally suppress the
  tray icon when the option is on.

> **Why scope (A) to the listener and not to "is the IP private?"**
> The direct‑IP listener (`rendezvous_mediator.rs:850 direct_server`) is the
> single, unambiguous source of direct connections and already passes a distinct
> signature `create_tcp_connection(.., secure=false, None)`. This is more robust
> than sniffing the peer address. (Optional refinement: additionally require the
> peer IP to be private/LAN — see §7.)

> **Why a single option for both (A) and (B)?**
> The user asked for one coherent "be invisible for local direct access" mode.
> The tray is inherently global (research §3), so the same toggle hides it
> machine‑wide while the CM suppression remains scoped to direct connections. If
> finer control is wanted later, split into two options (§7).

### 2.1 Interaction with authentication

Silent mode suppresses **UI**, not **authorization**. For a genuinely
hands‑off experience the operator should pair it with an auth mode that needs no
human at the controlled side — i.e. a **permanent password** (or an accept‑mode
that auto‑authorizes). This mirrors the constraint behind the existing
`allow-hide-cm` and should be stated in the tooltip. The headless CM
(`--cm-no-ui`) already runs the full auth/permission flow; it just renders
nothing. (Click‑to‑accept approve mode would block forever with no UI, so the
tooltip/docs must warn against combining silent mode with click‑approval.)

## 3. Work items

### Step 1 — Define the option key (`hbb_common` submodule)

> ⚠️ `libs/hbb_common` is a **git submodule** and is *not checked out* in this
> working copy. These edits land in the `rustdesk/hbb_common` repo and the
> submodule pointer is bumped here.

In `libs/hbb_common/src/config.rs`, `mod keys`:

```rust
pub const OPTION_ALLOW_SILENT_DIRECT_ACCESS: &str = "allow-silent-direct-access";
```

- Add it to the keys list that is recognised as a valid server option (the same
  list/registry that `OPTION_ALLOW_AUTO_DISCONNECT`, `OPTION_HIDE_TRAY`, etc. are
  registered in — search `config.rs` for `OPTION_HIDE_TRAY` and follow the same
  array/`KEYS_SETTING`‑style registration so the value round‑trips through
  `OPTIONS`, `--option`, and custom‑client settings maps).
- `option2bool`/`bool2option` already handle the `allow-` prefix — **no change**.

### Step 2 — Thread a `from_direct_ip` flag into `Connection`

Goal: let the server know a given connection came from the direct‑IP listener.

1. `src/rendezvous_mediator.rs:899` (`direct_server`): pass `true` for a new
   parameter:
   ```rust
   crate::server::create_tcp_connection(server, stream, addr, false, None /*,*/ )
   ```
   → add `from_direct_ip: true`.
2. `src/server.rs:190 create_tcp_connection(...)` and `:263 Connection::start(...)`:
   add a `from_direct_ip: bool` parameter and forward it. **All other callers**
   pass `false`:
   - `src/server.rs:178` (`accept_connection_`)
   - `src/server.rs:337` (`create_relay_connection_`)
   - `src/rendezvous_mediator.rs:632`, `:715` (`accept_connection`) — note these
     go through `accept_connection`/`accept_connection_`, so the flag is added to
     that chain too (default `false`).
3. `src/server/connection.rs:311` (`Connection` struct): add field
   `from_direct_ip: bool`, initialise it in `Connection::start` /the constructor
   (near `from_switch: false` at `:505`).

### Step 3 — Decide silent at CM launch time

The launch‑args decision lives in the CM‑bridge routine around
`src/server/connection.rs:5280`‑`5343` (where `headless_cm` currently selects
`--cm-no-ui`). Extend it:

```rust
let silent_direct = conn_is_direct_ip
    && config::option2bool(
        keys::OPTION_ALLOW_SILENT_DIRECT_ACCESS,
        &Config::get_option(keys::OPTION_ALLOW_SILENT_DIRECT_ACCESS),
    );
// existing:
let headless_cm = /* linux headless detection */;
// new:
if headless_cm || silent_direct {
    args = vec!["--cm-no-ui"];
}
```

- `conn_is_direct_ip` is the `from_direct_ip` flag plumbed in Step 2, made
  available to this routine (it runs as part of the connection's CM bridge, so it
  can capture `self.from_direct_ip` when `start_cm_ipc_para` is built around
  `:427`/`:467`, or be passed alongside the existing params).
- Because the **first** direct connection spawns the CM process, and silent mode
  is intended for setups where *all* inbound is direct‑IP, this is sufficient for
  the target scenario. See §6 for the mixed‑mode caveat.

**Result:** the CM runs windowless; `add_connection`, `remove_connection`, and
`file_transfer_log` all still flow over IPC but render nothing — satisfying
requirement (1).

### Step 4 — Suppress the tray icon (global)

In `src/tray.rs`:

- `start_tray()` (`:11`): in addition to the `OPTION_HIDE_TRAY` builtin check,
  return early when the user option is on:
  ```rust
  let silent = crate::ui_interface::get_option(
      keys::OPTION_ALLOW_SILENT_DIRECT_ACCESS) == "Y";
  if silent { /* same early-return as hide-tray, incl. macOS handling */ }
  ```
  Mirror the existing macOS special‑case at `:13`‑`:16` and the second check in
  `make_tray()` at `:141`.
- Use `get_option` (user option), not `get_builtin_option`. If the option should
  *also* be settable as a deploy/builtin override, OR the two checks.

> The tray process is started by `--tray` (and auto‑start at
> `core_main.rs:82`‑`96`). No change needed there: the gate inside
> `start_tray()`/`make_tray()` covers every launch path.

### Step 5 — Settings UI toggle

1. Constant — `flutter/lib/consts.dart` (near line 112):
   ```dart
   const String kOptionAllowSilentDirectAccess = "allow-silent-direct-access";
   ```
2. Toggle — `flutter/lib/desktop/pages/desktop_setting_page.dart`, in the
   **Security tab** (near the other permission checkboxes, ~`:1086`+, e.g. just
   after `allow-hide-cm`/`hide_cm()`):
   ```dart
   _OptionCheckBox(
     context,
     'Silent for direct IP access',            // translation key
     kOptionAllowSilentDirectAccess,
     enabled: enabled,
     fakeValue: fakeValue,
   ),
   ```
   `_OptionCheckBox` (`:2539`) already handles read (`mainGetBoolOptionSync`),
   write (`mainSetBoolOption`), and `isOptionFixed` (override) for free.
3. (Optional) a `_Tip`/tooltip explaining it requires a permanent password and is
   incompatible with click‑to‑accept, like `hide_cm_tip`.

### Step 6 — Translations

Add the new label (and optional tip) key to **every** `src/lang/*.rs` map. At
minimum add to `src/lang/en.rs`; the resolver falls back to English for missing
keys (`src/lang.rs:161`), but follow the repo convention of populating all
languages (English text is acceptable as a placeholder for non‑en files, matching
how new strings are typically seeded):

```rust
("Silent for direct IP access",
 "Stay silent for direct IP (LAN) connections: no window, no tray icon"),
("silent_direct_access_tip",
 "Requires a permanent password. Not compatible with manual accept (click) mode."),
```

### Step 7 — CLI

Two layers:

1. **Generic (already works once Step 1 lands):**
   ```
   rustdesk --option allow-silent-direct-access Y    # enable
   rustdesk --option allow-silent-direct-access ''   # disable
   rustdesk --option allow-silent-direct-access      # read
   ```
   Handled at `src/core_main.rs:521`‑`536` (installed + root required; honours the
   "settings disabled" guard).

2. **Dedicated convenience flags** — add branches in `core_main` dispatch:
   - `--silent` → `crate::ipc::set_option("allow-silent-direct-access", "Y")`
     then `return None` (or continue, matching `--option` semantics).
   - `--no-silent` → set value to `""`.

   Place these next to the `--option` handling and reuse the same
   permission/settings‑disabled checks. Document that, like `--option`, they
   require an installed + privileged context to reach the running `--server`.

   *(Optional, advanced)*: if a `--tray` process is already running when the
   option flips, it won't re‑read the gate until restarted. For immediate effect
   the setter can additionally signal the tray to exit/relaunch — out of scope for
   v1; document that a service/tray restart applies tray changes.

## 4. Files to change (checklist)

| # | File | Change |
|---|------|--------|
| 1 | `libs/hbb_common/src/config.rs` *(submodule)* | add `OPTION_ALLOW_SILENT_DIRECT_ACCESS` key + register it |
| 2 | `src/rendezvous_mediator.rs:899` | pass `from_direct_ip = true` |
| 3 | `src/server.rs:190,263` (+ callers `:178,:337`) | add/forward `from_direct_ip` |
| 4 | `src/server/connection.rs:311,~427/467,~505,5280‑5343` | store flag; select `--cm-no-ui` when silent |
| 5 | `src/tray.rs:11,141` | early‑return when option on |
| 6 | `flutter/lib/consts.dart` | `kOptionAllowSilentDirectAccess` |
| 7 | `flutter/lib/desktop/pages/desktop_setting_page.dart` (Security tab) | `_OptionCheckBox` |
| 8 | `src/lang/*.rs` | label + tip strings |
| 9 | `src/core_main.rs` | `--silent` / `--no-silent` flags |
| 10| Sciter UI *(optional)* `src/ui/cm.*`, `src/ui_interface.rs` | parity for legacy UI builds |

## 5. Testing

- **Unit/build:** `cargo check` after the Rust changes; confirm the submodule
  bump compiles.
- **Direct‑IP silent path:**
  - Enable `direct-server` + set a permanent password; turn on the new option.
  - Connect from another LAN host via `IP:21118`. Expect: no CM window, no tray
    icon, connection authorized, input/file transfer work, disconnect is silent.
- **Non‑silent regression:** with the option **off**, the CM window and tray must
  appear exactly as before for both direct‑IP and ID connections.
- **Scope check:** with the option **on**, an **ID/relay** connection should
  still show the CM window (only direct‑IP is silenced) — verifies the
  `from_direct_ip` plumbing. *(See §6 caveat for mixed concurrent sessions.)*
- **CLI:** `--option allow-silent-direct-access Y`, `--silent`, `--no-silent`
  round‑trip via `--option <key>` read‑back.
- **Override/fixed:** put the key in `OVERWRITE_SETTINGS` (custom client) and
  confirm the Settings toggle renders disabled (`isOptionFixed`).
- **Platforms:** verify tray suppression on Windows, Linux, macOS (macOS keeps
  the event loop but no icon — mirror existing `OPTION_HIDE_TRAY` handling).

## 6. Known caveat — mixed concurrent connections

The CM is **one process** shared by all active sessions, and its UI/no‑UI mode is
fixed by whichever connection **spawns** it first (research §1.2). Therefore:

- If a **direct‑IP** connection spawns the CM in `--cm-no-ui` and an **ID**
  connection arrives while it is alive, the ID session will *also* be windowless
  until the CM exits.
- Conversely, an ID connection that spawned a visible CM will show a window even
  for a later direct‑IP session.

For the stated use‑case (a box reached *only* via local direct IP) this is
acceptable. If true per‑session UI isolation is required it is a larger change
(per‑connection CM windows) and should be a separate effort.

## 7. Optional refinements / future work

- **Split into two options** (`allow-silent-direct-access` for the CM,
  `hide-tray`/a user‑level tray toggle) if independent control is desired.
- **LAN‑only guard:** additionally require the peer IP to be
  private/link‑local before silencing (combine `from_direct_ip` with an
  `ip.is_private()`‑style check; note the historical removal of an
  `is_private()` check at `src/lan.rs:178`).
- **Sciter parity:** wire the same option into `src/ui/cm.rs:165` /
  `src/ui.rs:121` so non‑Flutter builds behave identically.
- **Live tray refresh:** signal a running `--tray` process to re‑evaluate the
  option without a restart.
- **Audit/log:** keep `post_conn_audit` (`connection.rs:1337`) intact so silent
  sessions are still recorded server‑side even though nothing is shown locally.
