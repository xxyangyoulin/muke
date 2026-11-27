use anyhow::{Context, Result, bail};
use chrono::{SecondsFormat, Utc};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub data_dir: PathBuf,
    pub listen: SocketAddr,
    pub frontend: PathBuf,
    pub export_dir: PathBuf,
    pub timezone: chrono_tz::Tz,
    pub setup_hosts: Vec<String>,
    pub cookie_secure: bool,
    pub http_proxy: Option<String>,
    pub settings: BTreeMap<String, String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("var"),
            listen: "127.0.0.1:8785".parse().unwrap(),
            frontend: PathBuf::from("frontend"),
            export_dir: PathBuf::from("dist"),
            timezone: chrono_tz::Asia::Shanghai,
            setup_hosts: vec!["127.0.0.1".into(), "::1".into()],
            cookie_secure: false,
            http_proxy: None,
            settings: BTreeMap::new(),
        }
    }
}
impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let mut config: Self = if let Some(path) = path {
            toml::from_str(&std::fs::read_to_string(path).context("读取配置失败")?)
                .context("TOML 配置无效")?
        } else {
            Self::default()
        };
        if let Ok(value) = std::env::var("MKBLOG_DATA_DIR") {
            config.data_dir = value.into();
        }
        if let Ok(value) = std::env::var("MKBLOG_HTTP_PROXY") {
            config.http_proxy = Some(value);
        }
        if let Ok(value) = std::env::var("MKBLOG_SETUP_HOSTS") {
            config.setup_hosts = value.split(',').map(|s| s.trim().to_owned()).collect();
        }
        if let Ok(value) = std::env::var("MKBLOG_TIMEZONE") {
            config.timezone = value.parse().context("时区无效")?;
        }
        for (env, key) in [
            ("MKBLOG_SITE_URL", "site_url"),
            ("MKBLOG_SITE_NAME", "site_name"),
            ("MKBLOG_SITE_SUBTITLE", "site_subtitle"),
            ("MKBLOG_TAGLINE", "site_tagline"),
            ("MKBLOG_DESC", "site_description"),
            ("MKBLOG_LOCALE", "locale"),
        ] {
            if let Ok(value) = std::env::var(env) {
                config.settings.insert(key.into(), value);
            }
        }
        for host in &config.setup_hosts {
            if host != "*" && host.parse::<std::net::IpAddr>().is_err() {
                bail!("setup_hosts 必须为 IP 地址或 *");
            }
        }
        Ok(config)
    }
    pub fn now(&self) -> String {
        Utc::now()
            .with_timezone(&self.timezone)
            .to_rfc3339_opts(SecondsFormat::Secs, true)
    }
}

pub struct Settings(pub BTreeMap<String, String>);
impl Settings {
    pub async fn load(app: &crate::api::App) -> crate::error::Result<Self> {
        let mut values: BTreeMap<String, String> =
            serde_json::from_str(include_str!("defaults.json")).expect("shipped defaults");
        values.extend(app.config.settings.clone());
        values.extend(
            sqlx::query_as::<_, (String, String)>("SELECT key,value FROM settings")
                .fetch_all(&app.db)
                .await?,
        );
        Ok(Self(values))
    }
    pub fn get(&self, key: &str) -> &str {
        self.0.get(key).map(String::as_str).unwrap_or("")
    }
    pub fn number(&self, key: &str) -> i64 {
        self.get(key).parse().unwrap_or(0)
    }
    pub fn enabled(&self, key: &str) -> bool {
        matches!(
            self.get(key).to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    }
}
