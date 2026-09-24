//! Per-platform auto-start on login. `install()` / `uninstall()` are invoked
//! by the `--install` / `--uninstall` CLI flags.

use std::path::PathBuf;

const LABEL: &str = "com.hallygtree.trayce";

/// Absolute path to the executable that auto-start should launch.
fn exe_path() -> Option<PathBuf> {
    match std::env::current_exe() {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("could not resolve the executable path: {e}");
            None
        }
    }
}

/// Home directory, or `None` with a printed message (used by macOS/Linux).
#[cfg(not(target_os = "windows"))]
fn home() -> Option<PathBuf> {
    match dirs::home_dir() {
        Some(h) => Some(h),
        None => {
            eprintln!("could not resolve the home directory");
            None
        }
    }
}

#[cfg(target_os = "macos")]
fn current_uid() -> u32 {
    // `id -u` avoids a libc dependency just for getuid().
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
pub fn install() {
    use std::process::Command;

    let (Some(home), Some(exe)) = (home(), exe_path()) else {
        return;
    };
    let plist_dir = home.join("Library/LaunchAgents");
    if let Err(e) = std::fs::create_dir_all(&plist_dir) {
        eprintln!("could not create {}: {e}", plist_dir.display());
        return;
    }
    let plist_path = plist_dir.join(format!("{LABEL}.plist"));
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key><array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
</dict>
</plist>
"#,
        exe.display()
    );
    if let Err(e) = std::fs::write(&plist_path, plist) {
        eprintln!("could not write {}: {e}", plist_path.display());
        return;
    }
    if let Some(p) = plist_path.to_str() {
        let uid = format!("gui/{}", current_uid());
        let _ = Command::new("launchctl")
            .args(["bootstrap", &uid, p])
            .status();
    }
    println!("installed: {}", plist_path.display());
}

#[cfg(target_os = "macos")]
pub fn uninstall() {
    let Some(home) = home() else {
        return;
    };
    let plist_path = home
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"));
    if let Some(p) = plist_path.to_str() {
        let uid = format!("gui/{}", current_uid());
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &uid, p])
            .status();
    }
    let _ = std::fs::remove_file(&plist_path);
    println!("uninstalled");
}

#[cfg(target_os = "linux")]
pub fn install() {
    let (Some(home), Some(exe)) = (home(), exe_path()) else {
        return;
    };
    let dir = home.join(".config/autostart");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("could not create {}: {e}", dir.display());
        return;
    }
    let path = dir.join("trayce.desktop");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Trayce\nExec={}\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    if let Err(e) = std::fs::write(&path, entry) {
        eprintln!("could not write {}: {e}", path.display());
        return;
    }
    println!("installed: {}", path.display());
}

#[cfg(target_os = "linux")]
pub fn uninstall() {
    let Some(home) = home() else {
        return;
    };
    let path = home.join(".config/autostart/trayce.desktop");
    let _ = std::fs::remove_file(&path);
    println!("uninstalled");
}

#[cfg(target_os = "windows")]
pub fn install() {
    let Some(exe) = exe_path() else {
        return;
    };
    // HKCU Run key — value name = LABEL, value data = exe path.
    let status = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            LABEL,
            "/t",
            "REG_SZ",
            "/d",
            &exe.display().to_string(),
            "/f",
        ])
        .status();
    match status {
        Ok(s) if s.success() => println!("installed: HKCU Run\\{LABEL}"),
        _ => eprintln!("install failed — could not write the registry Run key"),
    }
}

#[cfg(target_os = "windows")]
pub fn uninstall() {
    let _ = std::process::Command::new("reg")
        .args([
            "delete",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            LABEL,
            "/f",
        ])
        .status();
    println!("uninstalled");
}
