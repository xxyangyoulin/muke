use anyhow::Result;
use std::{io::Write, path::Path, time::SystemTime};
use zip::write::SimpleFileOptions;

/// 目录最新文件的修改时间（空目录返回 UNIX epoch）。
pub fn newest_modified(source: &Path) -> std::io::Result<SystemTime> {
    let mut newest = SystemTime::UNIX_EPOCH;
    let mut directories = vec![source.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                directories.push(path);
            } else {
                let modified = entry.metadata()?.modified()?;
                if modified > newest {
                    newest = modified;
                }
            }
        }
    }
    Ok(newest)
}

/// 递归压缩目录到 target（先写临时文件再原子改名），条目路径相对 source。
pub fn zip_tree(source: &Path, target: &Path) -> Result<()> {
    if !std::fs::symlink_metadata(source)?.is_dir() {
        anyhow::bail!("压缩源必须是目录");
    }
    let temporary = target.with_extension("zip.tmp");
    let file = std::fs::File::create(&temporary)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let mut directories = vec![source.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(source)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type()?.is_dir() {
                directories.push(path);
            } else {
                writer.start_file(relative, options)?;
                let mut input = std::fs::File::open(&path)?;
                std::io::copy(&mut input, &mut writer)?;
                writer.flush()?;
            }
        }
    }
    let file = writer.finish()?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zips_tree_with_relative_entries_and_reuses_cache_time() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("site");
        std::fs::create_dir_all(source.join("assets").join("deep")).unwrap();
        std::fs::write(source.join("index.html"), b"<html>hello</html>").unwrap();
        std::fs::write(source.join("assets/style.css"), b"body{}").unwrap();
        std::fs::write(source.join("assets/deep/x.txt"), b"data").unwrap();
        let target = root.path().join("out.zip");
        zip_tree(&source, &target).unwrap();
        let file = std::fs::File::open(&target).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["assets/deep/x.txt", "assets/style.css", "index.html"]
        );
        let newest = newest_modified(&source).unwrap();
        assert!(newest > SystemTime::UNIX_EPOCH);
        let mut entry = archive.by_name("index.html").unwrap();
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut content).unwrap();
        assert_eq!(content, b"<html>hello</html>");
        assert!(zip_tree(&root.path().join("missing"), &target).is_err());
        assert!(!root.path().join("out.zip.tmp").exists());
    }
}
