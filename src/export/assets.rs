use crate::plugins;
use anyhow::{Context, Result, bail};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub async fn copy(frontend: &Path, destination: &Path, stop: &CancellationToken) -> Result<()> {
    let frontend = frontend.to_owned();
    let destination = destination.to_owned();
    let stop = stop.clone();
    tokio::task::spawn_blocking(move || {
        if !std::fs::symlink_metadata(&frontend)?.is_dir() {
            bail!("前端目录不能是符号链接");
        }
        let assets = frontend.join("assets");
        for name in ["app.js", "registry.js", "style.css"] {
            if !std::fs::symlink_metadata(assets.join(name))?.is_file() {
                bail!("缺少前端资源：{name}");
            }
        }
        copy_tree(&assets, &destination.join("assets"), true, &stop)?;
        let root = frontend
            .parent()
            .context("前端目录缺少父目录")?
            .join("plugins");
        if !std::fs::symlink_metadata(&root)?.is_dir() {
            bail!("插件目录不能是符号链接");
        }
        for (id, source) in plugins::frontend_directories(&root) {
            if !std::fs::symlink_metadata(root.join(id))?.is_dir() {
                bail!("插件目录不能是符号链接：{id}");
            }
            for field in ["module", "style"] {
                if let Some(file) = plugins::get(id)?.manifest()["frontend"][field].as_str()
                    && !std::fs::symlink_metadata(source.join(file))?.is_file()
                {
                    bail!("缺少插件前端资源：{id}/{file}");
                }
            }
            copy_tree(&source, &destination.join("plugins").join(id), false, &stop)?;
        }
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        Ok(())
    })
    .await?
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    core_assets: bool,
    stop: &CancellationToken,
) -> Result<()> {
    let mut directories = vec![(source.to_owned(), destination.to_owned())];
    while let Some((directory, target)) = directories.pop() {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        if !std::fs::symlink_metadata(&directory)?.is_dir() {
            bail!("资源目录不能是符号链接");
        }
        std::fs::create_dir_all(&target)?;
        for entry in std::fs::read_dir(&directory)? {
            if stop.is_cancelled() {
                bail!("任务已取消");
            }
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            if core_assets
                && directory == source
                && matches!(name.to_str(), Some("content.json" | "articles.json"))
            {
                continue;
            }
            let kind = entry.file_type()?;
            let target = target.join(&name);
            if kind.is_dir() {
                directories.push((entry.path(), target));
            } else if kind.is_file() {
                let mut input = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(entry.path())?;
                use std::os::unix::fs::OpenOptionsExt;
                if !input.metadata()?.is_file() {
                    bail!("静态资源必须是普通文件");
                }
                let mut output = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)?;
                std::io::copy(&mut input, &mut output)?;
            } else {
                bail!(
                    "静态资源不能是符号链接或特殊文件：{}",
                    entry.path().display()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn packages_frontend_resources_without_backend_or_generated_json_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let frontend = root.path().join("frontend");
        let assets = frontend.join("assets");
        std::fs::create_dir_all(assets.join("lib")).unwrap();
        for name in ["app.js", "registry.js", "style.css", "lib/import.js"] {
            std::fs::write(assets.join(name), name).unwrap();
        }
        std::fs::write(assets.join("content.json"), "stale content").unwrap();
        std::fs::write(assets.join("articles.json"), "stale articles").unwrap();
        std::fs::write(assets.join(".secret"), "private").unwrap();
        std::fs::write(frontend.join("server.rs"), "backend").unwrap();
        let plugins = root.path().join("plugins");
        for (_, directory) in plugins::frontend_directories(&plugins) {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("plugin.js"), "plugin").unwrap();
            std::fs::write(directory.join("plugin.css"), "style").unwrap();
            let backend = directory.parent().unwrap().join("backend");
            std::fs::create_dir_all(&backend).unwrap();
            std::fs::write(backend.join("mod.rs"), "backend").unwrap();
        }
        let destination = root.path().join("output");
        std::fs::create_dir_all(destination.join("assets")).unwrap();
        std::fs::write(destination.join("assets/content.json"), "fresh content").unwrap();
        std::fs::write(destination.join("assets/articles.json"), "fresh articles").unwrap();
        let stop = CancellationToken::new();
        copy(&frontend, &destination, &stop).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("assets/content.json")).unwrap(),
            "fresh content"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("assets/articles.json")).unwrap(),
            "fresh articles"
        );
        assert!(destination.join("assets/lib/import.js").is_file());
        assert!(destination.join("plugins/rss/plugin.js").is_file());
        assert!(!destination.join("plugins/rss/backend").exists());
        assert!(!destination.join("assets/.secret").exists());
        assert!(!destination.join("server.rs").exists());
        std::os::unix::fs::symlink(frontend.join("server.rs"), assets.join("leak.js")).unwrap();
        assert!(
            copy(&frontend, &root.path().join("bad"), &stop)
                .await
                .is_err()
        );
        assert!(!root.path().join("bad/assets/leak.js").exists());
        stop.cancel();
        assert!(
            copy(&frontend, &root.path().join("cancelled"), &stop)
                .await
                .is_err()
        );
        assert!(!root.path().join("cancelled").exists());
    }
}
