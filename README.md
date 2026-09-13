# dsh-desktop

Boring Tauri v2 shell for upstream DSH — currently carrying the
**multi-WebView native-browser feel experiment** (see below).

Hard rules (from the project brief):

- No copy of DSH frontend source here; no DSH fork; no session APIs in Rust.
- The CDP browser implementation stays in `dsh-browser` (Node). No Rust rewrite
  without a concrete reason; no second WebView for the browser pane.
- The same `dsh-browser` plugin must work in plain `dsh web` and under this shell.
- No Electron, no GPUI. Backend self-update (Upgrade dsh) shipped; shell
  installer + first-run provisioning in scope (see `packaging/`).

## Run the experiment shell

```sh
cd <this-repo>
cargo build            # debug binary -> target/debug/dsh-desktop.exe
$env:DSH_SRC = "<your dsh checkout>"
$env:DSH_BIN = "<this-repo>/dsh.cmd"
./target/debug/dsh-desktop
```

`dsh.cmd` is a dev-only shim launching `dsh web` from the live checkout in
`DSH_SRC` on a free loopback port. Normal use provisions the packaged
backend instead (see `packaging/`). The shell prints the DSH URL,
creates one native window with **two WebViews** (`dsh` = official UI filling
the window, `browser` = real page, hidden), and serves the loopback control
plane at `http://127.0.0.1:45331`.

Backend resolution order (first hit wins, logged at boot as
`dsh backend: <bin> (<source>)`): `DSH_BIN` env (dev against a live
checkout) → settings custom dir (user's own checkout, launched as
`node --import tsx/esm apps/cli/src/bin.ts` with cwd) → installer-packaged
backend (`%LOCALAPPDATA%\dsh-desktop\dsh-src\dsh-packaged.cmd`) → sibling
`dsh.cmd` next to the exe → `dsh` on PATH → nothing (first-run setup).

## First-run setup

With no usable backend (and no `DSH_BIN`) the shell boots shell-owned:
the shellbar view goes full-window with a choice card (no DSH page
exists yet). **Install bundled** provisions upstream (clone → manifest +
launcher + updater → `pnpm install` → `pnpm run build`, all with polled
progress and a heartbeat; a failed attempt wipes itself so retry heals;
success relaunches into normal boot). **Use my own** takes a folder,
validates installed + built, saves it to settings, and relaunches onto
it. An unfinished packaged tree or a broken custom dir also lands here
with the exact reason instead of a dead exit. Settings → **Backend…**
reopens the card anytime (Cancel returns when a backend exists).

## Packaged backend + Upgrade dsh

The installer lays down a clone of **upstream**
(`https://github.com/deepseek-ai/deepseek-harness.git`, `master`) plus
three generated files (all in the clone's `.git/info/exclude`, so merges
never touch them):

- `dsh-manifest.json` — `{remote, branch, commit}` recorded at install.
- `dsh-packaged.cmd` — `node --import tsx/esm apps/cli/src/bin.ts` in that dir.
- `updater.cmd` — the detached updater (below).

Backend rule: the packaged clone is pristine upstream, clone-only, never
edited (`git status` there shows only the three generated files; the
updater's ff-only merge refuses anything else). All behavior change ships
as out-of-tree plugins (the `dsh-browser` template: own dir +
`cordis.patch.yml`, loaded via profile into any backend). Fork only on
purpose, temporarily, when something is impossible through extension
points.

The clone must be `pnpm install`ed **and built**: the per-user
`$DSH_HOME/profiles/node_modules` fallback links at each boot to whichever
checkout launched, and those links resolve workspace `lib/` output — a fresh
clone with no `lib/` fails boot with `ERR_MODULE_NOT_FOUND` (measured).
Same reason the updater runs `pnpm run build` after every merge. The shell
pre-flights the packaged dir (`node_modules` + `apps/cli/lib/bin.js`) and
refuses with the exact fix instead of spawning into that failure.

Settings → **Upgrade dsh**: `POST /dsh-check` fetches the tracked branch
(fetch only, safe while the child runs) and compares — up-to-date, behind
(N commits), diverged, or dev-checkout-mode each get their own menu flash.
Behind + native confirm → `POST /dsh-upgrade` spawns `updater.cmd`
detached, kills the child, and exits. The updater fast-forward-merges,
installs, builds (rolling back with `reset --hard` on install/build
failure), relaunches the shell exe, and leaves `update-status.json`; the
next boot toasts the verdict once and consumes the file. Refused outside
the packaged backend — a dev checkout updates with plain git.

Browser/backend skew: at boot the shell probes the running backend's
`--version` and compares it against the staged dsh-browser copy's
`package.json` `dsh.engines.backend` range (`>=0.1.5-rc.1` — the release
that introduced the tab system). Match → `Backend: packaged · <v> ✓` in
the Settings menu, silent. Mismatch → the same label names the gap plus
one warning toast (`Browser tab unavailable… Chat is unaffected`), and
the app runs on without the tab. Custom (non-packaged) backends are
self-managed: labeled with their source, never judged, never touched —
the updater refuses anything but the packaged dir.

The shell pins its child's frame server (`DSH_BROWSER_FRAME_PORT`, default
9453) and answers panel discovery at `GET /panel-config?dshPort=` with
identity, so every Browser tab uses its own frame server and rect reports
from foreign windows are ignored (never steer the overlay). Plain `dsh web`
needs no shell restart for any of this work.

WebView2 does not save downloads on its own, so both WebViews carry an
`on_download` hook: session exports (`/api/session.export?sessionId=…`)
land in `%USERPROFILE%\Downloads` as `dsh-session-<id>.zip` (the upstream
filename convention); anything else keeps its URL last segment. Without
this, the export modal reports success (its HEAD check passed) while no
file ever arrives. Each finished download also pops a small bottom-right
toast window naming the file and its folder (pure-JS Close button, ~9s
auto-dismiss, no new dependencies — content renders from a temp HTML file,
which is race-free on cold WebView2 start where a post-load eval showed an
empty frame).

## Experiment: native WebView2 over the Browser panel

Goal: answer whether Tauri v2 can host the DSH UI in one WebView and a real
browser in a second WebView positioned over the Browser side-panel region,
with the agent controlling that exact WebView2 through CDP — and whether it
feels like Codex/Zcode.

```text
Tauri native window "main"
├── WebView "dsh"      official `dsh web` (existing UI + Browser tab shell)
└── WebView "browser"  real remote webpage (native scroll/select/type/video)
```

- Second WebView: `WebviewBuilder::new("browser", …)` + `window.add_child(…)`
  (needs the `unstable` feature), hidden until the Browser tab reports a rect.
- Bounds sync: `dsh-browser`'s tab (`lib/client.js`, isolated `nativeRectBridge`
  effect) POSTs `{visible,x,y,width,height,dpr}` of its canvas to
  `POST /rect`. Rust applies `set_bounds` + `show`, or `hide` when the tab is
  hidden/switched away. No hardcoded coordinates; ResizeObserver +
  IntersectionObserver + window resize + 500ms tick + unmount `visible:false`.
- Underlying WebView2: `webview.with_webview(|platform| …)` on Windows gives
  `platform.controller().CoreWebView2()`; CDP goes through
  `CallDevToolsProtocolMethod` (pattern copied from the `tinybot` prior art).
  No remote-debugging port, no second browser process — one target by
  construction.
- Agent control = loopback HTTP (usable from any script, including the DSH
  agent via shell): `POST /navigate {url}`, `POST /eval {js}`,
  `POST /cdp {method, params}`, `GET /state`, `POST /show|/hide`,
  `GET /health`, `GET /rect`. Scripted drive:
  `pwsh -File experiment/test-drive.ps1`.

### Machine-proven (2026-09-12, this box)

- `cargo check` + `cargo build` green (fixes needed: placeholder
  `icons/icon.ico`, `frontendDist: "./dist-stub"`, shim must not duplicate
  the `web` argv).
- Window boots with both WebViews, no deadlock; DSH serves, control plane
  answers.
- Agent `navigate` → `example.com`, `github.com`; `eval document.title`
  reads back correctly; raw CDP `Runtime.evaluate(location.href)` returns
  the same page the (hidden-then-shown) WebView displays.
- CDP `Page.captureScreenshot` renders sharp vector text
  (`experiment/shot.jpg`); scripted DOM scroll/link queries work.
- Finding: `Page.captureScreenshot` on a **hidden** WebView hangs past the
  10s timeout; showing first fixes it (same compositor lesson as the
  headless/occluded screencast work — budget a `tinybot`-style off-screen
  `show` for background captures).

### Needs human eyes/hands (mostly done 2026-09-12)

- ✅ Native feel confirmed by user ("much better, much more responsive").
- ✅ Picker: magenta hover outline + click returned a full ElementReference.
- ✅ Resize: user dragged 667→391px; overlay, screenshot (489x1015 =
  391×1.25 × 812×1.25 exact), and clicks tracked. Narrow-width caveat:
  GitHub collapses nav into "More", hiding ref targets (see dsh-browser
  README).
- ⏳ Open (judgment calls): YouTube video playback smoothness, long
  text-selection feel. Page loads + state sync verified by machine.

## Decision (2026-09-12)

```text
Tauri multi-Webview feels native and agent control works
→ continue with Tauri
```

Phase-1 agent loop (observation/action on the exact same WebView2) is
proven end-to-end through the shipped `dsh-browser` tools; evidence and
per-finding notes live in `experiment/` (`live-loop.mjs`,
`tool-level.mjs`, `visible-path.mjs`, `form.html`, shots) and the
`dsh-browser` README. No Rust changes were needed beyond the experiment
scaffold — the `/navigate|/eval|/cdp|/state` plane proved sufficient.

Report with the decision: files changed, bounds-sync path, show/hide path,
WebView2/CDP access path, coordinate/DPI issues, focus/input problems,
video behavior, resize feel, and whether it feels materially closer to
Codex than the screencast canvas.

## What this does NOT do (by design)

Same plugin, both surfaces: `dsh-browser` stays a Node DSH plugin and works
unchanged under this shell — the shell never sees CDP sessions or tools, only
one panel rectangle and raw page control. The streamed canvas implementation
is untouched and remains the fallback under plain `dsh web`.
