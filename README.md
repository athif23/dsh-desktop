# dsh-desktop

Native desktop shell for upstream DSH (Tauri v2, Windows): frameless
window with a custom title bar, a real-browser surface over the Browser
panel, and a self-updating packaged backend. No DSH fork, no DSH
frontend source here — the shell hosts `dsh web` and extends it through
the out-of-tree [`dsh-browser`](https://github.com/athif23/dsh-browser)
plugin.

## Install & first run

Two downloads from [Releases](https://github.com/athif23/dsh-desktop/releases):

- **`*_x64-setup.exe`** — installer, per-user, no admin.
- **`*_portable_x64.zip`** — extract anywhere, run the exe, install
  nothing. Shell-owned state goes to a `data\` folder beside the exe
  (backend, settings, logs, WebView2 profile), so the folder is the whole
  app and deleting it resets everything. The zip's `portable.txt` marker
  is the switch; the exe inside is byte-identical to the installer's.

Either way: Git, Node.js 22+, and pnpm on PATH (first-run provisioning
and backend updates shell out to them) plus the WebView2 runtime. First
launch shows a choice card:

- **Install bundled dsh** (recommended) — clones upstream, installs,
  builds. Later updates arrive via Settings → Upgrade dsh.
- **Use my own dsh** — your checkout, your `git pull`, your builds. The
  shell uses it, checks it can boot, and otherwise never touches it.

Settings → **Backend…** reopens the card anytime.

Choosing an option collapses the card into a progress view: the full step
checklist (clone → launcher → dependencies → build) with done / running /
pending marks, a live log (auto-opens, collapsible via Show details), and
Cancel, which stops the running step and lets you re-pick — residue wipes
itself on retry. Picking **Install bundled dsh** when a working packaged
backend already exists switches to it instead of re-cloning. An unfinished
packaged tree or a broken custom dir also lands here with the exact reason
instead of a dead exit.

Portable and installed copies share the loopback control port, so run one
at a time.

## How it works

One native window, three WebViews, explicit bounds everywhere (no
auto-resize — one layout function owns every pixel, including under
display scaling):

```text
Tauri native window "main"
├── "shellbar"  custom title bar (shell-owned page)
├── "dsh"       official `dsh web` UI, untouched
└── "browser"   real page, hidden until the Browser tab reports a rect
```

The Browser tab (`dsh-browser` client) POSTs its canvas rect
(`{visible,x,y,width,height,dpr}`) to the shell; Rust moves/shows the
overlay under the panel toolbar, or hides it. Resize, reflow, and tab
switches are all covered by observer + tick + unmount reports — no
hardcoded coordinates, and stale rects are clamped so they can neither
cover the toolbar nor spill past the window edge.

The same WebView2 is agent-drivable through the loopback control plane
(`http://127.0.0.1:45331`, same channel the bar and menus use):
`POST /navigate {url}`, `POST /eval {js}`, `POST /cdp {method, params}`,
`GET /state`, `POST /show|/hide`, `POST /reveal-tab`, window controls,
restart/reload, settings, backend upgrade. CDP goes through
`CallDevToolsProtocolMethod` on the platform WebView — no
remote-debugging port, no second browser process.

Backend resolution order (first hit wins, logged at boot): `DSH_BIN`
env (dev) → settings custom dir (launched as
`node --import tsx/esm apps/cli/src/bin.ts` with cwd) → packaged
backend (`<state root>\dsh-src`: `%LOCALAPPDATA%\dsh-desktop` installed,
or `data\` beside the exe in portable mode) → sibling `dsh.cmd` →
`dsh` on PATH → first-run setup.

## Packaged backend + Upgrade dsh

The provisioned backend is a clone of **upstream**
(`https://github.com/deepseek-ai/deepseek-harness.git`, `master`) plus
three generated files (all in the clone's `.git/info/exclude`, so merges
never touch them): `dsh-manifest.json` (`{remote, branch, commit}`),
the launcher, and the detached `updater.cmd`.

Backend rule: the packaged clone is pristine upstream, clone-only, never
edited. All behavior change ships as out-of-tree plugins. Fork only on
purpose, temporarily, when something is impossible through extension
points.

The clone must be installed **and built**: per-user profile fallbacks
link into workspace `lib/` output, so an unbuilt tree fails with
`ERR_MODULE_NOT_FOUND`. The shell pre-flights (`node_modules` +
`apps/cli/lib/bin.js`) and refuses with the exact fix instead of
spawning into that failure; the provisioner and updater both build.

Settings → **Upgrade dsh**: check fetches the tracked branch (safe
while running) and reports up-to-date / behind (N commits) / diverged /
dev-mode. Behind + confirm spawns the updater detached, kills the
child, and exits; the updater fast-forward-merges, installs, builds
(`reset --hard` rollback on install/build failure), relaunches the
shell, and leaves a status file the next boot toasts once. Refused
outside the packaged backend — custom checkouts update with plain git.

Browser/backend skew: at boot the shell probes `--version` and compares
against the staged dsh-browser copy's `dsh.engines.backend` range.
Match → `Backend: … ✓` in Settings, silent. Mismatch → the label names
the gap plus one warning toast (`Browser tab unavailable… Chat is
unaffected`); the app runs on without the tab. Warn-only, always —
for packaged and custom alike.

The shell pins its child's frame server (`DSH_BROWSER_FRAME_PORT`,
default 9453) and answers panel discovery with identity, so every
Browser tab uses its own frame server and foreign rect reports are
acked but never steer the overlay.

Downloads: WebView2 doesn't save files on its own, so both views carry
a download hook — session exports land in `%USERPROFILE%\Downloads` as
`dsh-session-<id>.zip` (upstream's convention), anything else keeps its
URL last segment, and each finish pops a small auto-dismissing toast.

## Security model

The control plane is loopback-only with no auth: any process running as
your user can drive it (navigate, eval, restart, upgrade). That is the
entire trust boundary — same-user. It is not reachable from the
network, and app code treats it as local-only. The updater only ever
runs inside the packaged dir; it never touches a custom checkout (not
fetch, not merge, never `reset --hard`).

## Development

```sh
cargo build            # debug binary -> target/debug/dsh-desktop.exe
$env:DSH_SRC = "<your dsh checkout>"
$env:DSH_BIN = "<this-repo>/dsh.cmd"   # dev shim, requires DSH_SRC
./target/debug/dsh-desktop
```

Layout: `src/main.rs` (shell), `src/platform.rs` (every OS assumption —
the macOS/Linux port touches this file plus script twins),
`packaging/` (reproducible backend provisioning: install script, updater
template, porting notes), `experiment/` (throwaway drive scripts, not
part of the product), `dsh.cmd` (dev-only shim, never shipped).
Debug builds log to a console; release builds are GUI-subsystem (no
console — all tool spawns equally suppress it).

Releases: tag `vX.Y.Z` (matches `tauri.conf.json`) → CI builds the
NSIS installer on a Windows runner → published GitHub release with the
shell/backend/browser triplet in the notes. Unsigned builds (SmartScreen
click-through) until signing is set up.
