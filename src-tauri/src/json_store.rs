use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const MAX_JSON_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) struct JsonStore;

impl JsonStore {
    pub(crate) fn ensure_object(path: &Path) -> anyhow::Result<()> {
        if !path.exists() {
            Self::write_atomic(path, &serde_json::json!({}))?;
        }
        Ok(())
    }

    pub(crate) fn read_or_default<T>(path: &Path, default: impl FnOnce() -> T) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        if !path.exists() {
            return Ok(default());
        }
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_JSON_BYTES {
            anyhow::bail!("JSON 数据文件超过 32 MB：{}", path.display());
        }
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes)
            .map_err(|error| anyhow::anyhow!("JSON 数据文件损坏（{}）：{error}", path.display()))
    }

    pub(crate) fn write_atomic<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(value)?;
        Self::write_atomic_bytes(path, &bytes)
    }

    pub(crate) fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        let same_length =
            fs::metadata(path).is_ok_and(|metadata| metadata.len() == bytes.len() as u64);
        if same_length && fs::read(path).is_ok_and(|current| current == bytes) {
            secure_file(path)?;
            return Ok(());
        }
        let temporary = temporary_path(path);
        {
            let mut file = create_private_file(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        if let Err(error) = replace_file(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        secure_file(path)?;
        Ok(())
    }
}

pub(crate) fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("data.json");
    path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()))
}

fn create_private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(unix)]
fn secure_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    fs::rename(source, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn identical_write_keeps_existing_file() {
        use std::os::unix::fs::MetadataExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.json");
        JsonStore::write_atomic(&path, &serde_json::json!({ "value": 1 })).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();

        JsonStore::write_atomic(&path, &serde_json::json!({ "value": 1 })).unwrap();

        assert_eq!(fs::metadata(path).unwrap().ino(), inode);
    }
}
