use anyhow::{Context, Result, bail};
use std::{
    ffi::CString,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

pub struct Artifact {
    directory: TempDir,
    destination: PathBuf,
}

impl Artifact {
    /// destination 为最终导出目录；暂存目录建在其父目录内，保证同一文件系统内原子切换。
    pub fn new(destination: &Path) -> Result<Self> {
        let root = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(root)?;
        if !std::fs::symlink_metadata(root)?.is_dir() {
            bail!("导出目录不能是符号链接");
        }
        Ok(Self {
            directory: tempfile::Builder::new()
                .prefix(".export-")
                .tempdir_in(root)?,
            destination: destination.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    pub fn publish(self, stop: &CancellationToken) -> Result<PathBuf> {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let exists = match std::fs::symlink_metadata(&self.destination) {
            Ok(metadata) if metadata.is_dir() => true,
            Ok(_) => bail!("已有导出产物必须是目录"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if exists {
            let source = CString::new(self.directory.path().as_os_str().as_bytes())?;
            let destination = CString::new(self.destination.as_os_str().as_bytes())?;
            // Both directories are on the same filesystem; exchange keeps dist continuously visible.
            let result = unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    source.as_ptr(),
                    libc::AT_FDCWD,
                    destination.as_ptr(),
                    libc::RENAME_EXCHANGE,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error()).context("无法切换导出产物");
            }
            if let Err(error) = self.directory.close() {
                // Publication has committed; cleanup must not turn a successful export into a retry.
                tracing::warn!(%error, "上一版导出暂存目录清理失败");
            }
        } else {
            std::fs::rename(self.directory.path(), &self.destination)
                .context("无法发布导出产物")?;
            // 暂存目录已整体改名，交给目标路径，无需再按暂存路径清理。
            let _ = self.directory.keep();
        }
        Ok(self.destination)
    }
}

pub fn recover(root: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !std::fs::symlink_metadata(root)?.is_dir() {
        bail!("导出目录不能是符号链接");
    }
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(".export-")
            && entry.file_type()?.is_dir()
        {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_cancel_and_restart_preserve_committed_version() {
        let root = tempfile::tempdir().unwrap();
        let stop = CancellationToken::new();
        let destination = root.path().join("dist");
        let first = Artifact::new(&destination).unwrap();
        std::fs::write(first.path().join("index.html"), "first").unwrap();
        assert_eq!(first.publish(&stop).unwrap(), destination);
        assert_eq!(
            std::fs::read_to_string(destination.join("index.html")).unwrap(),
            "first"
        );
        let cancelled = Artifact::new(&destination).unwrap();
        std::fs::write(cancelled.path().join("index.html"), "cancelled").unwrap();
        let cancelled_path = cancelled.path().to_owned();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(cancelled.publish(&cancellation).is_err());
        assert!(!cancelled_path.exists());
        assert_eq!(
            std::fs::read_to_string(destination.join("index.html")).unwrap(),
            "first"
        );
        let broken = Artifact::new(&destination).unwrap();
        std::fs::remove_dir(broken.path()).unwrap();
        assert!(broken.publish(&stop).is_err());
        assert_eq!(
            std::fs::read_to_string(destination.join("index.html")).unwrap(),
            "first"
        );
        let next = Artifact::new(&destination).unwrap();
        std::fs::write(next.path().join("index.html"), "next").unwrap();
        next.publish(&stop).unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("index.html")).unwrap(),
            "next"
        );
        let abandoned = root.path().join(".export-abandoned");
        std::fs::create_dir(&abandoned).unwrap();
        std::fs::write(abandoned.join("index.html"), "old or incomplete").unwrap();
        recover(root.path()).unwrap();
        assert!(!abandoned.exists());
        assert_eq!(
            std::fs::read_to_string(destination.join("index.html")).unwrap(),
            "next"
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_destination_links_and_failed_publication_preserves_files() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("index.html"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("dist")).unwrap();
        let artifact = Artifact::new(&root.path().join("dist")).unwrap();
        let stage = artifact.path().to_owned();
        assert!(artifact.publish(&CancellationToken::new()).is_err());
        assert!(!stage.exists());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("index.html")).unwrap(),
            "outside"
        );
        let link = root.path().join(".export-link");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        recover(root.path()).unwrap();
        assert!(link.is_symlink());
        assert!(outside.path().join("index.html").exists());
    }
}
