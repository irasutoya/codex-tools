use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

pub fn codex_command() -> Command {
    Command::new(codex_executable())
}

fn codex_executable() -> OsString {
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

    command.spawn().map(|_| ())
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
