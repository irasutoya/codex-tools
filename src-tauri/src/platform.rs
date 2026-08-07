use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

pub fn codex_cli_command() -> Command {
    Command::new(codex_cli_executable())
}

fn codex_cli_executable() -> OsString {
    if let Some(configured) = std::env::var_os("CODEX_BIN")
        && !configured.is_empty()
    {
        return configured;
    }

    let program = if cfg!(windows) { "codex.exe" } else { "codex" };
    if let Some(path) = executable_on_path(program) {
        return path.into_os_string();
    }

    #[cfg(target_os = "macos")]
    if let Some(path) = macos_codex_candidates()
        .into_iter()
        .find(|path| is_executable(path))
    {
        return path.into_os_string();
    }

    program.into()
}

fn executable_on_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|path| path.join(program))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(target_os = "macos")]
fn macos_codex_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/codex"));
        candidates.push(home.join(".npm-global/bin/codex"));
        candidates.push(home.join(".volta/bin/codex"));
        candidates.push(home.join("Library/pnpm/codex"));
        candidates.push(home.join(".bun/bin/codex"));
        if let Ok(versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            candidates.extend(
                versions
                    .flatten()
                    .map(|entry| entry.path().join("bin/codex")),
            );
        }
    }
    candidates.extend([
        PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
        PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
    ]);
    candidates
}

pub fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("rundll32.exe");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("/usr/bin/open");
        command.arg(url);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "系统无法打开链接（退出状态：{status}）"
        )))
    }
}

/// 找到 Codex 桌面应用：优先使用手动配置的路径（.app 目录或可执行文件），
/// 否则自动检测（macOS 返回 `.app` 目录，Windows 返回可执行文件路径）。
pub fn codex_app_path(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = configured.and_then(normalize_app_path) {
        return Some(path);
    }
    #[cfg(target_os = "macos")]
    {
        macos_codex_app_path()
    }
    #[cfg(windows)]
    {
        windows_codex_exe()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        None
    }
}

/// 规范化手动配置的应用路径：只接受存在的 `.app` 目录或可执行文件。
fn normalize_app_path(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path.trim());
    if path.exists() { Some(path) } else { None }
}

pub fn codex_app_found(configured: Option<&str>) -> bool {
    codex_app_path(configured).is_some()
}

/// Codex 桌面应用当前是否在运行（用于提示“需要重启才能带调试端口启动”）。
pub fn codex_app_running(configured: Option<&str>) -> bool {
    #[cfg(target_os = "macos")]
    {
        let path = codex_app_path(configured);
        if path.as_deref().is_some_and(path_running) {
            return true;
        }
        ["Codex", "ChatGPT"].into_iter().any(process_named_running)
    }
    #[cfg(windows)]
    {
        let path = codex_app_path(configured);
        if path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(process_named_running)
        {
            return true;
        }
        ["Codex.exe", "ChatGPT.exe"]
            .into_iter()
            .any(process_named_running)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        false
    }
}

/// 退出正在运行的 Codex 桌面应用（macOS 优雅退出，Windows 强制结束）。
pub fn quit_codex_app(configured: Option<&str>) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let path = codex_app_path(configured);
        // 手动配置的是可执行文件时，无法用 osascript 优雅退出，回退 pkill。
        if let Some(path) = path.as_deref().filter(|path| !path.is_dir()) {
            let _ = Command::new("/usr/bin/pkill")
                .args(["-f", &path.display().to_string()])
                .status();
            return Ok(());
        }
        let name = path
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .unwrap_or("Codex")
            .to_string();
        let status = Command::new("/usr/bin/osascript")
            .args(["-e", &format!("tell application \"{name}\" to quit")])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                "macOS 未能优雅退出 Codex，请手动关闭后再试。",
            ))
        }
    }
    #[cfg(windows)]
    {
        // 先按手动配置的可执行文件名结束（与 codex_app_running 的识别逻辑一致），
        // 再兜底结束默认名称的进程。
        let mut names = vec!["Codex.exe", "ChatGPT.exe"];
        let configured_path = codex_app_path(configured);
        if let Some(name) = configured_path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .filter(|name| !names.contains(name))
        {
            names.push(name);
        }
        for name in names {
            let _ = Command::new("taskkill").args(["/IM", name, "/F"]).status();
        }
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Err(std::io::Error::other("当前系统暂不支持自动重启 Codex。"))
    }
}

/// 以调试模式启动 Codex 桌面应用：传入 `--remote-debugging-port` 和
/// `--remote-allow-origins`，使本应用可以通过 CDP 注入解锁脚本。
/// `configured` 为手动指定的 `.app` 目录或可执行文件路径。
pub fn launch_codex_app_with_debug(port: u16, configured: Option<&str>) -> std::io::Result<()> {
    let debug_args = [
        format!("--remote-debugging-port={port}"),
        format!("--remote-allow-origins=http://127.0.0.1:{port}"),
    ];
    #[cfg(target_os = "macos")]
    {
        let path = codex_app_path(configured);
        if let Some(path) = path.as_deref().filter(|path| !path.is_dir()) {
            // 手动指定的可执行文件：直接运行。
            Command::new(path).args(&debug_args).spawn().map(|_| ())
        } else {
            let app = path.as_deref().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "未找到 Codex 桌面应用")
            })?;
            let status = Command::new("/usr/bin/open")
                .arg("-a")
                .arg(app)
                .arg("--args")
                .args(&debug_args)
                .status()?;
            if status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other(format!(
                    "无法启动 Codex（退出状态：{status}）"
                )))
            }
        }
    }
    #[cfg(windows)]
    {
        let exe = codex_app_path(configured).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "未找到 Codex 桌面应用")
        })?;
        Command::new(exe).args(&debug_args).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Err(std::io::Error::other("当前系统暂不支持自动启动 Codex。"))
    }
}

#[cfg(target_os = "macos")]
fn macos_codex_app_path() -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/Applications/Codex.app"),
        PathBuf::from("/Applications/ChatGPT.app"),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Applications/Codex.app"));
        candidates.push(home.join("Applications/ChatGPT.app"));
    }
    candidates.into_iter().find(|path| path.is_dir())
}

#[cfg(target_os = "macos")]
fn process_named_running(name: &str) -> bool {
    Command::new("/usr/bin/pgrep")
        .args(["-x", name])
        .output()
        .ok()
        .is_some_and(|output| output.status.success())
}

#[cfg(target_os = "macos")]
fn path_running(path: &Path) -> bool {
    // .app 目录按可执行文件路径匹配；可执行文件按自身路径匹配。
    let needle = if path.is_dir() {
        path.join("Contents/MacOS").to_string_lossy().to_string()
    } else {
        path.display().to_string()
    };
    Command::new("/usr/bin/pgrep")
        .args(["-f", &needle])
        .output()
        .ok()
        .is_some_and(|output| output.status.success())
}

#[cfg(windows)]
fn windows_codex_exe() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let local = Path::new(&local);
    [
        local.join("Programs/OpenAI/Codex/Codex.exe"),
        local.join("Programs/Codex/Codex.exe"),
        local.join("Programs/ChatGPT/ChatGPT.exe"),
        local.join("OpenAI/Codex/bin/codex.exe"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

#[cfg(windows)]
fn process_named_running(name: &str) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {name}"), "/NH"])
        .output()
        .ok()
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).contains(name))
}

pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "Mac OS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        std::env::consts::OS
    }
}

pub fn os_version() -> String {
    #[cfg(windows)]
    return windows_version();

    #[cfg(target_os = "macos")]
    return command_version("/usr/bin/sw_vers", &["-productVersion"]);

    #[cfg(all(unix, not(target_os = "macos")))]
    return command_version("uname", &["-r"]);

    #[allow(unreachable_code)]
    "unknown".into()
}

#[cfg(not(windows))]
fn command_version(program: impl AsRef<Path>, args: &[&str]) -> String {
    Command::new(program.as_ref())
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(windows)]
fn windows_version() -> String {
    #[repr(C)]
    struct RtlOsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform_id: u32,
        service_pack: [u16; 128],
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version_information: *mut RtlOsVersionInfo) -> i32;
    }

    let mut version = RtlOsVersionInfo {
        size: std::mem::size_of::<RtlOsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform_id: 0,
        service_pack: [0; 128],
    };
    // SAFETY: `version` uses the documented RTL_OSVERSIONINFOW layout and
    // remains valid for the duration of the synchronous ntdll call.
    let status = unsafe { RtlGetVersion(&mut version) };
    if status >= 0 && version.major > 0 {
        format!("{}.{}.{}", version.major, version.minor, version.build)
    } else {
        "unknown".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_supported_platform_name() {
        assert!(!os_name().is_empty());
        assert!(!os_version().is_empty());
    }
}
