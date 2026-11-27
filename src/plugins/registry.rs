use super::{Plugin, dms, douban, rss, wallpapers, weibo};
use crate::error::{Error, Result};
use std::sync::{Arc, RwLock};
use tokio::sync::{Mutex, OwnedMutexGuard};

#[derive(Clone)]
struct Entry {
    plugin: Arc<dyn Plugin>,
    gate: Arc<Mutex<()>>,
}

#[derive(Clone)]
pub struct Registry(Arc<RwLock<Vec<Entry>>>);

impl Default for Registry {
    fn default() -> Self {
        let registry = Self(Arc::new(RwLock::new(Vec::new())));
        for plugin in [
            Arc::new(rss::Rss) as Arc<dyn Plugin>,
            Arc::new(dms::Dms),
            Arc::new(weibo::Weibo),
            Arc::new(douban::Douban),
            Arc::new(wallpapers::Wallpapers),
        ] {
            registry.insert(plugin);
        }
        registry
    }
}

impl Registry {
    pub fn snapshot(&self) -> Self {
        Self(Arc::new(RwLock::new(self.0.read().unwrap().clone())))
    }

    pub fn all(&self) -> Vec<Arc<dyn Plugin>> {
        self.0
            .read()
            .unwrap()
            .iter()
            .map(|entry| entry.plugin.clone())
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<Arc<dyn Plugin>> {
        self.0
            .read()
            .unwrap()
            .iter()
            .find(|entry| entry.plugin.id() == id)
            .map(|entry| entry.plugin.clone())
            .ok_or_else(|| Error::not_found("插件不存在"))
    }

    pub(super) fn insert(&self, plugin: Arc<dyn Plugin>) {
        let mut entries = self.0.write().unwrap();
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.plugin.id() == plugin.id())
        {
            entry.plugin = plugin;
        } else {
            entries.push(Entry {
                plugin,
                gate: Arc::new(Mutex::new(())),
            });
        }
    }

    pub(super) fn gate(&self, id: &str) -> Result<Arc<Mutex<()>>> {
        self.0
            .read()
            .unwrap()
            .iter()
            .find(|entry| entry.plugin.id() == id)
            .map(|entry| entry.gate.clone())
            .ok_or_else(|| Error::not_found("插件不存在"))
    }

    pub fn mutation_guard(&self, id: &str) -> Result<OwnedMutexGuard<()>> {
        self.gate(id)?.try_lock_owned().map_err(|_| {
            Error::Http(
                axum::http::StatusCode::CONFLICT,
                "插件正在执行任务，请等待完成或先取消任务".into(),
            )
        })
    }
}
