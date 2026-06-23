# Per-connection silent-mode override & close prevention

Companion to [`silent-direct-access-implementation-plan.md`](./silent-direct-access-implementation-plan.md).
This covers the two client-signalled controls layered on top of silent mode.

## Requirements

1. **Override silent mode per connection.** When connecting, the client signals
   the controlled computer to override silent mode: show the connection-manager
   (CM) dialog as usual and show the tray icon while the connection is active.
   Hide the tray icon again when the session ends if the connection config says so.
2. **Prevent server-side close.** The client signals that closing must be
   prevented, so the close/disconnect actions in the CM are disabled (and the
   server ignores CM-initiated close for that connection).

## Protocol change (hbb_common submodule)

`LoginRequest` (`libs/hbb_common/protos/message.proto`) gains three booleans the
client sets at login:

```proto
bool override_silent_mode = 18;
bool auto_hide_tray       = 19;
bool prevent_close        = 20;
```

These are carried in the existing login handshake — no new message type.

## Client side (controlling peer)

`src/client.rs` (`LoginConfigHandler`, where `LoginRequest` is built) reads three
per-peer **connection-config** options and sets the fields:

| Proto field           | Peer option key          |
|-----------------------|--------------------------|
| `override_silent_mode`| `override-silent-mode`   |
| `auto_hide_tray`      | `auto-hide-tray`         |
| `prevent_close`       | `prevent-server-close`   |

Keys are also defined in `flutter/lib/consts.dart`. A pre-connect UI toggle to
set them is the remaining follow-up (the plumbing/sending is in place).

## Controlled side (server)

### CM window (override)

The CM-launch decision in `start_ipc` (`src/server/connection.rs`) already chose
`--cm-no-ui` for silent direct sessions. It now also checks the login signal:

```
silent_direct_cm = from_direct_ip && !override_silent_mode && <allow-silent-direct-access>
```

`override_silent_mode` is read from `self.lr` at `try_start_cm_ipc` time (login
has already been stored) and threaded into `start_ipc`. So an overriding
connection gets the normal `--cm` window.

### Tray (override + auto-hide)

The tray is a separate, global process, so per-connection visibility is driven by
a live server signal:

- Server globals (`connection.rs`): `TRAY_OVERRIDE_ACTIVE` (count of active
  override sessions) and `TRAY_STICKY_SHOW` (sticky flag).
  - On authorize, if `override_silent_mode`: `tray_force_show_add()`.
  - On close: `tray_force_show_remove(auto_hide_tray)` — decrements the count and,
    if `auto_hide_tray` is **false**, sets the sticky flag so the icon stays
    visible until the service restarts.
  - `tray_should_force_show()` = `count > 0 || sticky`.
- Exposed over IPC: `ipc::get_config("force_show_tray")` returns `"Y"/"N"`
  (`src/ipc.rs`).
- Tray process (`src/tray.rs`): a background thread polls `force_show_tray` once a
  second; the event loop toggles the icon with `TrayIcon::set_visible(show)` where
  `show = !hard_hidden && (!silent_hidden || force_show)`. In silent mode the tray
  process now keeps its event loop running (icon hidden) so it can appear on
  demand; only the admin `hide-tray` builtin suppresses the process entirely.

### Close prevention

- `prevent_close` is propagated to the CM via `ipc::Data::Login` →
  `Client.prevent_close` (`src/ui_cm_interface.rs`) → serialized to Flutter
  (`Client.preventClose`).
- Flutter CM (`server_page.dart`) hides the **Disconnect** button and shows a
  "closing disabled by peer" note when `preventClose`.
- Server backstop: the `ipc::Data::Close` handlers (remote and port-forward loops
  in `connection.rs`) ignore CM-initiated close when `self.lr.prevent_close` is
  set. Peer-initiated disconnect is unaffected — only the *server* side is blocked.

## Notes & limitations

- Silent mode still pairs with a permanent password; not compatible with
  click-to-accept (no UI to approve) for the silent (non-override) case.
- Mixed concurrent sessions share one CM process; the first session's connection
  decides UI vs no-UI (documented in the silent-mode plan).
- `TRAY_STICKY_SHOW` does not reset until the service restarts (matches "auto hide
  only if the config says so").
- `prevent_close` only disables the controlled side; the controlling client can
  always disconnect.
