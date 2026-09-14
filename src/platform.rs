//! Platform seams: every OS-specific assumption in one place.
//!
//! Windows-first; macOS/Linux are contributor-led, best-effort. A port
//! touches this file plus the `packaging/` script twins (`.cmd` ↔ `.sh`)
//! and the Tauri bundle target — never call sites. What lives here:
//!
//! - per-user app-data root (`LOCALAPPDATA` on Windows,
//!   `~/Library/Application Support` on macOS, `~/.local/share` on Linux)
//! - generated script names (`dsh-packaged`, `updater`, dev `dsh` shim)
//! - the detached-spawn contract (Windows: `cmd /C start /min`, which
//!   outlives the parent; POSIX: double-fork/nohup or `open`, TBD by the
//!   porter — the call site is marked)
use std::path::{Path, PathBuf};

/// Marker file next to the exe that switches this binary to portable mode
/// (shipped inside the portable zip, self-documenting). Absent = the
/// normal per-user AppData layout, exactly as the installer leaves it.
pub fn portable_marker_file() -> &'static str {
    "portable.txt"
}

/// Portable root: `<exe dir>/data` when the marker file sits beside the
/// exe, else `None`. The same binary serves both layouts — the marker is
/// the only switch, so an installed copy and a portable copy stay
/// byte-identical and independently testable.
pub fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join(portable_marker_file())
        .is_file()
        .then(|| dir.join("data"))
}

/// Per-user root for shell-owned state: backend clone, staged browser
/// copy, updater status. Per-user (never Program Files) so backend
/// updates never need elevation; portable mode keeps it beside the exe.
pub fn app_data_dir() -> Option<PathBuf> {
    if let Some(root) = portable_root() {
        return Some(root);
    }
    // Port: windows → LOCALAPPDATA; macos → home + Library/Application
    // Support; linux → $XDG_DATA_HOME or home + .local/share.
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|base| Path::new(&base).join("dsh-desktop"))
}

/// Directory holding the shell's own settings JSON (roaming on Windows
/// so the backend choice follows the user profile). Portable mode keeps
/// it with the rest of the portable state.
pub fn settings_dir() -> Option<PathBuf> {
    if let Some(root) = portable_root() {
        return Some(root);
    }
    std::env::var("APPDATA").ok().map(|roam| Path::new(&roam).join("dsh-desktop"))
}

/// One-line description of where shell state lives, logged at boot so a
/// support log always names the layout in effect.
pub fn state_root_note() -> String {
    match (portable_root(), app_data_dir()) {
        (Some(root), _) => format!("portable — state in {}", root.display()),
        (None, Some(root)) => format!("per-user — state in {}", root.display()),
        (None, None) => "per-user — no state root resolved".to_string(),
    }
}

/// Home of the installer-packaged dsh backend: pristine upstream clone
/// plus `dsh-manifest.json` and the generated launcher/updater.
pub fn packaged_dir() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("dsh-src"))
}

/// Home of the staged dsh-browser copy the shell reads compatibility
/// metadata from. Staged beside the backend at provision time; refreshed
/// on shell releases.
pub fn bundled_browser_dir() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("dsh-browser"))
}

/// Where the detached updater leaves its result for the next boot to toast.
pub fn update_status_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("update-status.json"))
}

/// Extension of generated shell scripts. The provision step writes the
/// matching twin (`.cmd` here, `.sh` on POSIX with chmod +x).
pub fn script_ext() -> &'static str {
    "cmd"
}

/// Generated backend launcher inside the packaged dir.
pub fn packaged_launcher_file(dir: &std::path::Path) -> PathBuf {
    dir.join(format!("dsh-packaged.{}", script_ext()))
}

/// Detached updater inside the packaged dir.
pub fn updater_file(dir: &std::path::Path) -> PathBuf {
    dir.join(format!("updater.{}", script_ext()))
}

/// Spawn a console tool without ever flashing a console window.
/// Release builds are `windows_subsystem` GUI apps; without this flag
/// every git/node/pnpm/cmd child would pop its own console. Stdio pipes
/// keep working — only the visible window is suppressed.
pub fn silent_command<S: AsRef<std::ffi::OsStr>>(prog: S) -> std::process::Command {
    let mut cmd = std::process::Command::new(prog);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}

/// Dev-only shim sitting next to the shell exe (never shipped).
pub fn dev_shim_file(exe_dir: &std::path::Path) -> PathBuf {
    exe_dir.join(format!("dsh.{}", script_ext()))
}

/// Fast PATH probe (no execution): is `name` resolvable right now?
/// Used to tell "a PATH backend exists" from "nothing installed", so
/// first-run setup only appears when there is genuinely nothing to boot.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    find_on_path_all(name).into_iter().next()
}

/// Every `where`/`which` hit, in order. Spawning needs a directly
/// executable image: `where pnpm` also lists the extensionless Node
/// entry script, which CreateProcess rejects (os error 193).
pub fn find_on_path_all(name: &str) -> Vec<PathBuf> {
    // Port: windows → `where`; POSIX → `which`.
    silent_command("where")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| PathBuf::from(l.trim()))
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}
