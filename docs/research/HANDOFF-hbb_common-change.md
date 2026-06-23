# Handoff: applying the hbb_common submodule change in a new session

The `allow-silent-direct-access` feature needs one change in the **hbb_common
submodule** (`teggen/hbb_common`). That change is committed locally in this
session but **could not be pushed** because the session was scoped to
`teggen/rustdesk` only. The container is ephemeral, so the local submodule
commit will be lost — this patch is the durable copy.

- Patch file: [`hbb_common-silent-direct-access.patch`](./hbb_common-silent-direct-access.patch)
- Original (lost) submodule commit SHA: `c69ef691370d24b8e19e10ac6b11b703adf831ef`
- Parent-repo branch (already pushed): `claude/pensive-einstein-m38yn4`
  - `.gitmodules` already points `libs/hbb_common` at `https://github.com/teggen/hbb_common`.
  - The submodule **pointer is intentionally NOT bumped yet** (still the upstream
    pinned commit `e50ac3c…`), because the new commit isn't on the remote.

## Steps in the new session (with `teggen/hbb_common` access enabled)

```bash
cd <repo>                       # the rustdesk checkout
git checkout claude/pensive-einstein-m38yn4
git pull origin claude/pensive-einstein-m38yn4

# 1. Make sure the submodule is checked out from the fork
git submodule sync libs/hbb_common
git submodule update --init libs/hbb_common

# 2. Apply the saved change inside the submodule
cd libs/hbb_common
git checkout -b claude/pensive-einstein-m38yn4
git config user.email noreply@anthropic.com
git config user.name Claude
git am ../../docs/research/hbb_common-silent-direct-access.patch
#   (if `git am` is fussy, use:
#    git apply ../../docs/research/hbb_common-silent-direct-access.patch && \
#    git commit -am "config: add OPTION_ALLOW_SILENT_DIRECT_ACCESS key")

# 3. Push the submodule branch to the fork
git push -u origin claude/pensive-einstein-m38yn4
NEW_SHA=$(git rev-parse HEAD)
cd ../..

# 4. Bump the parent submodule pointer to the new fork commit and push
git add libs/hbb_common
git commit -m "build: bump hbb_common to OPTION_ALLOW_SILENT_DIRECT_ACCESS commit"
git push origin claude/pensive-einstein-m38yn4
```

## Then verify the build

The previous session could **not** compile (the full build fetches a dependency
from `chromium.googlesource.com`, blocked by the network policy). Once the
submodule is wired up and network allows, run:

```bash
cargo check          # confirms OPTION_ALLOW_SILENT_DIRECT_ACCESS resolves
```

## What the parent branch already contains

All non-submodule work is already committed/pushed on
`claude/pensive-einstein-m38yn4` (commit `8412024`):

- `src/server/connection.rs`, `src/server.rs`, `src/rendezvous_mediator.rs` —
  thread `from_direct_ip` and launch `--cm-no-ui` for silent direct sessions.
- `src/tray.rs` — hide tray when the option is on.
- `src/core_main.rs` — `--silent` / `--no-silent` CLI flags.
- `flutter/lib/consts.dart`, `flutter/lib/desktop/pages/desktop_setting_page.dart`
  — Security-tab toggle + tooltip.
- `src/lang/*.rs` — `silent_direct_access_tip` + label strings.

After step 4 the branch should be complete and buildable.
