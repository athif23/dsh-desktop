//! dsh-desktop: boring Tauri v2 shell around upstream `dsh web`.
//!
//! EXPERIMENT (multi-WebView native browser feel):
//! ```text
//! Tauri native window "main"
//! ├── WebView "dsh"      official `dsh web` (existing DSH UI, untouched)
//! └── WebView "browser"  real remote webpage (WebView2, native feel)
//! ```
//! The DSH Browser plugin reports its panel rectangle to the loopback
//! control server below; Rust positions/sizes/shows/hides the "browser"
//! WebView over that rectangle. The agent drives the *same* WebView2
//! instance through `with_webview` + `CallDevToolsProtocolMethod`
//! (proven pattern from the `tinybot` prior art), exposed here as tiny
//! HTTP endpoints so any script — including the DSH agent via bash/curl —
//! can navigate/eval/CDP the exact page the human sees.
//!
//! Nothing here forks DSH: the DSH web UI, its side panel, and the
//! streamed `dsh-browser` canvas implementation are all untouched. This
//! is an additional surface backend for the feel experiment only.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use tauri::{
    webview::{DownloadEvent, WebviewBuilder},
    LogicalPosition, LogicalSize, Manager, Rect, Webview, WebviewUrl, Wry,
};
use tauri_plugin_dialog::DialogExt;
use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
use windows::core::HSTRING;

mod platform;

/// First `dsh web:` startup line wins; the URL carries the process token.
const STARTUP_PREFIX: &str = "dsh web: ";
/// How long to wait for the child to print its URL before killing it.
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// Fixed loopback control plane for the experiment (documented in README).
/// Chosen far from the dsh-browser frame range (9333..9342) on purpose.
const CONTROL_ADDR: &str = "127.0.0.1:45331";
/// Shell toolbar strip height (logical px) reserved at the top of the
/// native window, under the OS title bar. The DSH page and the browser
/// overlay both start below it.
const SHELLBAR_H: f64 = 32.0;
/// Pinned frame-server port for the shell's own DSH child (mirrors the
/// plugin's DSH_BROWSER_FRAME_PORT pin): the panel discovers this instead
/// of racing :9333 with every other `dsh web` on the box.
const DEFAULT_SHELL_FRAME_PORT: u16 = 9453;

fn shell_frame_port() -> u16 {
    std::env::var("DSH_BROWSER_FRAME_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| *p > 0)
        .unwrap_or(DEFAULT_SHELL_FRAME_PORT)
}

struct DshChild(Mutex<Option<Child>>);

/// Last panel rectangle reported by the DSH Browser tab (CSS px == DIP).
#[derive(Clone, Copy, Debug, Default)]
struct PanelRect {
    visible: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    dpr: f64,
}

#[derive(Debug, Deserialize)]
struct RectReport {
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    height: f64,
    #[serde(default)]
    dpr: Option<f64>,
    /// Reporting DSH's own port (location.port). Reports from any other DSH
    /// (e.g. a plain `dsh web` beside the shell, or an older panel without
    /// the field) are acknowledged but never applied — last-writer-wins
    /// across windows would otherwise steer the overlay from the wrong tab.
    #[serde(rename = "dshPort", default)]
    dsh_port: u16,
}

#[derive(Debug, Deserialize)]
struct NavigateBody {
    #[serde(default)]
    url: String,
}

#[derive(Debug, Deserialize)]
struct EvalBody {
    #[serde(default)]
    js: String,
}

#[derive(Debug, Deserialize)]
struct CdpBody {
    #[serde(default)]
    method: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

/// `POST /reveal` body: a file or folder the toast's "Show in folder"
/// button asks Explorer to open. Constrained to the Downloads dir below.
#[derive(Debug, Deserialize)]
struct RevealBody {
    #[serde(default)]
    path: String,
}

/// `POST /settings` body: the shell-owned knobs. Today only the download
/// folder; unknown fields are ignored so the card can grow freely.
#[derive(Debug, Deserialize)]
struct SettingsBody {
    #[serde(default, rename = "downloadDir")]
    download_dir: String,
}

/// `POST /window` body: one native window action for the frameless title
/// bar (`minimize`, `toggle-max`, `close`). Dragging has its own route.
#[derive(Debug, Deserialize)]
struct WindowBody {
    #[serde(default)]
    action: String,
}

/// `POST /setup-start` body: `bundled` provisions from upstream, `custom`
/// adopts the user's checkout at `path` (validated before saving).
#[derive(Debug, Deserialize)]
struct SetupBody {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    path: Option<String>,
}

/// Pick an explicit loopback port so two shells (or a manual `dsh web`) never
/// fight over the default. Binds :0, reads the port back, releases it.
fn free_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Home of the installer-packaged dsh backend: a clone of the tracked fork
/// plus `dsh-manifest.json` (remote/branch/commit) and the generated
/// launcher. Per-user dir (see `platform`) so backend updates never need
/// elevation.
fn packaged_dir() -> Option<std::path::PathBuf> {
    platform::packaged_dir()
}

/// The install-time record of which fork/branch the packaged backend tracks.
/// Informational today (Phase 2's updater compares and merges against it).
#[derive(Debug, Deserialize)]
struct PackagedManifest {
    #[serde(default)]
    remote: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    commit: String,
}

fn packaged_manifest() -> Option<PackagedManifest> {
    let dir = packaged_dir()?;
    let text = std::fs::read_to_string(dir.join("dsh-manifest.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Run git in a directory, returning trimmed stdout or trimmed stderr.
fn git_out(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git unavailable: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn short_sha(full: &str) -> String {
    full.chars().take(7).collect()
}

/// Check the packaged backend against its tracked fork branch: fetch (touches
/// only remote-tracking refs, safe while the child runs from that dir) and
/// compare. Never merges, never closes anything.
/// The manifest records the remote URL; git commands want the remote *name*.
/// Resolve it through the clone's own remotes (normally `origin`).
fn tracked_remote_name(dir: &std::path::Path, url: &str) -> Option<String> {
    let remotes = git_out(dir, &["remote", "-v"]).ok()?;
    for line in remotes.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(name), Some(remote_url)) = (parts.next(), parts.next()) {
            if remote_url == url {
                return Some(name.to_string());
            }
        }
    }
    None
}

fn check_backend_update() -> serde_json::Value {
    let (_, source) = dsh_bin();
    if source != "packaged backend" {
        return serde_json::json!({"status": "devmode", "source": source});
    }
    let Some(dir) = packaged_dir() else {
        return serde_json::json!({"status": "error", "message": "no packaged backend dir"});
    };
    let Some(manifest) = packaged_manifest() else {
        return serde_json::json!({"status": "error", "message": "packaged backend has no manifest"});
    };
    if manifest.remote.is_empty() || manifest.branch.is_empty() {
        return serde_json::json!({"status": "error", "message": "packaged backend manifest is incomplete"});
    }
    // The manifest records the remote URL; git wants the remote *name*.
    // Resolve it through the clone's own remotes (normally `origin`).
    let Some(remote_name) = tracked_remote_name(&dir, &manifest.remote) else {
        return serde_json::json!({"status": "error", "message": "tracked fork is not a remote of the packaged clone"});
    };
    let local = match git_out(&dir, &["rev-parse", "HEAD"]) {
        Ok(sha) => sha,
        Err(detail) => return serde_json::json!({"status": "error", "message": format!("local HEAD unreadable: {detail}")}),
    };
    if let Err(detail) = git_out(&dir, &["fetch", &manifest.remote, &manifest.branch]) {
        return serde_json::json!({"status": "error", "message": format!("could not reach the fork: {detail}")});
    }
    let remote_ref = format!("{remote_name}/{}", manifest.branch);
    let remote = match git_out(&dir, &["rev-parse", &remote_ref]) {
        Ok(sha) => sha,
        Err(detail) => return serde_json::json!({"status": "error", "message": format!("remote branch unreadable: {detail}")}),
    };
    if local == remote {
        return serde_json::json!({"status": "uptodate", "local": short_sha(&local)});
    }
    // Remote already inside our history means local commits on top: diverged,
    // the updater's ff-only merge would refuse, so say so now.
    let ancestor = std::process::Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["merge-base", "--is-ancestor", &remote, "HEAD"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ancestor {
        return serde_json::json!({"status": "diverged", "local": short_sha(&local), "remote": short_sha(&remote)});
    }
    let behind = git_out(&dir, &["rev-list", "--count", &format!("HEAD..{remote}")])
        .ok()
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(0);
    serde_json::json!({"status": "available", "local": short_sha(&local), "remote": short_sha(&remote), "behind": behind})
}

/// Where the detached updater leaves its result for the next boot to toast.
fn update_status_path() -> Option<std::path::PathBuf> {
    platform::update_status_path()
}

/// One-line result toast for the updater (same injected-DOM channel and
/// styling as the download toast, minus the folder button).
fn show_note_toast(app: &tauri::AppHandle, title: &str, body: &str, ok: bool) {
    let j_title = serde_json::to_string(title).unwrap_or_default();
    let j_body = serde_json::to_string(body).unwrap_or_default();
    let (bg, edge) = if ok { ("#1e1e1e", "#3a3a3a") } else { ("#2a1a1a", "#5a2a2a") };
    let js = format!(
        "(function(){{var o=document.getElementById('dsh-note-toast');if(o)o.remove();\
        var b=document.createElement('div');b.id='dsh-note-toast';\
        b.style.cssText='position:fixed;top:14px;left:50%;transform:translateX(-50%);z-index:2147483647;background:{bg};color:#eee;border:1px solid {edge};border-radius:10px;padding:12px 14px;width:360px;box-sizing:border-box;font-family:\"Segoe UI\",sans-serif;font-size:12px;box-shadow:0 8px 28px rgba(0,0,0,.5)';\
        var h=document.createElement('div');h.textContent={j_title};h.style.cssText='font-weight:600;font-size:13px;margin-bottom:4px';\
        var a=document.createElement('div');a.textContent={j_body};a.style.cssText='font-size:12px;word-break:break-all';\
        var r=document.createElement('div');r.style.cssText='text-align:right;margin-top:8px';\
        var c=document.createElement('button');c.textContent='Close';c.style.cssText='background:#2d2d2d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:3px 12px;font-size:12px;cursor:pointer';\
        c.onclick=function(){{b.remove()}};r.appendChild(c);b.appendChild(h);b.appendChild(a);b.appendChild(r);\
        document.body.appendChild(b);setTimeout(function(){{b.remove()}},9000);}})()",
    );
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(view) = handle.get_webview("dsh") {
            if view.eval(js.as_str()).is_err() {
                eprintln!("[browser] update toast eval failed");
            }
        }
    });
}

/// How to spawn the backend: an opaque launcher/bin, or a dsh checkout
/// dir launched as `node --import tsx/esm apps/cli/src/bin.ts` with cwd.
enum BackendSpawn {
    Bin(String),
    NodeDir(std::path::PathBuf),
}

fn dsh_bin() -> (Option<BackendSpawn>, &'static str) {
    // Explicit env wins (dev against a live checkout); then the settings
    // custom dir; otherwise the installer-packaged backend; otherwise a
    // `dsh.cmd` sitting next to the shell binary (the experiment checkout
    // layout) so double-clicking the exe works; then PATH; else nothing
    // (first-run setup mode).
    if let Ok(from_env) = std::env::var("DSH_BIN") {
        if !from_env.trim().is_empty() {
            return (Some(BackendSpawn::Bin(from_env)), "env DSH_BIN");
        }
    }
    if let Ok(slot) = shell_settings().lock() {
        if let Some(choice) = slot.backend.clone() {
            if choice.kind == "custom" {
                let dir = choice.path.unwrap_or_default();
                let dir = std::path::PathBuf::from(&dir);
                if !custom_dir_usable(&dir) {
                    return (None, "custom backend (missing)");
                }
                // Installed + built is checked here (cheap path probes) so
                // a checkout the user later breaks lands in setup with
                // guidance instead of a dead exit.
                if packaged_backend_ready(&dir, false).is_err() {
                    return (None, "custom backend (unusable)");
                }
                return (Some(BackendSpawn::NodeDir(dir)), "custom backend");
            }
        }
    }
    if let Some(dir) = packaged_dir() {
        let launcher = platform::packaged_launcher_file(&dir);
        if launcher.is_file() {
            // A launcher with an unready tree is a failed provision or a
            // half-removed backend: setup mode, not a dead exit — the card
            // offers wipe-and-retry.
            if packaged_backend_ready(&dir, true).is_err() {
                return (None, "packaged backend (unfinished)");
            }
            return (Some(BackendSpawn::Bin(launcher.to_string_lossy().into_owned())), "packaged backend");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let shim = platform::dev_shim_file(dir);
            if shim.is_file() {
                return (Some(BackendSpawn::Bin(shim.to_string_lossy().into_owned())), "sibling dsh.cmd");
            }
        }
    }
    if platform::find_on_path("dsh").is_some() {
        return (Some(BackendSpawn::Bin("dsh".to_string())), "PATH");
    }
    (None, "none")
}

/// A custom backend dir is usable when it looks like a dsh checkout with
/// the entry point present. Deep readiness (installed + built) is checked
/// at spawn; this gate only decides whether setup mode is needed.
fn custom_dir_usable(dir: &std::path::Path) -> bool {
    dir.join("apps").join("cli").join("src").join("bin.ts").is_file()
}

/// Pre-flight for a dsh checkout dir (packaged clone or custom): the
/// per-user profiles fallback links at boot into the checkout's workspace
/// `lib/` output, so a tree that was never `pnpm install`ed + built fails
/// with a confusing `ERR_MODULE_NOT_FOUND` deep in plugin load (measured).
/// Refuse early with the exact fix instead. `ours` selects the wording:
/// packaged (shell-owned, Upgrade fixes it) vs custom (user-owned).
fn packaged_backend_ready(dir: &std::path::Path, ours: bool) -> Result<(), String> {
    let (who, fix) = if ours {
        ("packaged backend", "run pnpm install inside it (or Upgrade dsh once running)")
    } else {
        ("custom backend", "run pnpm install inside it")
    };
    if !dir.join("node_modules").is_dir() {
        return Err(format!("{who} at {} has no node_modules: {fix}", dir.display()));
    }
    let (who, fix) = if ours {
        ("packaged backend", "run pnpm run build inside it (or Upgrade dsh once running)")
    } else {
        ("custom backend", "run pnpm run build inside it")
    };
    if !dir.join("apps").join("cli").join("lib").join("bin.js").is_file() {
        return Err(format!("{who} at {} was never built (workspace lib/ missing): {fix}", dir.display()));
    }
    Ok(())
}

/// Home of the staged dsh-browser copy the shell reads compatibility
/// metadata from (its `package.json` `dsh.engines.backend` range). Staged
/// beside the backend at provision time; refreshed on shell releases.
fn bundled_browser_dir() -> Option<std::path::PathBuf> {
    platform::bundled_browser_dir()
}

/// Last browser/backend compatibility verdict, shown in the Settings menu.
/// Starts as "checking…" until the background probe finishes.
static COMPAT_LABEL: std::sync::LazyLock<std::sync::Mutex<String>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new("Backend: checking…".to_string()));

fn compat_label() -> String {
    COMPAT_LABEL.lock().map(|g| g.clone()).unwrap_or_default()
}

fn parse_semver(s: &str) -> Option<(u64, u64, u64, Option<String>)> {
    let s = s.trim().strip_prefix('v').unwrap_or(s.trim());
    let (nums, pre) = match s.split_once('-') {
        Some((n, p)) => (n, Some(p.to_string())),
        None => (s, None),
    };
    let mut it = nums.split('.');
    let parts = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if it.next().is_some() {
        return None;
    }
    Some((parts.0, parts.1, parts.2, pre))
}

fn cmp_prerelease(x: &str, y: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let mut xi = x.split('.');
    let mut yi = y.split('.');
    loop {
        match (xi.next(), yi.next()) {
            (None, None) => return Equal,
            (None, Some(_)) => return Less,
            (Some(_), None) => return Greater,
            (Some(a), Some(b)) => {
                let ord = match (a.parse::<u64>().ok(), b.parse::<u64>().ok()) {
                    (Some(m), Some(n)) => m.cmp(&n),
                    (Some(_), None) => Less,
                    (None, Some(_)) => Greater,
                    (None, None) => a.cmp(b),
                };
                if ord != Equal {
                    return ord;
                }
            }
        }
    }
}

fn cmp_semver(
    a: &(u64, u64, u64, Option<String>),
    b: &(u64, u64, u64, Option<String>),
) -> std::cmp::Ordering {
    (a.0, a.1, a.2)
        .cmp(&(b.0, b.1, b.2))
        .then_with(|| match (&a.3, &b.3) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(x), Some(y)) => cmp_prerelease(x, y),
        })
}

/// Requirement of the form `>=1.2.3-rc.1` against a probed version.
/// Anything unparseable fails closed to "no verdict" (None): without a
/// trustworthy comparison the shell stays quiet instead of warning wrongly.
fn requirement_satisfied(requirement: &str, version: &str) -> Option<bool> {
    let floor = requirement.trim().strip_prefix(">=")?;
    let (want, have) = (parse_semver(floor)?, parse_semver(version)?);
    Some(cmp_semver(&have, &want) != std::cmp::Ordering::Less)
}

/// Probe `<dir>`'s CLI for its version (`--version` prints it bare).
/// Runs on a background thread; a hung node only stalls the label.
fn backend_version(dir: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("node")
        .arg("--import")
        .arg("tsx/esm")
        .args(["apps/cli/src/bin.ts", "--version"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()?
        .trim()
        .trim_start_matches('v')
        .to_string();
    if parse_semver(&token).is_some() {
        Some(token)
    } else {
        None
    }
}

/// Compare the running backend against the staged dsh-browser's tested
/// range. Packaged and custom checkouts are both probed (warn-only on
/// mismatch); anything else is labeled self-managed and left alone.
/// Returns the menu label plus an optional (backend version, requirement)
/// mismatch pair. The updater still refuses everything but packaged.
fn check_browser_compat() -> (String, Option<(String, String)>) {
    let (_, source) = dsh_bin();
    let (kind, dir) = if source == "packaged backend" {
        ("packaged", packaged_dir())
    } else if source == "custom backend" {
        let dir = shell_settings()
            .lock()
            .ok()
            .and_then(|s| s.backend.clone())
            .and_then(|b| b.path)
            .map(std::path::PathBuf::from);
        ("custom", dir)
    } else {
        return (format!("Backend: {source} (self-managed)"), None);
    };
    let requirement = bundled_browser_dir()
        .and_then(|d| std::fs::read_to_string(d.join("package.json")).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.pointer("/dsh/engines/backend").and_then(|r| r.as_str()).map(str::to_string));
    let (Some(requirement), Some(dir)) = (requirement, dir) else {
        return (format!("Backend: {kind} · compatibility unknown"), None);
    };
    match backend_version(&dir) {
        Some(version) => match requirement_satisfied(&requirement, &version) {
            Some(true) => (format!("Backend: {kind} · {version} ✓"), None),
            Some(false) => (
                format!("Backend: {kind} · {version} — browser needs {requirement}"),
                Some((version, requirement)),
            ),
            None => (format!("Backend: {kind} · version unknown"), None),
        },
        None => (format!("Backend: {kind} · version unknown"), None),
    }
}

/// First-run setup, entered when no usable backend resolves and no
/// DSH_BIN is set. Shell-owned UI — no DSH page exists yet — so the
/// shellbar view goes full-window with the choice card below; the bundled
/// provision runs on a worker thread with polled progress; success
/// relaunches into normal boot.
const UPSTREAM_REMOTE: &str = "https://github.com/deepseek-ai/deepseek-harness.git";
const UPSTREAM_BRANCH: &str = "master";
const PACKAGED_LAUNCHER_TEXT: &str = "@echo off\r\nrem Generated at install: launch the packaged dsh backend (no checkout needed).\r\ncd /d %~dp0\r\nnode --import tsx/esm apps/cli/src/bin.ts %*\r\n";

fn setup_flag_path() -> Option<std::path::PathBuf> {
    platform::app_data_dir().map(|d| d.join("setup.pending"))
}

static SETUP_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn setup_mode() -> bool {
    SETUP_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Default)]
struct ProvisionState {
    phase: String,
    log: Vec<String>,
    done: bool,
    error: Option<String>,
}

static PROVISION: std::sync::LazyLock<Mutex<ProvisionState>> =
    std::sync::LazyLock::new(|| Mutex::new(ProvisionState::default()));

fn prov_log(line: String) {
    let line = line.chars().take(300).collect::<String>();
    eprintln!("[setup] {line}");
    if let Ok(mut s) = PROVISION.lock() {
        s.log.push(line);
        if s.log.len() > 60 {
            let drop = s.log.len() - 60;
            s.log.drain(..drop);
        }
    }
}

fn prov_phase(phase: &str) {
    prov_log(format!("—— {phase}"));
    if let Ok(mut s) = PROVISION.lock() {
        s.phase = phase.to_string();
    }
}

/// One provision step: run, keep the tail in the card log, fail loud.
fn run_step(dir: &std::path::Path, prog: &std::path::Path, args: &[&str], phase: &str) -> Result<(), String> {
    prov_phase(phase);
    let out = std::process::Command::new(prog)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not start {}: {e}", prog.display()))?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let tail: Vec<&str> = combined.lines().filter(|l| !l.trim().is_empty()).collect();
    for line in tail.iter().rev().take(4).rev() {
        prov_log(format!("| {line}"));
    }
    if !out.status.success() {
        return Err(format!("{} {} failed", prog.display(), args.join(" ")));
    }
    Ok(())
}

fn resolve_tool(name: &str, why: &str) -> Result<std::path::PathBuf, String> {
    // Prefer a directly executable image: PATH shims (extensionless Node
    // entry scripts, .ps1) are listed by `where` but CreateProcess
    // rejects them — measured with pnpm (os error 193).
    platform::find_on_path_all(name)
        .into_iter()
        .find(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(),
                Some("exe") | Some("cmd") | Some("bat") | Some("com")
            )
        })
        .ok_or_else(|| format!("no executable {name} on PATH ({why}) — install it, then retry"))
}

fn provision_bundled() {
    if PROVISION.lock().map(|s| !s.phase.is_empty() && !s.done).unwrap_or(false) {
        return;
    }
    std::thread::spawn(|| {
        let started = std::time::Instant::now();
        // Heartbeat: install/build are minute-scale silences otherwise.
        std::thread::spawn(move || {
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_secs(30));
                let finished = PROVISION.lock().map(|s| s.done).unwrap_or(true);
                if finished {
                    return;
                }
                prov_log(format!("… still working ({}s elapsed)", started.elapsed().as_secs()));
            }
        });
        let result = provision_bundled_inner();
        if let Ok(mut s) = PROVISION.lock() {
            s.done = true;
            match &result {
                Ok(sha) => s.phase = format!("Ready ({sha}) — relaunching…"),
                Err(e) => {
                    s.phase = "Failed".to_string();
                    s.error = Some(e.clone());
                }
            }
        }
        if result.is_ok() {
            std::thread::sleep(std::time::Duration::from_millis(800));
            relaunch_self();
        }
    });
}

fn provision_bundled_inner() -> Result<String, String> {
    let git = resolve_tool("git", "Git")?;
    let node = resolve_tool("node", "Node.js 22+")?;
    let pnpm = resolve_tool("pnpm", "pnpm")?;
    let dest = packaged_dir().ok_or("cannot locate the app-data dir")?;
    // An unready tree — with or without a manifest — is a failed attempt
    // (clone/install/build died midway): wipe and redo so retry heals. A
    // ready tree is never reinstalled — updates go through Upgrade dsh.
    if dest.exists() && packaged_backend_ready(&dest, true).is_err() {
        prov_phase("Clearing the failed attempt");
        std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    if dest.join("dsh-manifest.json").is_file() {
        return Err("already provisioned — backend updates go through Settings → Upgrade dsh".to_string());
    }
    if dest.exists() {
        return Err(format!(
            "{} exists but is not a provisioned backend — delete it or pick a custom backend",
            dest.display()
        ));
    }
    let parent = dest.parent().ok_or("packaged dir has no parent")?.to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let dest_arg = dest.to_string_lossy().into_owned();
    run_step(&parent, &git, &["clone", "--branch", UPSTREAM_BRANCH, "--single-branch", UPSTREAM_REMOTE, &dest_arg], "Cloning upstream dsh")?;
    let sha = git_out(&dest, &["rev-parse", "HEAD"])?;
    prov_phase("Writing launcher + updater");
    let manifest = serde_json::json!({"remote": UPSTREAM_REMOTE, "branch": UPSTREAM_BRANCH, "commit": sha});
    std::fs::write(dest.join("dsh-manifest.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    let launcher_name = platform::packaged_launcher_file(&dest)
        .file_name().map(|n| n.to_owned()).ok_or("launcher name")?;
    std::fs::write(dest.join(launcher_name), PACKAGED_LAUNCHER_TEXT).map_err(|e| e.to_string())?;
    std::fs::write(platform::updater_file(&dest), include_str!("../packaging/updater.cmd.template"))
        .map_err(|e| e.to_string())?;
    let exclude = dest.join(".git").join("info").join("exclude");
    let mut prev = std::fs::read_to_string(&exclude).unwrap_or_default();
    for name in ["dsh-manifest.json", &platform::packaged_launcher_file(&dest).file_name().unwrap().to_string_lossy().into_owned(), &platform::updater_file(&dest).file_name().unwrap().to_string_lossy().into_owned()] {
        if !prev.contains(name) {
            prev.push_str(&format!("\n{name}"));
        }
    }
    std::fs::write(&exclude, prev).map_err(|e| e.to_string())?;
    run_step(&dest, &pnpm, &["install"], "Installing dependencies (pnpm install)")?;
    run_step(&dest, &pnpm, &["run", "build"], "Building the backend (pnpm run build)")?;
    let _ = node;
    packaged_backend_ready(&dest, true)?;
    if let Some(flag) = setup_flag_path() {
        let _ = std::fs::remove_file(flag);
    }
    Ok(short_sha(&sha))
}

/// Relaunch the shell exe detached, then exit. Used after setup completes
/// and when entering setup from the menu. Port: windows `start`; POSIX
/// needs its own detach here (see `platform`).
fn relaunch_self() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", &exe.to_string_lossy()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    std::process::exit(0);
}

/// Validate a user-picked custom dir: looks like dsh, installed, built.
/// Their checkout, their updates — the shell only checks it can boot.
fn validate_custom_dir(path: &str) -> Result<String, String> {
    let dir = std::path::PathBuf::from(path.trim());
    if !dir.is_dir() {
        return Err("that folder does not exist".to_string());
    }
    if !custom_dir_usable(&dir) {
        return Err("not a dsh checkout (apps/cli/src/bin.ts missing)".to_string());
    }
    packaged_backend_ready(&dir, false)?;
    let version = backend_version(&dir).unwrap_or_else(|| "unknown version".to_string());
    Ok(version)
}

/// One-line setup-card state: what boots today, or why nothing does.
fn setup_state_text() -> String {
    let (spawn, source) = dsh_bin();
    match spawn {
        Some(BackendSpawn::Bin(_)) => format!("Current backend: {source}"),
        Some(BackendSpawn::NodeDir(d)) => format!("Current backend: {source} at {}", d.display()),
        None => match source {
            "custom backend (missing)" => "Your custom backend folder is missing — pick again or install bundled.".to_string(),
            "custom backend (unusable)" => "Your custom backend is no longer installed + built (run pnpm install / build there) — fix it, pick again, or install bundled.".to_string(),
            "packaged backend (unfinished)" => "The packaged backend never finished installing — choose Install bundled to wipe and retry.".to_string(),
            _ => "No dsh backend found on this machine.".to_string(),
        },
    }
}

/// (`GET /setup`): first-run card. Same visual language as the shellbar;
/// `__BASE__` placeholder like `shellbar_html` (avoids format! doubling).
fn setup_html() -> String {
    const PAGE: &str = r#"<!doctype html><html><head><meta charset=utf-8><title>dsh-desktop setup</title>
<style>
html,body{margin:0;padding:0;min-height:100%;background:#141414;color:#eee;font:13px 'Segoe UI',sans-serif}
#head{display:flex;align-items:center;gap:8px;height:32px;padding:0 0 0 10px;user-select:none}
#brand{color:#888;font-weight:600}
#wins{margin-left:auto;display:flex;height:32px}
.winbtn{background:none;border:none;width:46px;color:#ccc;font-size:11px;cursor:pointer}
.winbtn:hover{background:#3a3a3a}
#cls:hover{background:#c42b1c;color:#fff}
#card{max-width:560px;margin:8vh auto 40px;background:#1e1e1e;border:1px solid #3a3a3a;border-radius:12px;padding:26px 30px;box-sizing:border-box}
h1{font-size:18px;margin:0 0 6px;text-align:center}
#state{color:#9a9a9a;font-size:12px;margin-bottom:18px;text-align:center}
.opt{display:block;width:100%;text-align:left;background:#2d2d2d;color:#eee;border:1px solid #4a4a4a;border-radius:8px;padding:12px 14px;font-size:13px;cursor:pointer;margin-bottom:10px;font-family:inherit}
.opt:hover{background:#3a3a3a}
.opt small{display:block;color:#9a9a9a;font-size:11px;margin-top:3px}
#cpath{display:none;margin:4px 0 10px}
#cpath input{width:100%;background:#0d0d0d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:7px 10px;font-size:12px;font-family:inherit;box-sizing:border-box;margin-bottom:8px}
.row{display:flex;gap:8px}
button{background:#2d2d2d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:6px 14px;font-size:12px;cursor:pointer;font-family:inherit}
button:hover{background:#3a3a3a}
button:disabled{opacity:.5;cursor:default}
#phase{color:#9a9a9a;font-size:12px;margin:0 0 6px;min-height:16px}
#details{display:none;margin-top:14px}
#toggle{background:none;border:none;color:#9a9a9a;font-size:12px;cursor:pointer;display:flex;align-items:center;gap:6px;padding:4px 0;font-family:inherit}
#toggle:hover{color:#ccc}
#toggle svg{transition:transform .15s}
#toggle.open svg{transform:rotate(180deg)}
#log{display:none;background:#0d0d0d;border:1px solid #2e2e2e;border-radius:8px;padding:10px 12px;font:11px Consolas,monospace;white-space:pre-wrap;max-height:220px;overflow-y:auto;color:#bbb;margin-top:6px}
#foot{display:flex;justify-content:flex-end;margin-top:18px}
#cancel{background:none;border:1px solid #4a4a4a;color:#ccc;border-radius:6px;padding:6px 16px;font-size:12px;cursor:pointer;font-family:inherit}
#cancel:hover{background:#2d2d2d;color:#fff}
#cancel:disabled{opacity:.4;cursor:default}
</style></head><body>
<div id=head><span id=brand>dsh-desktop setup</span><span id=wins><button class=winbtn id=min>─</button><button class=winbtn id=max>▢</button><button class=winbtn id=cls>✕</button></span></div>
<div id=card><h1>Choose your dsh backend</h1><div id=state></div>
<button class=opt id=b-bundled>Install bundled dsh (recommended)<small>Clones upstream, installs, builds. Updates arrive via Settings → Upgrade dsh.</small></button>
<button class=opt id=b-custom>Use my own dsh<small>You update it yourself with plain git. The shell never touches it.</small></button>
<div id=cpath><input id=pin readonly placeholder="No folder chosen yet"><div class=row><button id=browse>Browse…</button><button id=use>Use this folder</button></div></div>
<div id=details><div id=phase></div><button id=toggle>Show details<svg width="10" height="10" viewBox="0 0 10 10"><path d="M2 3.5 5 6.5 8 3.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg></button><div id=log></div></div>
<div id=foot><button id=cancel>Cancel</button></div>
</div><script>
(function(){
var base='__BASE__';
function post(p,b){return fetch(base+p,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(b||{})}).then(function(r){return r.json()})}
document.getElementById('min').onclick=function(){post('/window',{action:'minimize'})};
document.getElementById('max').onclick=function(){post('/window',{action:'toggle-max'})};
document.getElementById('cls').onclick=function(){post('/window',{action:'close'})};
var head=document.getElementById('head');var dp=null;
head.addEventListener('mousedown',function(e){if(e.button!==0)return;if(e.target.closest('button'))return;dp=[e.clientX,e.clientY]});
head.addEventListener('mousemove',function(e){if(!dp)return;if(Math.abs(e.clientX-dp[0])+Math.abs(e.clientY-dp[1])>5){dp=null;fetch(base+'/drag-start',{method:'POST'})}});
head.addEventListener('mouseup',function(){dp=null});
var phase=document.getElementById('phase'),log=document.getElementById('log'),state=document.getElementById('state');
var cpath=document.getElementById('cpath'),pin=document.getElementById('pin');
var details=document.getElementById('details'),toggle=document.getElementById('toggle'),cancelBtn=document.getElementById('cancel');
var busy=false;
function revealDetails(){details.style.display='block'}
function setBusy(b){busy=b;cancelBtn.disabled=b}
toggle.onclick=function(){var open=log.style.display==='block';log.style.display=open?'none':'block';toggle.classList.toggle('open',!open);toggle.childNodes[0].textContent=!open?'Hide details':'Show details'};
document.getElementById('b-bundled').onclick=function(){
if(busy)return;setBusy(true);revealDetails();phase.textContent='Starting…';
post('/setup-start',{kind:'bundled'}).then(function(r){if(r.error){setBusy(false);phase.textContent='Failed: '+r.error}}).catch(function(){setBusy(false);phase.textContent='Shell unreachable'})};
document.getElementById('b-custom').onclick=function(){cpath.style.display='block'};
document.getElementById('browse').onclick=function(){post('/setup-pick',{}).then(function(r){if(r.path)pin.value=r.path})};
document.getElementById('use').onclick=function(){
if(busy||!pin.value)return;setBusy(true);revealDetails();phase.textContent='Validating…';
post('/setup-start',{kind:'custom',path:pin.value}).then(function(r){if(r.error){setBusy(false);phase.textContent='Not usable: '+r.error}else{phase.textContent='Accepted — relaunching…'}}).catch(function(){setBusy(false);phase.textContent='Shell unreachable'})};
document.getElementById('cancel').onclick=function(){if(busy)return;post('/setup-cancel',{})};
setInterval(function(){fetch(base+'/setup-status').then(function(r){return r.json()}).then(function(s){
state.textContent=s.state||'';if(s.phase){revealDetails();phase.textContent=s.phase}
if(s.log)log.textContent=s.log.join('\n');
if(s.done&&!s.error)phase.textContent=s.phase+' — relaunching…';
if(s.done&&s.error){setBusy(false);phase.textContent='Failed: '+s.error}}).catch(function(){})},1000);
})();
</script></body></html>"#;
    PAGE.replace("__BASE__", &format!("http://{CONTROL_ADDR}"))
}

/// Spawn `dsh web --no-open --port <free>` and wait (bounded) for the
/// authenticated server URL on its stdout. A wedged silent child is killed
/// instead of hanging the shell forever.
fn spawn_dsh(frame_port: u16) -> Result<(Child, String, u16), String> {
    let port = free_port().map_err(|e| format!("no free loopback port: {e}"))?;
    let (spawn, source) = dsh_bin();
    // Build the full child command per backend kind, sharing one stdio +
    // startup-line tail below. NodeDir (custom checkout) runs source mode
    // with cwd; everything else spawns its launcher with argv.
    let (mut cmd, label) = match spawn {
        Some(BackendSpawn::Bin(bin)) => {
            let mut c = Command::new(&bin);
            c.args(["web", "--no-open", "--port", &port.to_string()]);
            (c, bin)
        }
        Some(BackendSpawn::NodeDir(dir)) => {
            let label = dir.display().to_string();
            packaged_backend_ready(&dir, false)?;
            let mut c = Command::new("node");
            c.arg("--import")
                .arg("tsx/esm")
                .args(["apps/cli/src/bin.ts", "web", "--no-open", "--port", &port.to_string()])
                .current_dir(&dir);
            (c, label)
        }
        None => return Err("no dsh backend found (first-run setup never completed)".to_string()),
    };
    eprintln!("[browser] dsh backend: {label} ({source})");
    if source == "packaged backend" {
        if let Some(dir) = packaged_dir() {
            packaged_backend_ready(&dir, true)?;
        }
    }
    let mut child = cmd
        .env("DSH_BROWSER_FRAME_PORT", frame_port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start `{label}`: {e}"))?;

    // Drain stderr on a thread so a chatty child can never block on a pipe.
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                eprintln!("[dsh] {line}");
            }
        });
    }
    let stdout = child.stdout.take().ok_or("dsh child has no stdout")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut found = None;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(url) = line.strip_prefix(STARTUP_PREFIX).map(str::trim) {
                // The line may carry a trailing "(LAN: ...)" suffix; take the URL.
                found = Some(url.split_whitespace().next().unwrap_or(url).to_string());
                break;
            }
        }
        let _ = tx.send(found);
    });
    match rx.recv_timeout(START_TIMEOUT) {
        Ok(Some(url)) => Ok((child, url, port)),
        Ok(None) => Err("dsh exited without printing its `dsh web:` startup line".to_string()),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err("timed out waiting for the `dsh web:` startup line".to_string())
        }
    }
}

fn kill_child(slot: &Mutex<Option<Child>>) {
    if let Ok(mut guard) = slot.lock() {
        if let Some(mut child) = guard.take() {
            // Hard kill: portable graceful stop is follow-up work (a Ctrl-C
            // equivalent on Windows needs console job-object plumbing). dsh
            // owns its own teardown races; the shell guarantees no orphans.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Default download destination without new dependencies:
/// %USERPROFILE%\Downloads, falling back to the OS temp dir when the
/// profile env is missing.
fn default_downloads_dir() -> std::path::PathBuf {
    std::env::var("USERPROFILE")
        .map(|home| std::path::PathBuf::from(home).join("Downloads"))
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// Effective destination: the configured dir while it stays usable,
/// otherwise the default (a bad saved value never breaks downloads).
fn downloads_dir() -> std::path::PathBuf {
    let configured = shell_settings()
        .lock()
        .ok()
        .and_then(|s| s.download_dir.clone());
    if let Some(dir) = configured {
        let path = std::path::PathBuf::from(&dir);
        if path.is_absolute() && (path.is_dir() || std::fs::create_dir_all(&path).is_ok()) {
            return path;
        }
        eprintln!("[browser] configured download dir unusable, using default: {dir}");
    }
    default_downloads_dir()
}

fn sanitize_filename(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = clean.trim_matches('.').trim();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// Filename for a download URL. The session-export endpoint carries no
/// filename in its path (only `?sessionId=`), so rebuild the upstream
/// `dsh-session-<id>.zip` convention; anything else keeps its last segment.
fn download_filename(url: &url::Url) -> String {
    if url.path() == "/api/session.export" {
        let id = url
            .query_pairs()
            .find(|(k, _)| k == "sessionId")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        return format!("dsh-session-{}.zip", sanitize_filename(&id));
    }
    let last = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("");
    let base = sanitize_filename(last);
    if base == "download" || base.is_empty() {
        "download.bin".to_string()
    } else {
        base
    }
}

/// Shared download hook for both WebViews. WRY does not save downloads on
/// its own: without this, the export modal reports success (its HEAD check
/// passed) while no file ever lands — exactly the silent failure users see.
fn handle_download(view: Webview<Wry>, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => {
            let path = downloads_dir().join(download_filename(&url));
            eprintln!("[browser] download requested {} -> {}", url, path.display());
            *destination = path;
            true
        }
        DownloadEvent::Finished { url, path, success } => {
            match (success, path) {
                (true, Some(saved)) => {
                    eprintln!("[browser] download saved {} -> {}", url, saved.display());
                    show_download_toast(&view, true, &saved);
                }
                (true, None) => eprintln!("[browser] download finished {url} (path not reported)"),
                (false, _) => {
                    eprintln!("[browser] download FAILED {url}");
                    show_download_toast(&view, false, &downloads_dir());
                }
            }
            true
        }
        // Non-exhaustive upstream enum: deny unknown future variants.
        _ => false,
    }
}

/// Download confirmation rendered inside the DSH UI itself: a top-center
/// card naming the saved file and its folder, with a Close button and a
/// ~9s auto-dismiss. A separate native toast window painted blank in
/// testing (a runtime-created window plus child WebView never rendered its
/// page, over `file://` or loopback http alike); the DSH-WebView eval
/// channel below is the already-proven path, needs no new dependencies,
/// and stays visible in every tab (the native browser overlay only ever
/// covers the side-panel region).
fn show_download_toast(view: &Webview<Wry>, ok: bool, path: &std::path::Path) {
    let app = view.app_handle().clone();
    let file = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());
    let folder = path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let title = if ok {
        "Download complete"
    } else {
        "Download failed"
    };
    let (bg, edge) = if ok {
        ("#1e1e1e", "#3a3a3a")
    } else {
        ("#2a1a1a", "#5a2a2a")
    };
    // JSON string literals double as JS string literals; everything sinks
    // through textContent, so no HTML-escaping concerns. The full path goes
    // back to POST /reveal (same loopback fetch the panel bridge uses) when
    // "Show in folder" is clicked.
    let j_title = serde_json::to_string(&title).unwrap_or_default();
    let j_file = serde_json::to_string(&file).unwrap_or_default();
    let j_folder = serde_json::to_string(&folder).unwrap_or_default();
    let j_full = serde_json::to_string(&path.display().to_string()).unwrap_or_default();
    let js = format!(
        "(function(){{var o=document.getElementById('dsh-dl-toast');if(o)o.remove();\
        var t={j_title},f={j_file},d={j_folder},p={j_full};\
        var b=document.createElement('div');b.id='dsh-dl-toast';\
        b.style.cssText='position:fixed;top:14px;left:50%;transform:translateX(-50%);z-index:2147483647;background:{bg};color:#eee;border:1px solid {edge};border-radius:10px;padding:12px 14px;width:360px;box-sizing:border-box;font-family:\"Segoe UI\",sans-serif;font-size:12px;box-shadow:0 8px 28px rgba(0,0,0,.5)';\
        var h=document.createElement('div');h.textContent=t;h.style.cssText='font-weight:600;font-size:13px;margin-bottom:4px';\
        var a=document.createElement('div');a.textContent=f;a.style.cssText='font-size:12px;word-break:break-all';\
        var g=document.createElement('div');g.textContent=d;g.style.cssText='font-size:11px;color:#9a9a9a;word-break:break-all;margin-top:2px';\
        var r=document.createElement('div');r.style.cssText='text-align:right;margin-top:8px';\
        var s=document.createElement('button');s.textContent='Show in folder';s.style.cssText='background:#2d2d2d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:3px 12px;font-size:12px;cursor:pointer;margin-right:8px';\
        s.onclick=function(){{s.disabled=true;s.textContent='Opening…';fetch('http://{CONTROL_ADDR}/reveal',{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify({{path:p}})}}).then(function(){{s.textContent='Opened ✓';setTimeout(function(){{b.remove()}},800)}}).catch(function(){{s.disabled=false;s.textContent='Show in folder'}})}};\
        var c=document.createElement('button');c.textContent='Close';c.style.cssText='background:#2d2d2d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:3px 12px;font-size:12px;cursor:pointer';\
        c.onclick=function(){{b.remove()}};\
        r.appendChild(s);r.appendChild(c);b.appendChild(h);b.appendChild(a);b.appendChild(g);b.appendChild(r);\
        document.body.appendChild(b);setTimeout(function(){{b.remove()}},9000);}})()",
    );
    let build_app = app.clone();
    let build = move || {
        let Some(dsh) = build_app.get_webview("dsh") else {
            eprintln!("[browser] toast: dsh webview missing");
            return;
        };
        match dsh.eval(js.as_str()) {
            Ok(()) => eprintln!("[browser] toast shown via dsh dom"),
            Err(e) => eprintln!("[browser] toast eval failed: {e}"),
        }
    };
    if app.run_on_main_thread(build).is_err() {
        eprintln!("[browser] toast dispatch failed");
    }
}

/// Shell settings menu: a floating card inside the DSH page, directly under
/// the title bar's Settings label — a true overlay (fixed positioning,
/// max z-index), so opening it never reflows the page the way growing the
/// native strip did. Toggles on each title-bar click; a transparent
/// backdrop closes it on outside click. Same injected-DOM channel as the
/// download toast.
fn settings_menu_js() -> String {
    let backend_label = compat_label();
    format!(
        "(function(){{var o=document.getElementById('dsh-shell-menu');if(o){{closeMenu();return}}\
        var base='http://{CONTROL_ADDR}';\
        function post(p,b){{return fetch(base+p,{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify(b||{{}})}}).then(function(r){{return r.json()}})}};\
        var back=document.createElement('div');back.id='dsh-shell-menu-back';\
        back.style.cssText='position:fixed;inset:0;z-index:2147483646;background:transparent';\
        var m=document.createElement('div');m.id='dsh-shell-menu';\
        m.style.cssText='position:fixed;top:8px;left:100px;z-index:2147483647;width:260px;background:#1e1e1e;color:#eee;border:1px solid #3a3a3a;border-radius:10px;padding:6px;box-sizing:border-box;font-family:\"Segoe UI\",sans-serif;font-size:12px;box-shadow:0 8px 28px rgba(0,0,0,.5)';\
        function closeMenu(){{var mm=document.getElementById('dsh-shell-menu');if(mm)mm.remove();var bb=document.getElementById('dsh-shell-menu-back');if(bb)bb.remove();document.removeEventListener('keydown',escMenu,true)}}\
        function escMenu(e){{if(e.key==='Escape')closeMenu()}}\
        document.addEventListener('keydown',escMenu,true);\
        function item(label){{var d=document.createElement('div');d.textContent=label;d.style.cssText='padding:7px 10px;border-radius:6px;cursor:pointer;white-space:nowrap';d.onmouseenter=function(){{d.style.background='#2d2d2d'}};d.onmouseleave=function(){{d.style.background='transparent'}};m.appendChild(d);return d}}\
        var flab=document.createElement('div');flab.textContent='Download folder';flab.style.cssText='padding:7px 10px 3px;color:#9a9a9a;font-size:11px';m.appendChild(flab);\
        var frow=document.createElement('div');frow.style.cssText='display:flex;gap:8px;padding:0 10px 10px';\
        var inp=document.createElement('input');inp.type='text';\
        inp.style.cssText='flex:1;min-width:0;background:#0d0d0d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:5px 8px;font-size:12px;font-family:inherit';\
        var ch=document.createElement('button');ch.textContent='Change';ch.style.cssText='background:#2d2d2d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:4px 10px;font-size:12px;cursor:pointer;font-family:inherit;white-space:nowrap';\
        frow.appendChild(inp);frow.appendChild(ch);m.appendChild(frow);\
        var blab=document.createElement('div');blab.textContent='{backend_label}';blab.style.cssText='padding:3px 10px 10px;color:#9a9a9a;font-size:11px;white-space:nowrap';m.appendChild(blab);\
        item('Reload').onclick=function(){{post('/dsh-reload',{{}})}};\
        var sep=document.createElement('div');sep.style.cssText='border-top:1px solid #2e2e2e;margin:4px 6px';m.appendChild(sep);\
        item('Restart').onclick=function(){{if(!confirm('Restart the DSH web server? The UI reloads in a few seconds.'))return;closeMenu();\
        var rst=document.createElement('style');rst.textContent='@keyframes dshRestartSpin{{to{{transform:rotate(360deg)}}}}';document.head.appendChild(rst);\
        var ov=document.createElement('div');ov.id='dsh-restart-overlay';\
        ov.style.cssText='position:fixed;inset:0;z-index:2147483647;background:rgba(10,10,10,.72);display:flex;align-items:center;justify-content:center;font-family:\"Segoe UI\",sans-serif';\
        ov.innerHTML='<div style=\"background:#1e1e1e;border:1px solid #3a3a3a;border-radius:12px;padding:22px 30px;text-align:center;color:#eee\"><svg width=\"26\" height=\"26\" viewBox=\"0 0 26 26\" style=\"margin:0 auto 12px;animation:dshRestartSpin .8s linear infinite\"><circle cx=\"13\" cy=\"13\" r=\"10\" fill=\"none\" stroke=\"#575757\" stroke-width=\"3\"/><circle cx=\"13\" cy=\"13\" r=\"10\" fill=\"none\" stroke=\"#fff\" stroke-width=\"3\" stroke-linecap=\"round\" stroke-dasharray=\"16 47\"/></svg><div style=\"font-weight:600;font-size:14px\">Restarting DSH web…</div><div id=\"dsh-restart-sub\" style=\"color:#9a9a9a;font-size:12px;margin-top:6px\">Stopping the server…</div></div>';\
        document.body.appendChild(ov);post('/dsh-restart',{{}})}};\
        var usep=document.createElement('div');usep.style.cssText='border-top:1px solid #2e2e2e;margin:4px 6px';m.appendChild(usep);\
        var upg=item('Upgrade dsh');upg.onclick=function(){{upg.textContent='Checking…';post('/dsh-check',{{}}).then(function(r){{\
        if(r.status==='uptodate'){{upg.textContent='Up to date ✓';setTimeout(function(){{upg.textContent='Upgrade dsh'}},1500)}}\
        else if(r.status==='available'){{upg.textContent='Upgrade dsh';if(!confirm('Update dsh backend? '+r.local+' to '+r.remote+' ('+r.behind+' commits behind). The shell will close, update in the background, then reopen. Running agent turns will be interrupted.'))return;closeMenu();post('/dsh-upgrade',{{}})}}\
        else if(r.status==='diverged'){{upg.textContent='Local copy diverged';setTimeout(function(){{upg.textContent='Upgrade dsh'}},1800)}}\
        else if(r.status==='devmode'){{upg.textContent='Dev checkout mode';setTimeout(function(){{upg.textContent='Upgrade dsh'}},1500)}}\
        else{{upg.textContent='Check failed';setTimeout(function(){{upg.textContent='Upgrade dsh'}},1500)}}}}).catch(function(){{upg.textContent='Check failed';setTimeout(function(){{upg.textContent='Upgrade dsh'}},1500)}})}};\
        var bnd=item('Backend…');bnd.onclick=function(){{if(!confirm('Reopen backend setup? The shell will relaunch.'))return;closeMenu();post('/backend-setup',{{}})}};\
        function hold(b,t){{b.disabled=true;if(!b._t)b._t=b.textContent;b.textContent=t}}\
        function back(b,f){{if(f){{b.textContent=f;setTimeout(function(){{b.textContent=b._t;b.disabled=false}},1200)}}else{{b.textContent=b._t;b.disabled=false}}}}\
        function savePath(v){{hold(ch,'Saving…');post('/settings',{{downloadDir:v}}).then(function(r){{if(r.error){{back(ch,'Save failed')}}else{{back(ch,'Saved ✓');refresh()}}}}).catch(function(){{back(ch,'Save failed')}})}}\
        ch.onclick=function(){{hold(ch,'Choosing…');post('/browse-folder',{{}}).then(function(r){{if(r.cancelled){{back(ch)}}else if(r.path){{inp.value=r.path;savePath(r.path)}}}}).catch(function(){{back(ch,'Picker failed')}})}};\
        inp.onkeydown=function(e){{if(e.key==='Enter')savePath(inp.value)}};\
        function refresh(){{fetch(base+'/settings').then(function(r){{return r.json()}}).then(function(s){{inp.value=s.downloadDir||s.effectiveDownloadDir||''}}).catch(function(){{inp.placeholder='(shell unreachable)'}})}};\
        back.onclick=function(){{closeMenu()}};\
        document.body.appendChild(back);document.body.appendChild(m);refresh();}})()",
    )
}

fn show_settings_menu(app: &tauri::AppHandle) {
    let js = settings_menu_js();
    let handle = app.clone();
    if app
        .run_on_main_thread(move || {
            if let Some(view) = handle.get_webview("dsh") {
                if view.eval(js.as_str()).is_err() {
                    eprintln!("[browser] settings menu inject failed");
                }
            }
        })
        .is_err()
    {
        eprintln!("[browser] settings menu dispatch failed");
    }
}

/// Point the DSH page's restart overlay (if present) at a status line. The
/// old page outlives its dead server until navigate() replaces it, so this
/// stays visible through the whole dead-air window — including failure,
/// when it flips red instead of hanging on "Restarting…" forever.
fn restart_overlay_status(app: &tauri::AppHandle, text: &str, failed: bool) {
    let json_text = serde_json::to_string(text).unwrap_or_default();
    let color = if failed { "#e08080" } else { "#9a9a9a" };
    let js = format!("try{{var s=document.getElementById('dsh-restart-sub');if(s){{s.textContent={json_text};s.style.color='{color}'}}}}catch(e){{}}");
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(view) = handle.get_webview("dsh") {
            let _ = view.eval(js.as_str());
        }
    });
}

/// Kill the `dsh web` child and spawn a fresh one, then point the DSH
/// WebView at the new URL. Runs on a worker thread: the spawn alone takes
/// seconds, and it must never block the control plane. The pinned frame
/// port is reused, so the panel bridge keeps working; only the child's own
/// port changes, published through the shared ids for rect gating.
fn restart_dsh(app: tauri::AppHandle, ids: Arc<Mutex<ShellIds>>) {
    let frame_port = ids
        .lock()
        .map(|i| i.frame_port)
        .unwrap_or(DEFAULT_SHELL_FRAME_PORT);
    eprintln!("[browser] dsh restart requested");
    match app.try_state::<DshChild>() {
        Some(state) => kill_child(&state.0),
        None => eprintln!("[browser] dsh restart: no child state"),
    }
    restart_overlay_status(&app, "Starting a fresh server…", false);
    match spawn_dsh(frame_port) {
        Ok((child, url, port)) => {
            eprintln!("dsh-desktop: restarted, serving {url}");
            if let Some(state) = app.try_state::<DshChild>() {
                if let Ok(mut slot) = state.0.lock() {
                    *slot = Some(child);
                }
            }
            if let Ok(mut guard) = ids.lock() {
                guard.dsh_port = port;
            }
            let target: Option<url::Url> = url.parse().ok();
            let handle = app.clone();
            let fail_handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(view) = handle.get_webview("dsh") {
                    match target {
                        Some(target) => {
                            if view.navigate(target).is_err() {
                                eprintln!("[browser] dsh restart navigate failed");
                            }
                        }
                        None => {
                            eprintln!("[browser] dsh restart: bad URL");
                            restart_overlay_status(&fail_handle, "Restart failed — see the shell log", true);
                        }
                    }
                }
            });
            // Nothing to re-inject: the toolbar strip is a native child
            // WebView, and the panel bridge re-reports its rect on reload.
        }
        Err(message) => {
            eprintln!("[browser] dsh restart failed: {message}");
            restart_overlay_status(&app, "Restart failed — see the shell log", true);
        }
    }
}

/// Shell title bar served to the setup-time `shellbar` child WebView
/// (`GET /shellbar`): VS Code-style integrated bar, one 32px row — shell
/// actions left, download folder middle, native window buttons right.
/// The window itself is frameless (`decorations(false)`); dragging and the
/// window buttons round-trip through the loopback control plane, the same
/// channel the panel bridge already uses, so no invoke/capabilities
/// plumbing. A `__BASE__` placeholder avoids `format!` brace-doubling
/// through the CSS/JS.
fn shellbar_html() -> String {
    const PAGE: &str = r#"<!doctype html><html><head><meta charset=utf-8><title>shellbar</title>
<style>
html,body{margin:0;padding:0;height:100%;overflow:hidden;background:#141414;color:#eee;font:12px 'Segoe UI',sans-serif}
#bar{display:flex;align-items:center;gap:8px;height:32px;padding:0 0 0 10px;box-sizing:border-box;user-select:none}
#brand{color:#888;font-weight:600;white-space:nowrap;margin-right:4px}
button{background:#2d2d2d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:4px 10px;font-size:12px;cursor:pointer;white-space:nowrap;font-family:inherit}
button:hover{background:#3a3a3a}
button:disabled{opacity:.5;cursor:default}
#dl{background:#0d0d0d;color:#eee;border:1px solid #4a4a4a;border-radius:6px;padding:5px 8px;font-size:12px;font-family:inherit}
#st{padding:6px 10px 4px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#9a9a9a;font-size:11px;min-height:14px}
#sp{flex:0 0 auto;color:#555}
#settings-btn{background:none;border:none;border-radius:6px;padding:4px 10px}
#settings-btn:hover{background:#2d2d2d}
#wins{margin-left:auto;display:flex;align-items:stretch;height:32px}
.winbtn{background:none;border:none;border-radius:0;width:46px;padding:0;font-size:11px;color:#ccc}
.winbtn:hover{background:#3a3a3a}
#cls:hover{background:#c42b1c;color:#fff}
</style></head><body><div id=bar>
<span id=brand>dsh-desktop</span>
<button id=settings-btn title="Shell settings">⚙ Settings</button>
<span id=wins><button class=winbtn id=min title="Minimize">─</button><button class=winbtn id=max title="Maximize / restore">▢</button><button class=winbtn id=cls title="Close">✕</button></span>
</div><script>
(function(){
var base='__BASE__';
function post(p,b){return fetch(base+p,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(b||{})}).then(function(r){return r.json()})}
document.getElementById('settings-btn').onclick=function(){fetch(base+'/settings-menu',{method:'POST'})};
document.getElementById('min').onclick=function(){post('/window',{action:'minimize'})};
document.getElementById('max').onclick=function(){post('/window',{action:'toggle-max'})};
document.getElementById('cls').onclick=function(){post('/window',{action:'close'})};
var bar=document.getElementById('bar');
var downPos=null;
bar.addEventListener('mousedown',function(e){if(e.button!==0)return;if(e.target.closest('button')||e.target.closest('input'))return;downPos=[e.clientX,e.clientY]});
bar.addEventListener('mousemove',function(e){if(!downPos)return;if(Math.abs(e.clientX-downPos[0])+Math.abs(e.clientY-downPos[1])>5){downPos=null;fetch(base+'/drag-start',{method:'POST'})}});
bar.addEventListener('mouseup',function(){downPos=null});
bar.addEventListener('dblclick',function(e){if(e.target.closest('button')||e.target.closest('input'))return;post('/window',{action:'toggle-max'})});
})();
</script></body></html>"#;
    PAGE.replace("__BASE__", &format!("http://{CONTROL_ADDR}"))
}

/// Lay out the native window client area: toolbar strip on top, DSH page
/// below it, browser overlay re-applied from the last panel report (page
/// coordinates shift down by the strip height). Called at setup and on
/// every window resize; the DSH view has no auto_resize so these explicit
/// bounds are the single layout authority.
fn layout_shell(
    window: &tauri::Window,
    size: tauri::PhysicalSize<u32>,
    last_rect: &Arc<Mutex<PanelRect>>,
) {
    let scale = window.scale_factor().unwrap_or(1.0).max(0.01);
    let w = (size.width as f64 / scale).clamp(1.0, 8000.0);
    let h = (size.height as f64 / scale).clamp(1.0, 8000.0);
    // Setup mode: the shellbar view owns the whole client area (choice
    // card); normal mode keeps the 32px strip + DSH page split.
    let bar_h = if setup_mode() { h } else { SHELLBAR_H };
    if let Some(bar) = window.get_webview("shellbar") {
        let _ = bar.set_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(w, bar_h).into(),
        });
    }
    if let Some(dsh) = window.get_webview("dsh") {
        let _ = dsh.set_bounds(Rect {
            position: LogicalPosition::new(0.0, bar_h).into(),
            size: LogicalSize::new(w, (h - bar_h).max(1.0)).into(),
        });
    }
    let rect = last_rect.lock().map(|r| *r).unwrap_or_default();
    apply_rect(&window.app_handle(), &rect);
}

/// Client area in logical px; None when the window is momentarily unreadable
/// (apply_rect then falls back to unclamped bounds).
fn client_logical(app: &tauri::AppHandle) -> Option<(f64, f64)> {
    let window = app.get_window("main")?;
    let scale = window.scale_factor().unwrap_or(1.0).max(0.01);
    let phys = window.inner_size().ok()?;
    Some((phys.width as f64 / scale, phys.height as f64 / scale))
}

/// Apply one panel report to the native browser WebView: show + move/resize
/// while the Browser tab is visible, hide otherwise. Position/size are
/// Logical (DIP) units, matching the client's CSS px when zoom is 100%,
/// shifted down past the shell toolbar strip — and clamped to the client
/// area below the strip, so a stale report (resize/reflow in flight) can
/// neither cover the panel toolbar nor spill past the window edge.
fn apply_rect(app: &tauri::AppHandle, rect: &PanelRect) {
    let Some(view) = app.get_webview("browser") else {
        return;
    };
    if !rect.visible || rect.width < 2.0 || rect.height < 2.0 {
        let _ = view.hide();
        return;
    }
    let (max_w, max_h) = client_logical(app).unwrap_or((4000.0, 4000.0));
    let x = rect.x.max(0.0).min(max_w);
    let y = (SHELLBAR_H + rect.y.max(0.0)).min(max_h);
    let w = rect
        .width
        .clamp(1.0, 4000.0)
        .min((max_w - x).max(1.0));
    let h = rect
        .height
        .clamp(1.0, 4000.0)
        .min((max_h - y).max(1.0));
    let bounds = Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(w, h).into(),
    };
    if let Err(e) = view.set_bounds(bounds) {
        eprintln!("[browser] set_bounds failed: {e}");
        return;
    }
    if let Err(e) = view.show() {
        eprintln!("[browser] show failed: {e}");
    }
}

/// Evaluate JS in the *visible* browser WebView and wait for the JSON result.
/// Pattern follows the `tinybot` prior art (eval_with_callback + oneshot).
fn eval_in_browser(view: &Webview<Wry>, js: &str) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    view.eval_with_callback(js.to_string(), move |result| {
        if let Some(tx) = tx.lock().map(|mut g| g.take()).unwrap_or(None) {
            let _ = tx.send(result);
        }
    })
    .map_err(|e| format!("eval dispatch failed: {e}"))?;
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|_| "eval timed out after 10s".to_string())
}

/// Send one raw CDP command to the exact WebView2 backing the visible
/// browser WebView. Uses `with_webview` so there is no second browser
/// process, no remote-debugging port, and no other target to mistake.
fn call_cdp(
    view: &Webview<Wry>,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let method_owned = method.to_string();
    let params_text = params.to_string();
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let immediate_tx = tx.clone();
    let call_label = method_owned.clone();
    view.with_webview(move |platform| {
        let result = unsafe {
            platform
                .controller()
                .CoreWebView2()
                .map_err(|e| e.to_string())
                .and_then(|core| {
                    let callback_tx = tx.clone();
                    let callback = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                        move |status, json| {
                            let value = status.map(|_| json).map_err(|e| e.to_string());
                            let _ = callback_tx.send(value);
                            Ok(())
                        },
                    ));
                    core.CallDevToolsProtocolMethod(
                        &HSTRING::from(method_owned.as_str()),
                        &HSTRING::from(params_text.as_str()),
                        &callback,
                    )
                    .map_err(|e| e.to_string())
                })
        };
        if let Err(e) = result {
            let _ = immediate_tx.send(Err(e));
        }
    })
    .map_err(|e| format!("failed to schedule CDP {call_label}: {e}"))?;
    let raw = rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| format!("CDP {call_label} timed out after 10s"))?
        .map_err(|e| format!("CDP {call_label} failed: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("invalid CDP JSON for {call_label}: {e}"))
}

fn html_response(stream: &mut std::net::TcpStream, body: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\naccess-control-allow-origin: *\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn json_response(stream: &mut std::net::TcpStream, code: u16, body: &str) {    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Error",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\ncontent-type: application/json; charset=utf-8\r\ncontent-length: {}\r\naccess-control-allow-origin: *\r\naccess-control-allow-methods: GET, POST, OPTIONS\r\naccess-control-allow-headers: content-type\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, String, String)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut head = String::new();
    let request_line = reader.read_line(&mut head).ok()?;
    if request_line == 0 {
        return None;
    }
    let mut parts = head.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = trimmed.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length.min(2_000_000)];
    if content_length > 0 {
        reader.read_exact(&mut body).ok()?;
    }
    Some((method, path, String::from_utf8_lossy(&body).into_owned()))
}

/// Shell identity for panel routing: which DSH is ours and where its frame
/// server lives. Shared across control-plane connections so a DSH restart
/// can publish the new child port without rebinding the control plane.
#[derive(Clone)]
struct ShellIds {
    dsh_port: u16,
    frame_port: u16,
    frame_base: String,
}

/// Persisted shell knobs (APPDATA JSON). Only what the shell itself owns:
/// the download folder and the backend choice. Everything else stays
/// DSH-upstream.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct ShellSettings {
    #[serde(default, rename = "downloadDir")]
    download_dir: Option<String>,
    #[serde(default)]
    backend: Option<BackendChoice>,
}

/// First-run backend choice. `bundled` = provision from upstream (default);
/// `custom` = use the user's own dsh checkout at `path`, which they update
/// themselves — the shell uses it, validates it boots, otherwise leaves it
/// alone (never fetch/merge/reset outside the packaged dir).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct BackendChoice {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    path: Option<String>,
}

fn settings_path() -> std::path::PathBuf {
    std::env::var("APPDATA")
        .map(|roam| std::path::PathBuf::from(roam).join("dsh-desktop").join("settings.json"))
        .unwrap_or_else(|_| std::env::temp_dir().join("dsh-desktop-settings.json"))
}

static SHELL_SETTINGS: std::sync::OnceLock<Mutex<ShellSettings>> = std::sync::OnceLock::new();

/// Settings, loaded once at first use; mutate through
/// `update_shell_settings` so one knob never wipes another.
fn shell_settings() -> &'static Mutex<ShellSettings> {
    SHELL_SETTINGS.get_or_init(|| {
        let loaded: ShellSettings = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Mutex::new(loaded)
    })
}

/// Load-modify-save under one lock; the file holds every knob, so every
/// writer goes through here (a fresh-struct write would drop the rest).
fn update_shell_settings(f: impl FnOnce(&mut ShellSettings)) -> Result<(), String> {
    let mut slot = shell_settings().lock().map_err(|e| e.to_string())?;
    f(&mut slot);
    let text = serde_json::to_string_pretty(&*slot).map_err(|e| e.to_string())?;
    if let Some(parent) = settings_path().parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(settings_path(), text).map_err(|e| e.to_string())
}

/// Minimal loopback control plane for the experiment. No auth (loopback
/// only), no DSH knowledge: rect sync in, navigate/eval/CDP/state out.
/// Every route logs so the feel test has timestamps to compare.
fn serve_control(
    app: tauri::AppHandle,
    last_rect: Arc<Mutex<PanelRect>>,
    ids: Arc<Mutex<ShellIds>>,
) {
    let listener = match TcpListener::bind(CONTROL_ADDR) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[browser] control server bind {CONTROL_ADDR} failed: {e}");
            return;
        }
    };
    eprintln!("[browser] control server on http://{CONTROL_ADDR}");
    for stream in listener.incoming().flatten() {
        let app = app.clone();
        let last_rect = last_rect.clone();
        let ids = ids.clone();
        std::thread::spawn(move || {
            let mut stream = stream;
            let Some((method, path, body)) = read_request(&mut stream) else {
                return;
            };
            if method == "OPTIONS" {
                json_response(&mut stream, 200, "{}");
                return;
            }
            let (route_path, query) = match path.split_once('?') {
                Some((p, q)) => (p, q),
                None => (path.as_str(), ""),
            };
            let route = format!("{method} {route_path}");
            match route.as_str() {
                "GET /health" => json_response(&mut stream, 200, r#"{"ok":true}"#),
                // Shell toolbar strip page for the setup-time child WebView.
                "GET /shellbar" => html_response(&mut stream, &shellbar_html()),
                // First-run setup card (the shellbar view goes full-window;
                // no DSH page exists yet, so all of this is shell-owned).
                "GET /setup" => html_response(&mut stream, &setup_html()),
                "GET /setup-status" => {
                    let (phase, log, done, error) = PROVISION
                        .lock()
                        .map(|s| (s.phase.clone(), s.log.clone(), s.done, s.error.clone()))
                        .unwrap_or_default();
                    let tail: Vec<String> =
                        log.into_iter().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect();
                    let out = serde_json::json!({
                        "phase": phase, "log": tail, "done": done,
                        "error": error, "state": setup_state_text(),
                    });
                    json_response(&mut stream, 200, &out.to_string());
                }
                "POST /setup-pick" => {
                    let (tx, rx) = mpsc::channel::<Option<String>>();
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        let picked = handle
                            .dialog()
                            .file()
                            .set_title("Choose your dsh checkout")
                            .blocking_pick_folder()
                            .map(|p| p.to_string());
                        let _ = tx.send(picked);
                    });
                    match rx.recv_timeout(Duration::from_secs(300)) {
                        Ok(Some(path)) => {
                            eprintln!("[setup] picked folder {path}");
                            json_response(
                                &mut stream,
                                200,
                                &serde_json::json!({"ok": true, "path": path}).to_string(),
                            )
                        }
                        Ok(None) => {
                            json_response(&mut stream, 200, r#"{"ok":true,"cancelled":true}"#)
                        }
                        Err(_) => json_response(
                            &mut stream,
                            500,
                            r#"{"error":"folder picker timed out"}"#,
                        ),
                    }
                }
                "POST /setup-start" => match serde_json::from_str::<SetupBody>(&body) {
                    Ok(req) if req.kind == "bundled" => {
                        provision_bundled();
                        json_response(&mut stream, 200, r#"{"ok":true,"started":true}"#);
                    }
                    Ok(req) if req.kind == "custom" => match req.path {
                        Some(p) => match validate_custom_dir(&p) {
                            Ok(version) => {
                                let saved = p.trim().to_string();
                                match update_shell_settings(|s| {
                                    s.backend = Some(BackendChoice {
                                        kind: "custom".to_string(),
                                        path: Some(saved.clone()),
                                    })
                                }) {
                                    Ok(()) => {
                                        eprintln!("[setup] custom backend {saved} ({version})");
                                        if let Some(flag) = setup_flag_path() {
                                            let _ = std::fs::remove_file(flag);
                                        }
                                        json_response(&mut stream, 200, r#"{"ok":true}"#);
                                        std::thread::sleep(Duration::from_millis(400));
                                        relaunch_self();
                                    }
                                    Err(e) => json_response(
                                        &mut stream,
                                        500,
                                        &serde_json::json!({"error": e}).to_string(),
                                    ),
                                }
                            }
                            Err(e) => json_response(
                                &mut stream,
                                400,
                                &serde_json::json!({"error": e}).to_string(),
                            ),
                        },
                        None => json_response(
                            &mut stream,
                            400,
                            r#"{"error":"no folder chosen"}"#,
                        ),
                    },
                    _ => json_response(
                        &mut stream,
                        400,
                        r#"{"error":"unknown setup kind"}"#,
                    ),
                },
                "POST /setup-cancel" => {
                    if let Some(flag) = setup_flag_path() {
                        let _ = std::fs::remove_file(flag);
                    }
                    json_response(&mut stream, 200, r#"{"ok":true}"#);
                    let (spawn, _) = dsh_bin();
                    if spawn.is_some() {
                        std::thread::sleep(Duration::from_millis(300));
                        relaunch_self();
                    } else {
                        let handle = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            if let Some(window) = handle.get_window("main") {
                                let _ = window.close();
                            }
                        });
                    }
                }
                // Menu → Backend…: reopen setup on relaunch (flag survives
                // the restart; setup offers Cancel back to the current).
                "POST /backend-setup" => {
                    let written = setup_flag_path()
                        .and_then(|flag| {
                            if let Some(parent) = flag.parent() {
                                std::fs::create_dir_all(parent).ok()?;
                            }
                            std::fs::write(flag, "1").ok()
                        })
                        .is_some();
                    if written {
                        json_response(&mut stream, 200, r#"{"ok":true}"#);
                        std::thread::sleep(Duration::from_millis(300));
                        relaunch_self();
                    } else {
                        json_response(
                            &mut stream,
                            500,
                            r#"{"error":"cannot write setup flag"}"#,
                        );
                    }
                }
                // Frame-server discovery with identity: only our own DSH tab
                // learns our pinned port; anyone else gets null and keeps its
                // default (:9333, its own server). No cross-window steering.
                "GET /panel-config" => {
                    let asked = query
                        .split('&')
                        .find_map(|kv| kv.strip_prefix("dshPort=")?.parse::<u16>().ok());
                    let (dsh_port, frame_base) = ids
                        .lock()
                        .map(|i| (i.dsh_port, i.frame_base.clone()))
                        .unwrap_or((0, String::new()));
                    let base = if asked == Some(dsh_port) {
                        serde_json::json!(frame_base)
                    } else {
                        serde_json::Value::Null
                    };
                    json_response(
                        &mut stream,
                        200,
                        &serde_json::json!({"frameBase": base}).to_string(),
                    );
                }
                "GET /state" => {
                    let url = app
                        .get_webview("browser")
                        .and_then(|v| v.url().ok())
                        .map(|u| u.to_string())
                        .unwrap_or_default();
                    let rect = last_rect.lock().map(|r| *r).unwrap_or_default();
                    let out = serde_json::json!({
                        "url": url,
                        "visible": rect.visible,
                        "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height, "dpr": rect.dpr},
                    });
                    json_response(&mut stream, 200, &out.to_string());
                }
                "GET /rect" => {
                    let rect = last_rect.lock().map(|r| *r).unwrap_or_default();
                    let out = serde_json::json!({
                        "visible": rect.visible,
                        "x": rect.x, "y": rect.y,
                        "width": rect.width, "height": rect.height, "dpr": rect.dpr,
                    });
                    json_response(&mut stream, 200, &out.to_string());
                }
                "POST /rect" => {
                    match serde_json::from_str::<RectReport>(&body) {
                        Ok(rep) => {
                            let ours = ids.lock().map(|i| i.dsh_port).unwrap_or(0);
                            if rep.dsh_port != ours {
                                // Foreign reporter (another DSH beside the
                                // shell, or an older panel): ack without
                                // applying so its tab keeps its own server.
                                use std::sync::Once;
                                static WARNED: Once = Once::new();
                                WARNED.call_once(|| {
                                    eprintln!(
                                        "[browser] ignoring rect from foreign dshPort={} (ours={})",
                                        rep.dsh_port, ours
                                    );
                                });
                                json_response(&mut stream, 200, r#"{"ok":true,"native":false}"#);
                                return;
                            }
                            let rect = PanelRect {
                                visible: rep.visible,
                                x: rep.x,
                                y: rep.y,
                                width: rep.width,
                                height: rep.height,
                                dpr: rep.dpr.unwrap_or(1.0),
                            };
                            eprintln!(
                                "[browser] rect visible={} x={:.0} y={:.0} w={:.0} h={:.0} dpr={}",
                                rect.visible, rect.x, rect.y, rect.width, rect.height, rect.dpr
                            );
                            if let Ok(mut slot) = last_rect.lock() {
                                *slot = rect;
                            }
                            apply_rect(&app, &rect);
                            json_response(&mut stream, 200, r#"{"ok":true,"native":true}"#);
                        }
                        Err(e) => json_response(
                            &mut stream,
                            400,
                            &serde_json::json!({"error": format!("bad rect: {e}")}).to_string(),
                        ),
                    }
                }
                "POST /navigate" => match serde_json::from_str::<NavigateBody>(&body) {
                    Ok(nav) => match nav.url.parse() {
                        Ok(url) => {
                            eprintln!("[browser] agent navigate {}", nav.url);
                            match app.get_webview("browser") {
                                Some(v) => match v.navigate(url) {
                                    Ok(()) => json_response(&mut stream, 200, r#"{"ok":true}"#),
                                    Err(e) => json_response(
                                        &mut stream,
                                        500,
                                        &serde_json::json!({"error": e.to_string()}).to_string(),
                                    ),
                                },
                                None => json_response(
                                    &mut stream,
                                    500,
                                    r#"{"error":"browser webview missing"}"#,
                                ),
                            }
                        }
                        Err(_) => json_response(
                            &mut stream,
                            400,
                            r#"{"error":"url must parse as a URL"}"#,
                        ),
                    },
                    Err(e) => json_response(
                        &mut stream,
                        400,
                        &serde_json::json!({"error": format!("bad navigate: {e}")}).to_string(),
                    ),
                },
                "POST /eval" => match serde_json::from_str::<EvalBody>(&body) {
                    Ok(ev) => match app.get_webview("browser") {
                        Some(v) => match eval_in_browser(&v, &ev.js) {
                            Ok(result) => json_response(
                                &mut stream,
                                200,
                                &serde_json::json!({"ok": true, "result": result}).to_string(),
                            ),
                            Err(e) => json_response(
                                &mut stream,
                                500,
                                &serde_json::json!({"error": e}).to_string(),
                            ),
                        },
                        None => json_response(
                            &mut stream,
                            500,
                            r#"{"error":"browser webview missing"}"#,
                        ),
                    },
                    Err(e) => json_response(
                        &mut stream,
                        400,
                        &serde_json::json!({"error": format!("bad eval: {e}")}).to_string(),
                    ),
                },
                "POST /cdp" => match serde_json::from_str::<CdpBody>(&body) {
                    Ok(c) if !c.method.is_empty() => match app.get_webview("browser") {
                        Some(v) => {
                            eprintln!("[browser] agent CDP {}", c.method);
                            match call_cdp(&v, &c.method, c.params.unwrap_or_default()) {
                                Ok(value) => json_response(
                                    &mut stream,
                                    200,
                                    &serde_json::json!({"ok": true, "result": value}).to_string(),
                                ),
                                Err(e) => json_response(
                                    &mut stream,
                                    500,
                                    &serde_json::json!({"error": e}).to_string(),
                                ),
                            }
                        }
                        None => json_response(
                            &mut stream,
                            500,
                            r#"{"error":"browser webview missing"}"#,
                        ),
                    },
                    _ => json_response(
                        &mut stream,
                        400,
                        r#"{"error":"method must be a non-empty string"}"#,
                    ),
                },
                "POST /dsh-eval" => {
                    // Experiment-only: run JS inside the DSH UI WebView
                    // (overlay/airspace tests). Same eval pattern, other target.
                    match serde_json::from_str::<EvalBody>(&body) {
                        Ok(ev) => match app.get_webview("dsh") {
                            Some(v) => match eval_in_browser(&v, &ev.js) {
                                Ok(result) => json_response(
                                    &mut stream,
                                    200,
                                    &serde_json::json!({"ok": true, "result": result}).to_string(),
                                ),
                                Err(e) => json_response(
                                    &mut stream,
                                    500,
                                    &serde_json::json!({"error": e}).to_string(),
                                ),
                            },
                            None => json_response(
                                &mut stream,
                                500,
                                r#"{"error":"dsh webview missing"}"#,
                            ),
                        },
                        Err(e) => json_response(
                            &mut stream,
                            400,
                            &serde_json::json!({"error": format!("bad eval: {e}")}).to_string(),
                        ),
                    }
                }
                "POST /show" => {
                    if let Some(v) = app.get_webview("browser") {
                        let _ = v.show();
                    }
                    json_response(&mut stream, 200, r#"{"ok":true}"#);
                }
                "POST /hide" => {
                    if let Some(v) = app.get_webview("browser") {
                        let _ = v.hide();
                    }
                    json_response(&mut stream, 200, r#"{"ok":true}"#);
                }
                // Experiment-only: fire the download toast without a real
                // download, so the DOM card can be verified on demand.
                "POST /toast-test" => {
                    if let Some(v) = app.get_webview("dsh") {
                        show_download_toast(&v, true, &downloads_dir().join("dsh-session-test.zip"));
                    }
                    json_response(&mut stream, 200, r#"{"ok":true}"#);
                }
                // Toast "Show in folder": open Explorer at the saved file.
                // Constrained to the Downloads dir; anything else is refused.
                "POST /reveal" => match serde_json::from_str::<RevealBody>(&body) {
                    Ok(req) => {
                        let target = std::path::PathBuf::from(&req.path);
                        let root = downloads_dir();
                        if req.path.is_empty() || !target.starts_with(&root) {
                            eprintln!("[browser] reveal refused {}", req.path);
                            json_response(&mut stream, 403, r#"{"error":"outside downloads"}"#);
                        } else if !target.exists() {
                            eprintln!("[browser] reveal missing {}", req.path);
                            json_response(&mut stream, 404, r#"{"error":"not found"}"#);
                        } else {
                            eprintln!("[browser] reveal {}", req.path);
                            // A file opens its folder highlighted; a folder
                            // just opens. Detached: never block the control
                            // plane on Explorer.
                            let mut cmd = Command::new("explorer");
                            if target.is_file() {
                                cmd.arg(format!("/select,{}", target.display()));
                            } else {
                                cmd.arg(&target);
                            }
                            match cmd.spawn() {
                                Ok(_) => json_response(&mut stream, 200, r#"{"ok":true}"#),
                                Err(e) => json_response(
                                    &mut stream,
                                    500,
                                    &serde_json::json!({"error": e.to_string()}).to_string(),
                                ),
                            }
                        }
                    }
                    Err(e) => json_response(
                        &mut stream,
                        400,
                        &serde_json::json!({"error": format!("bad reveal: {e}")}).to_string(),
                    ),
                },
                // Native folder picker for the menu's Change button. The
                // blocking dialog must run on the main thread; this route
                // waits on a oneshot for the user's choice (or cancel).
                "POST /browse-folder" => {
                    let (tx, rx) = mpsc::channel::<Option<String>>();
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        let picked = handle
                            .dialog()
                            .file()
                            .set_title("Choose download folder")
                            .blocking_pick_folder()
                            .map(|p| p.to_string());
                        let _ = tx.send(picked);
                    });
                    match rx.recv_timeout(Duration::from_secs(300)) {
                        Ok(Some(path)) => {
                            eprintln!("[browser] picked folder {path}");
                            json_response(
                                &mut stream,
                                200,
                                &serde_json::json!({"ok": true, "path": path}).to_string(),
                            )
                        }
                        Ok(None) => {
                            json_response(&mut stream, 200, r#"{"ok":true,"cancelled":true}"#)
                        }
                        Err(_) => json_response(
                            &mut stream,
                            500,
                            r#"{"error":"folder picker timed out"}"#,
                        ),
                    }
                }
                "GET /settings" => {
                    let configured = shell_settings()
                        .lock()
                        .ok()
                        .and_then(|s| s.download_dir.clone())
                        .unwrap_or_default();
                    let out = serde_json::json!({
                        "downloadDir": configured,
                        "effectiveDownloadDir": downloads_dir().display().to_string(),
                        "defaultDownloadDir": default_downloads_dir().display().to_string(),
                    });
                    json_response(&mut stream, 200, &out.to_string());
                }
                "POST /settings" => match serde_json::from_str::<SettingsBody>(&body) {
                    Ok(req) => {
                        let dir = req.download_dir.trim().to_string();
                        // Empty clears back to the default destination.
                        if dir.is_empty() {
                            match update_shell_settings(|s| s.download_dir = None) {
                                Ok(()) => {
                                    eprintln!("[browser] download dir cleared to default");
                                    json_response(
                                        &mut stream,
                                        200,
                                        &serde_json::json!({"ok": true, "downloadDir": ""})
                                            .to_string(),
                                    );
                                }
                                Err(e) => json_response(
                                    &mut stream,
                                    500,
                                    &serde_json::json!({"error": e}).to_string(),
                                ),
                            }
                            return;
                        }
                        let candidate = std::path::PathBuf::from(&dir);
                        if !candidate.is_absolute() {
                            json_response(
                                &mut stream,
                                400,
                                r#"{"error":"downloadDir must be an absolute path"}"#,
                            );
                        } else if std::fs::create_dir_all(&candidate).is_err() {
                            json_response(
                                &mut stream,
                                400,
                                r#"{"error":"cannot create that folder"}"#,
                            );
                        } else {
                            let saved = candidate.display().to_string();
                            match update_shell_settings(|s| s.download_dir = Some(saved.clone())) {
                                Ok(()) => {
                                    eprintln!("[browser] download dir set {saved}");
                                    json_response(
                                        &mut stream,
                                        200,
                                        &serde_json::json!({"ok": true, "downloadDir": saved})
                                            .to_string(),
                                    );
                                }
                                Err(e) => json_response(
                                    &mut stream,
                                    500,
                                    &serde_json::json!({"error": e.to_string()}).to_string(),
                                ),
                            }
                        }
                    }
                    Err(e) => json_response(
                        &mut stream,
                        400,
                        &serde_json::json!({"error": format!("bad settings: {e}")}).to_string(),
                    ),
                },
                // Soft reload: refresh the DSH UI in place (sessions kept).
                "POST /dsh-reload" => {
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(view) = handle.get_webview("dsh") {
                            match view.eval("location.reload()") {
                                Ok(()) => eprintln!("[browser] dsh reloaded"),
                                Err(e) => eprintln!("[browser] dsh reload failed: {e}"),
                            }
                        }
                    });
                    json_response(&mut stream, 200, r#"{"ok":true}"#);
                }
                // Hard restart: kill the `dsh web` child, spawn a fresh one
                // on the same pinned frame port, and point the DSH WebView
                // at it. Slow (seconds); the worker logs the outcome.
                "POST /dsh-restart" => {
                    let worker_app = app.clone();
                    let worker_ids = ids.clone();
                    std::thread::spawn(move || restart_dsh(worker_app, worker_ids));
                    json_response(&mut stream, 200, r#"{"restarting":true}"#);
                }
                // Backend update check: fetch the tracked fork branch (touches
                // only remote-tracking refs, safe while the child runs) and
                // compare. Never merges, never closes anything.
                "POST /dsh-check" => {
                    let out = check_backend_update();
                    json_response(&mut stream, 200, &out.to_string());
                }
                // Backend upgrade: the updater runs detached (new console,
                // outlives us), then this process exits. The updater merges,
                // installs, relaunches the shell, and the next boot toasts
                // the result. Refused outside the packaged backend: a dev
                // checkout updates with plain git.
                "POST /dsh-upgrade" => {
                    let (_, source) = dsh_bin();
                    if source != "packaged backend" {
                        json_response(&mut stream, 200, r#"{"ok":false,"error":"dev checkout mode: update with git directly"}"#);
                        return;
                    }
                    let Some(dir) = packaged_dir() else {
                        json_response(&mut stream, 200, r#"{"ok":false,"error":"no packaged backend dir"}"#);
                        return;
                    };
                    let Some(manifest) = packaged_manifest() else {
                        json_response(&mut stream, 200, r#"{"ok":false,"error":"packaged backend has no manifest"}"#);
                        return;
                    };
                    let updater = platform::updater_file(&dir);
                    if !updater.is_file() {
                        json_response(&mut stream, 200, r#"{"ok":false,"error":"updater script missing from packaged backend"}"#);
                        return;
                    }
                    // The updater's git commands need the remote name; the
                    // manifest records the URL, so resolve it here and pass
                    // the name down.
                    let Some(remote_name) = tracked_remote_name(&dir, &manifest.remote) else {
                        json_response(&mut stream, 200, r#"{"ok":false,"error":"tracked fork is not a remote of the packaged clone"}"#);
                        return;
                    };
                    let shell_exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    if shell_exe.is_empty() {
                        json_response(&mut stream, 200, r#"{"ok":false,"error":"cannot locate shell exe for relaunch"}"#);
                        return;
                    }
                    eprintln!("[browser] dsh upgrade accepted: {} -> {} ({}), relaunching via updater", manifest.branch, manifest.remote, dir.display());
                    let spawn_out = std::process::Command::new("cmd")
                        .args(["/C", "start", "/min", "", &updater.to_string_lossy(), &dir.to_string_lossy(), &remote_name, &manifest.branch, &shell_exe])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                    if let Err(e) = spawn_out {
                        json_response(&mut stream, 200, &serde_json::json!({"ok": false, "error": format!("updater would not start: {e}")}).to_string());
                        return;
                    }
                    json_response(&mut stream, 200, r#"{"ok":true,"closing":true}"#);
                    if let Some(state) = app.try_state::<DshChild>() {
                        kill_child(&state.0);
                    }
                    std::process::exit(0);
                }
                // Agent-driven tab reveal: the dsh-browser client exposes a
                // hook calling the supported sidebarRight openTab path
                // (expands the column, focuses the tab, idempotent). Waits
                // for the panel-bridge rect so the caller knows the overlay
                // is actually up before acting on the page.
                "POST /reveal-tab" => {
                    eprintln!("[browser] reveal-tab requested");
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(view) = handle.get_webview("dsh") {
                            let _ = view.eval("try{window.__dshBrowserReveal?window.__dshBrowserReveal():'no-hook'}catch(e){'hook-failed:'+e}");
                        }
                    });
                    let deadline = std::time::Instant::now() + Duration::from_secs(6);
                    let mut visible = false;
                    while std::time::Instant::now() < deadline {
                        if last_rect.lock().map(|r| r.visible).unwrap_or(false) {
                            visible = true;
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    eprintln!("[browser] reveal-tab visible={visible}");
                    json_response(
                        &mut stream,
                        200,
                        &serde_json::json!({"ok": true, "visible": visible}).to_string(),
                    );
                }
                // Settings menu overlay (title-bar Settings label click).
                "POST /settings-menu" => {
                    show_settings_menu(&app);
                    json_response(&mut stream, 200, r#"{"ok":true}"#);
                }
                // Frameless title-bar drag: the strip's mousedown round-trips
                // here (~1ms loopback) and the OS takes over the move.
                "POST /drag-start" => {
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(window) = handle.get_window("main") {
                            if window.start_dragging().is_err() {
                                eprintln!("[browser] drag start failed");
                            }
                        }
                    });
                    json_response(&mut stream, 200, r#"{"ok":true}"#);
                }
                // Frameless title-bar buttons.
                "POST /window" => match serde_json::from_str::<WindowBody>(&body) {
                    Ok(req) => {
                        let handle = app.clone();
                        let action = req.action.clone();
                        let _ = app.run_on_main_thread(move || {
                            let Some(window) = handle.get_window("main") else {
                                return;
                            };
                            let outcome = match action.as_str() {
                                "minimize" => window.minimize(),
                                "toggle-max" => match window.is_maximized() {
                                    Ok(true) => window.unmaximize(),
                                    _ => window.maximize(),
                                },
                                "close" => window.close(),
                                other => {
                                    eprintln!("[browser] unknown window action {other}");
                                    return;
                                }
                            };
                            if outcome.is_err() {
                                eprintln!("[browser] window action failed {action}");
                            }
                        });
                        json_response(&mut stream, 200, r#"{"ok":true}"#);
                    }
                    Err(e) => json_response(
                        &mut stream,
                        400,
                        &serde_json::json!({"error": format!("bad window: {e}")}).to_string(),
                    ),
                },
                _ => json_response(&mut stream, 404, r#"{"error":"unknown route"}"#),
            }
        });
    }
}

/// Setup window: shellbar view full-client on `GET /setup`, no child, no
/// DSH views. Control plane still serves (same routes); resize keeps the
/// card full-bleed via the setup branch in `layout_shell`.
fn run_setup_window(frame_port: u16) {
    eprintln!("[setup] entering first-run setup (no usable backend)");
    let last_rect = Arc::new(Mutex::new(PanelRect::default()));
    let rect_for_setup = last_rect.clone();
    let ids = Arc::new(Mutex::new(ShellIds {
        dsh_port: 0,
        frame_port,
        frame_base: format!("http://127.0.0.1:{frame_port}"),
    }));
    let ids_for_setup = ids.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(DshChild(Mutex::new(None)))
        .setup(move |app| {
            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("dsh-desktop setup")
                .decorations(false)
                .inner_size(900.0, 700.0)
                .build()?;
            let scale = window.scale_factor().unwrap_or(1.0).max(0.01);
            let phys = window.inner_size()?;
            let win_w = phys.width as f64 / scale;
            let win_h = phys.height as f64 / scale;
            let setup_url: url::Url = format!("http://{CONTROL_ADDR}/setup")
                .parse()
                .map_err(|e| format!("setup URL failed: {e}"))?;
            let bar_view = window.add_child(
                WebviewBuilder::new("shellbar", WebviewUrl::External(setup_url)),
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(win_w, win_h),
            )?;
            bar_view.show()?;
            eprintln!("[setup] setup card shown");
            let handle = app.handle().clone();
            std::thread::spawn(move || serve_control(handle, rect_for_setup, ids_for_setup));
            Ok(())
        })
        .on_window_event({
            let rect_for_resize = last_rect.clone();
            move |window, event| {
                match event {
                    tauri::WindowEvent::CloseRequested { .. }
                    | tauri::WindowEvent::Destroyed => {
                        kill_child(&window.state::<DshChild>().0);
                    }
                    tauri::WindowEvent::Resized(size) => {
                        layout_shell(window, *size, &rect_for_resize);
                    }
                    _ => {}
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("tauri runtime failed");
}

fn main() {
    let frame_port = shell_frame_port();
    // Dev override always boots straight through; otherwise a setup flag
    // (menu → Backend…) or no usable backend at all enters setup mode.
    let env_dev = std::env::var("DSH_BIN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let (spawn_opt, _) = dsh_bin();
    let flag = setup_flag_path().is_some_and(|p| p.is_file());
    SETUP_MODE.store(!env_dev && (flag || spawn_opt.is_none()), std::sync::atomic::Ordering::Relaxed);
    if setup_mode() {
        run_setup_window(frame_port);
        return;
    }
    let (child, url, dsh_port) = match spawn_dsh(frame_port) {
        Ok(triple) => triple,
        Err(message) => {
            eprintln!("dsh-desktop: {message}");
            std::process::exit(1);
        }
    };
    eprintln!("dsh-desktop: serving {url}");
    let last_rect = Arc::new(Mutex::new(PanelRect::default()));
    let rect_for_setup = last_rect.clone();
    let ids = Arc::new(Mutex::new(ShellIds {
        dsh_port,
        frame_port,
        frame_base: format!("http://127.0.0.1:{frame_port}"),
    }));
    let ids_for_setup = ids.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(DshChild(Mutex::new(Some(child))))
        .setup(move |app| {
            let parsed: url::Url = url
                .parse()
                .map_err(|e| format!("dsh printed an unparsable URL: {e}"))?;
            // One native window, three WebViews: the shell toolbar strip on
            // top (setup-time child, served by GET /shellbar), the DSH UI
            // below it, and the browser overlay (hidden until the Browser
            // tab reports a rect). Explicit bounds everywhere — no
            // auto_resize — so layout_shell owns every pixel on resize.
            // Frameless shell window with a VS Code-style integrated title
            // bar (the setup-time `shellbar` child). Native drag, resize,
            // minimize and taskbar behavior stay with the OS; only the
            // pixels are ours.
            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("dsh")
                .decorations(false)
                .inner_size(1400.0, 900.0)
                .build()?;
            let scale = window.scale_factor().unwrap_or(1.0).max(0.01);
            let phys = window.inner_size()?;
            let win_w = phys.width as f64 / scale;
            let win_h = phys.height as f64 / scale;
            let shellbar_url: url::Url = format!("http://{CONTROL_ADDR}/shellbar")
                .parse()
                .map_err(|e| format!("shellbar URL failed: {e}"))?;
            let bar_view = window.add_child(
                WebviewBuilder::new("shellbar", WebviewUrl::External(shellbar_url)),
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(win_w, SHELLBAR_H),
            )?;
            bar_view.show()?;
            let dsh_view = window.add_child(
                WebviewBuilder::new("dsh", WebviewUrl::External(parsed))
                    .on_download(handle_download),
                LogicalPosition::new(0.0, SHELLBAR_H),
                LogicalSize::new(win_w, (win_h - SHELLBAR_H).max(1.0)),
            )?;
            dsh_view.show()?;
            let browser_view = window.add_child(
                WebviewBuilder::new(
                    "browser",
                    WebviewUrl::External("https://github.com".parse().expect("static URL parses")),
                )
                .on_download(handle_download),
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(800.0, 600.0),
            )?;
            browser_view.hide()?;
            eprintln!("[browser] webviews ready: shellbar + dsh + browser(hidden)");
            let handle = app.handle().clone();
            std::thread::spawn(move || serve_control(handle, rect_for_setup, ids_for_setup));
            // Updater result toast: the detached updater leaves its verdict in
            // the status file; surface it once the fresh page is up, then
            // consume the file so it toasts exactly once. The toast replaces
            // itself, so a few blind retries while the page loads are harmless.
            let toast_app = app.handle().clone();
            std::thread::spawn(move || {
                let Some(path) = update_status_path() else { return };
                let text = match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(_) => return,
                };
                let _ = std::fs::remove_file(&path);
                let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let (title, body, ok) = if value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let sha = value.get("sha").and_then(|v| v.as_str()).unwrap_or("?");
                    let note = value.get("note").and_then(|v| v.as_str()).unwrap_or("");
                    let body = if note.is_empty() {
                        format!("Backend is now {sha}")
                    } else {
                        format!("Backend is now {sha} ({note})")
                    };
                    ("dsh updated".to_string(), body, true)
                } else {
                    let stage = value.get("stage").and_then(|v| v.as_str()).unwrap_or("update");
                    let detail = value.get("detail").and_then(|v| v.as_str()).unwrap_or("unknown reason");
                    (("dsh update failed".to_string(), format!("{stage}: {detail}"), false))
                };
                for _ in 0..4 {
                    std::thread::sleep(std::time::Duration::from_secs(8));
                    show_note_toast(&toast_app, &title, &body, ok);
                }
            });
            // Browser/backend compatibility probe: version the running
            // backend against the staged dsh-browser's tested range, cache
            // the verdict for the Settings label, and warn (never refuse)
            // on mismatch. Custom backends are self-managed: labeled only.
            let compat_app = app.handle().clone();
            std::thread::spawn(move || {
                let (label, mismatch) = check_browser_compat();
                eprintln!("[browser] compat: {label}");
                if let Ok(mut slot) = COMPAT_LABEL.lock() {
                    *slot = label;
                }
                if let Some((version, requirement)) = mismatch {
                    let title = "Browser tab unavailable".to_string();
                    let body = format!(
                        "Backend {version} is outside dsh-browser's tested range (needs {requirement}). Chat is unaffected."
                    );
                    for _ in 0..3 {
                        std::thread::sleep(std::time::Duration::from_secs(8));
                        show_note_toast(&compat_app, &title, &body, false);
                    }
                }
            });
            Ok(())
        })
        .on_window_event({
            let rect_for_resize = last_rect.clone();
            move |window, event| {
                match event {
                    tauri::WindowEvent::CloseRequested { .. }
                    | tauri::WindowEvent::Destroyed => {
                        kill_child(&window.state::<DshChild>().0);
                    }
                    tauri::WindowEvent::Resized(size) => {
                        layout_shell(window, *size, &rect_for_resize);
                    }
                    _ => {}
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("tauri runtime failed");
}
