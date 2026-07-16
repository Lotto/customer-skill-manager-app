//! Restart the Claude Desktop application, cross-platform.
//!
//! Claude Desktop caches its skill manifest, so after we publish new skills the
//! user may want to reload it. This kills the running Desktop processes and
//! relaunches the app. It deliberately never matches the Claude Code CLI,
//! node processes, or our own app.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use sysinfo::{Pid, ProcessesToUpdate, System};

/// Kill Claude Desktop and relaunch it. Returns a short status message.
pub fn reload() -> Result<String, String> {
    let current = sysinfo::get_current_pid().ok();

    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut pids: Vec<Pid> = Vec::new();
    let mut main_exe: Option<PathBuf> = None;
    let mut any_exe: Option<PathBuf> = None;

    for (pid, process) in sys.processes() {
        if Some(*pid) == current {
            continue;
        }
        let exe = process
            .exe()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !is_claude_desktop(&exe) {
            continue;
        }
        pids.push(*pid);
        if let Some(e) = process.exe() {
            any_exe.get_or_insert_with(|| e.to_path_buf());
        }
        // The Electron main process (the one to relaunch) has no `--type=` arg;
        // renderer/gpu/utility helpers do.
        let is_helper = process
            .cmd()
            .iter()
            .any(|a| a.to_string_lossy().starts_with("--type="));
        if !is_helper {
            if let Some(e) = process.exe() {
                main_exe.get_or_insert_with(|| e.to_path_buf());
            }
        }
    }

    // Not running: just launch it fresh.
    if pids.is_empty() {
        launch_fresh()?;
        return Ok("Claude Desktop lancé.".into());
    }

    let launch = main_exe
        .or(any_exe)
        .ok_or("Chemin de l'exécutable Claude Desktop introuvable.")?;

    let mut killed = 0usize;
    for pid in &pids {
        if let Some(p) = sys.process(*pid) {
            if p.kill() {
                killed += 1;
            }
        }
    }

    // Give the OS a moment to release the app before relaunching.
    std::thread::sleep(Duration::from_millis(1500));
    relaunch(&launch)?;

    Ok(format!(
        "Claude Desktop rechargé ({killed} processus arrêté·s)."
    ))
}

/// Launch Claude Desktop when no instance is running, by app identity (so it
/// works without a captured executable path).
#[cfg(target_os = "windows")]
fn launch_fresh() -> Result<(), String> {
    // Resolve the Store/app AUMID from the Start menu, then launch via explorer.
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-StartApps | Where-Object { $_.Name -eq 'Claude' } | Select-Object -First 1).AppID",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    let aumid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if aumid.is_empty() {
        return Err("Claude Desktop est introuvable sur ce système.".into());
    }
    Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{aumid}"))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_fresh() -> Result<(), String> {
    Command::new("open")
        .args(["-a", "Claude"])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch_fresh() -> Result<(), String> {
    // Try the packaged launcher (never the `claude` CLI).
    Command::new("claude-desktop")
        .spawn()
        .map_err(|_| "Claude Desktop est introuvable sur ce système.".to_string())?;
    Ok(())
}

/// Heuristic: is this executable path the Claude **Desktop** GUI app?
///
/// Crucially it must NEVER match the Claude Code CLI, which on Windows also
/// ships as `claude.exe` (at `…\.local\bin\claude.exe`). We therefore match only
/// GUI-app-specific install locations, and hard-exclude the CLI's `.local\bin`
/// path, node, and our own app.
fn is_claude_desktop(exe: &str) -> bool {
    if exe.is_empty()
        || exe.contains("customer-skill-manager")
        || exe.contains("claude-code")
        || exe.contains("node")
    {
        return false;
    }
    // The Claude Code CLI lives in `…/.local/bin/claude(.exe)` — never touch it.
    if exe.contains(".local") && exe.contains("bin") {
        return false;
    }
    exe.contains("claude.app")                                    // macOS bundle
        || exe.contains("anthropicclaude")                        // Windows installer
        || (exe.contains("windowsapps") && exe.contains("claude")) // Windows Store
        || exe.contains("claude-desktop")                         // Linux .deb/package
        || (exe.contains("claude") && exe.ends_with(".appimage")) // Linux AppImage
}

#[cfg(target_os = "macos")]
fn relaunch(exe: &Path) -> Result<(), String> {
    // Open the .app bundle (e.g. /Applications/Claude.app) rather than the inner
    // binary, so macOS launches it as a proper app.
    let app = app_bundle(exe).unwrap_or_else(|| exe.to_path_buf());
    Command::new("open")
        .arg(&app)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let s = exe.to_string_lossy();
    let idx = s.find(".app")?;
    Some(PathBuf::from(&s[..idx + 4]))
}

#[cfg(not(target_os = "macos"))]
fn relaunch(exe: &Path) -> Result<(), String> {
    Command::new(exe).spawn().map_err(|e| e.to_string())?;
    Ok(())
}
