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
use std::path::PathBuf;

/// Per-user root for shell-owned state: backend clone, staged browser
/// copy, updater status. Per-user (never Program Files) so backend
/// updates never need elevation.
pub fn app_data_dir() -> Option<PathBuf> {
    // Port: windows → LOCALAPPDATA; macos → home + Library/Application
    // Support; linux → $XDG_DATA_HOME or home + .local/share.
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|base| std::path::Path::new(&base).join("dsh-desktop"))
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

/// Dev-only shim sitting next to the shell exe (never shipped).
pub fn dev_shim_file(exe_dir: &std::path::Path) -> PathBuf {
    exe_dir.join(format!("dsh.{}", script_ext()))
}
