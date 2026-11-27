use super::{Plugin, import};
use crate::{api::App, config::Settings, tasks};
use async_trait::async_trait;
use chrono::{NaiveDate, TimeZone, Utc};
use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::LazyLock;

pub struct Douban;
static MANIFEST: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("manifest.json")).unwrap());
static SUBJECT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^https?://movie\.douban\.com/subject/(\d+)/?(?:[?#].*)?$").unwrap()
});
static PLACEHOLDER: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\{(title|year|rating|date|url|category)\}").unwrap());

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Sync {
    all: bool,
}
struct Movie {
    id: String,
    title: String,
    year: String,
    intro: String,
    rating: String,
    comment: String,
    date: String,
    cover: String,
}
fn text(node: ElementRef<'_>, selector: &str) -> String {
    node.select(&Selector::parse(selector).unwrap())
        .next()
        .map(|n| {
            n.text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}
fn parse(html: &[u8]) -> anyhow::Result<Vec<Movie>> {
    let document = Html::parse_document(std::str::from_utf8(html)?);
    let mut movies = Vec::new();
    for node in document.select(&Selector::parse(".item.comment-item").unwrap()) {
        let Some(id) = node
            .select(&Selector::parse("a[href]").unwrap())
            .find_map(|n| {
                SUBJECT
                    .captures(n.value().attr("href").unwrap())
                    .map(|m| m[1].to_owned())
            })
        else {
            continue;
        };
        let title = text(node, ".title a");
        if title.is_empty() {
            continue;
        }
        let intro = text(node, ".intro");
        let year = intro
            .get(..5)
            .filter(|s| s.ends_with('-') && s[..4].bytes().all(|c| c.is_ascii_digit()))
            .map(|s| s[..4].to_owned())
            .unwrap_or_default();
        let rating = node
            .select(&Selector::parse("[class]").unwrap())
            .flat_map(|n| n.value().classes())
            .find_map(|class| {
                let n = class
                    .strip_prefix("rating")?
                    .strip_suffix("-t")?
                    .parse::<usize>()
                    .ok()?;
                (n <= 5).then(|| format!("{}{}", "★".repeat(n), "☆".repeat(5 - n)))
            })
            .unwrap_or_default();
        let date = text(node, ".date");
        let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map(|d| d.to_string())
            .unwrap_or_default();
        let cover = node
            .select(&Selector::parse(".pic img[src]").unwrap())
            .next()
            .and_then(|n| n.value().attr("src"))
            .unwrap_or("")
            .into();
        movies.push(Movie {
            id,
            title,
            year,
            intro,
            rating,
            comment: text(node, ".comment"),
            date,
            cover,
        });
    }
    Ok(movies)
}
fn post(movie: Movie, config: &Settings, timezone: chrono_tz::Tz) -> import::Post {
    let category = config.get("category");
    let label = match category {
        "wish" => "想看",
        "do" => "在看",
        _ => "看过",
    };
    let link = format!("https://movie.douban.com/subject/{}/", movie.id);
    let content = PLACEHOLDER
        .replace_all(
            config.get("post_template"),
            |captures: &regex::Captures<'_>| {
                match &captures[1] {
                    "title" => movie.title.as_str(),
                    "year" => &movie.year,
                    "rating" => &movie.rating,
                    "date" => &movie.date,
                    "url" => &link,
                    "category" => label,
                    _ => unreachable!(),
                }
                .to_owned()
            },
        )
        .trim()
        .to_owned();
    let created = NaiveDate::parse_from_str(&movie.date, "%Y-%m-%d")
        .ok()
        .and_then(|d| {
            timezone
                .from_local_datetime(&d.and_hms_opt(21, 0, 0).unwrap())
                .earliest()
        })
        .unwrap_or_else(|| Utc::now().with_timezone(&timezone))
        .to_rfc3339();
    import::Post {
        key: format!("douban:{category}:{}", movie.id),
        content,
        created_at: created,
        link,
        images: Vec::new(),
        meta: json!({"card":"movie","title":movie.title,"year":movie.year,"intro":movie.intro,"rating":movie.rating,"comment":movie.comment,"date":movie.date,"category":label}),
    }
}

#[async_trait]
impl Plugin for Douban {
    fn render_post(&self, post: &Value) -> Option<String> {
        use crate::export::pages::{external_url, text};
        let meta = &post["meta"];
        let link = post["link"]
            .as_str()
            .and_then(external_url)
            .unwrap_or_else(|| "#".into());
        let mut html =
            format!(r#"<a class="post-movie-card" href="{link}" target="_blank" rel="noopener">"#);
        if let Some(cover) = post["images"].as_array().and_then(|images| images.first()) {
            html.push_str(&format!(
                r#"<img src="{}" alt="{}" loading="lazy">"#,
                text(cover, "thumb"),
                text(meta, "title")
            ));
        }
        html.push_str(&format!(
            r#"<div class="post-movie-info"><strong>{}</strong>"#,
            text(meta, "title")
        ));
        for (field, class) in [
            ("intro", "movie-intro"),
            ("rating", "movie-stars"),
            ("date", "movie-date"),
            ("category", "movie-category"),
            ("comment", "movie-comment"),
        ] {
            if meta[field].as_str().is_some_and(|value| !value.is_empty()) {
                html.push_str(&format!(
                    r#"<span class="{class}">{}</span>"#,
                    text(meta, field)
                ));
            }
        }
        html.push_str("</div></a>");
        Some(html)
    }
    fn id(&self) -> &'static str {
        "douban"
    }
    fn manifest(&self) -> &'static Value {
        &MANIFEST
    }
    async fn sync(&self, app: &App, ctx: &tasks::Context, params: Value) -> anyhow::Result<Value> {
        let options: Sync = serde_json::from_value(params)?;
        let config = super::config(self, &Settings::load(app).await?);
        let limit = if options.all {
            config.number("history_limit") as usize
        } else {
            30
        };
        let mut seen = 0;
        let mut added = 0;
        for page in 0..20 {
            ctx.check()?;
            let url = format!(
                "https://movie.douban.com/people/{}/{}?start={}&count=30&sortby=time",
                config.get("user_id"),
                config.get("category"),
                page * 30
            );
            let html = app
                .network
                .get_with_referer(
                    &url,
                    false,
                    8 * 1024 * 1024,
                    &ctx.stop,
                    Some("https://movie.douban.com/"),
                )
                .await?;
            let movies = parse(&html)?;
            if movies.is_empty() {
                break;
            }
            for movie in movies.into_iter().take(limit - seen) {
                ctx.check()?;
                seen += 1;
                let cover = movie.cover.clone();
                let mut post = post(movie, &config, app.config.timezone);
                if import::known(app, self.id(), &post.key).await? {
                    continue;
                }
                if !cover.is_empty()
                    && let Ok(name) =
                        import::fetch_image(app, ctx, &cover, 0, "https://movie.douban.com/").await
                {
                    post.images.push(name);
                }
                added += import::save(app, ctx, self.id(), post).await?;
                ctx.progress("同步豆瓣", seen as i64, Some(limit as i64))
                    .await?;
            }
            if seen >= limit {
                break;
            }
        }
        ctx.check()?;
        Ok(json!({"added":added}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, database, network};
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };
    use tokio_util::sync::CancellationToken;

    fn movie(id: usize, cover: &str) -> String {
        format!(
            r#"<div class="item comment-item"><div class="pic"><a href="https://movie.douban.com/subject/{id}/"><img src="{cover}"></a></div><ul><li class="title"><a href="https://movie.douban.com/subject/{id}/">电影 &amp; {id} {{rating}}</a></li><li class="intro">2020-01-01 / 中国 / 导演</li><li><span class="rating4-t"></span><span class="date">2026-11-26</span></li><li><span class="comment"> 很好 &amp; 好看 </span></li></ul></div>"#
        )
    }
    fn page(category: &str, start: usize) -> String {
        format!(
            "https://movie.douban.com/people/test/{category}?start={start}&count=30&sortby=time"
        )
    }
    #[test]
    fn parsing_templates_and_timezones() {
        let config = Settings(BTreeMap::from([
            ("category".into(), "wish".into()),
            (
                "post_template".into(),
                "{category}《{title}》{year} {rating} {date} {url} {unknown}".into(),
            ),
        ]));
        let mut movies = parse(movie(123, "http://example.org/cover").as_bytes()).unwrap();
        assert_eq!(movies[0].cover, "http://example.org/cover");
        let post = post(movies.remove(0), &config, chrono_tz::Asia::Shanghai);
        assert_eq!(post.key, "douban:wish:123");
        assert_eq!(
            post.content,
            "想看《电影 & 123 {rating}》2020 ★★★★☆ 2026-11-26 https://movie.douban.com/subject/123/ {unknown}"
        );
        assert_eq!(post.meta["comment"], "很好 & 好看");
        assert_eq!(post.meta["card"], "movie");
        assert_eq!(post.created_at, "2026-11-26T21:00:00+08:00");
        let broken = movie(1, "")
            .replace("2026-11-26", "2026-02-30")
            .replace("rating4-t", "rating9-t")
            .replace("2020-01-01", "标题信息");
        let movies = parse(broken.as_bytes()).unwrap();
        assert_eq!(movies[0].date, "");
        assert_eq!(movies[0].year, "");
        assert_eq!(movies[0].rating, "");
        assert!(
            parse(
                movie(1, "")
                    .replace("movie.douban.com/subject/", "evil.example/subject/")
                    .as_bytes()
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            parse(b"<html><body>Empty list</body></html>")
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn sync_pagination_limits_tombstones_and_media_rollback() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let work = root.path().join("work");
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let mut client = network::Client::new(None);
        client.fixture = Some(fixture.clone());
        let app = App {
            plugins: crate::plugins::Registry::default(),
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&work).await.unwrap(),
            network: client,
            data,
            config: Arc::new(Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        crate::plugins::migrate(&app).await.unwrap();
        for (key, value) in [
            ("user_id", "test"),
            ("history_limit", "35"),
            ("enabled", "1"),
        ] {
            sqlx::query("INSERT INTO settings(key,value) VALUES(?,?)")
                .bind(format!("plugin:douban:{key}"))
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
        let ctx = tasks::Context {
            id: 1,
            pool: app.tasks.clone(),
            directory: work.join("job-1"),
            stop: CancellationToken::new(),
        };
        std::fs::create_dir_all(&ctx.directory).unwrap();
        let cover = "https://img.example.org/cover.png";
        let picture = root.path().join("cover.png");
        image::RgbImage::new(16, 24).save(&picture).unwrap();
        let bytes = std::fs::read(picture).unwrap();
        {
            let mut fixture = fixture.lock().unwrap();
            fixture.responses.insert(
                page("collect", 0),
                (0..30)
                    .map(|i| movie(i, if i == 0 { cover } else { "" }))
                    .collect::<String>()
                    .into_bytes(),
            );
            fixture.responses.insert(
                page("collect", 30),
                (30..60)
                    .map(|i| movie(i, ""))
                    .collect::<String>()
                    .into_bytes(),
            );
            fixture.responses.insert(cover.into(), bytes.clone());
        }
        assert_eq!(
            Douban.sync(&app, &ctx, json!({})).await.unwrap()["added"],
            30
        );
        {
            let fixture = fixture.lock().unwrap();
            assert_eq!(fixture.requests.len(), 2);
            assert!(
                fixture
                    .requests
                    .iter()
                    .all(|(_, referer)| referer.as_deref() == Some("https://movie.douban.com/"))
            );
        }
        let stored: String =
            sqlx::query_scalar("SELECT images FROM posts WHERE source_key='douban:collect:0'")
                .fetch_one(&app.db)
                .await
                .unwrap();
        let refs: Vec<String> = serde_json::from_str(&stored).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(
            std::fs::read(app.data.join("uploads").join(&refs[0][3..])).unwrap(),
            bytes
        );
        assert_eq!(
            Douban.sync(&app, &ctx, json!({"all":true})).await.unwrap()["added"],
            5
        );
        assert_eq!(
            Douban.sync(&app, &ctx, json!({"all":true})).await.unwrap()["added"],
            0
        );
        assert_eq!(
            fixture
                .lock()
                .unwrap()
                .requests
                .iter()
                .filter(|(url, _)| url == cover)
                .count(),
            1
        );
        let id: i64 =
            sqlx::query_scalar("SELECT id FROM posts WHERE source_key='douban:collect:0'")
                .fetch_one(&app.db)
                .await
                .unwrap();
        crate::content::delete_post(&app, id).await.unwrap();
        assert!(!app.data.join("uploads").join(&refs[0][3..]).exists());
        assert_eq!(
            Douban.sync(&app, &ctx, json!({})).await.unwrap()["added"],
            0
        );
        // A different category is a different source key for the same movie.
        sqlx::query("INSERT INTO settings(key,value) VALUES('plugin:douban:category','wish')")
            .execute(&app.db)
            .await
            .unwrap();
        {
            let mut fixture = fixture.lock().unwrap();
            fixture
                .responses
                .insert(page("wish", 0), movie(0, cover).into_bytes());
            fixture
                .responses
                .insert(page("wish", 30), b"<html></html>".to_vec());
        }
        sqlx::query("CREATE TRIGGER fail_movie BEFORE INSERT ON posts WHEN NEW.source='douban' BEGIN SELECT RAISE(ABORT,'fixture failure'); END").execute(&app.db).await.unwrap();
        assert!(Douban.sync(&app, &ctx, json!({})).await.is_err());
        assert_eq!(
            std::fs::read_dir(app.data.join("uploads")).unwrap().count(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM plugin_pending_media")
                .fetch_one(&app.db)
                .await
                .unwrap(),
            0
        );
        sqlx::query("DROP TRIGGER fail_movie")
            .execute(&app.db)
            .await
            .unwrap();
        assert_eq!(
            Douban.sync(&app, &ctx, json!({})).await.unwrap()["added"],
            1
        );
        let meta: String =
            sqlx::query_scalar("SELECT meta FROM posts WHERE source_key='douban:wish:0'")
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&meta).unwrap()["category"],
            "想看"
        );
        // A later-page failure leaves committed entries intact and retry deduplicates them.
        sqlx::query("UPDATE settings SET value='do' WHERE key='plugin:douban:category'")
            .execute(&app.db)
            .await
            .unwrap();
        fixture
            .lock()
            .unwrap()
            .responses
            .insert(page("do", 0), movie(1, "").into_bytes());
        assert!(Douban.sync(&app, &ctx, json!({})).await.is_err());
        assert!(import::known(&app, "douban", "douban:do:1").await.unwrap());
        fixture
            .lock()
            .unwrap()
            .responses
            .insert(page("do", 30), b"<html></html>".to_vec());
        assert_eq!(
            Douban.sync(&app, &ctx, json!({})).await.unwrap()["added"],
            0
        );
        sqlx::query("UPDATE settings SET value='500' WHERE key='plugin:douban:history_limit'")
            .execute(&app.db)
            .await
            .unwrap();
        {
            let mut fixture = fixture.lock().unwrap();
            fixture.requests.clear();
            for index in 0..21 {
                fixture
                    .responses
                    .insert(page("do", index * 30), movie(1000 + index, "").into_bytes());
            }
        }
        assert_eq!(
            Douban.sync(&app, &ctx, json!({"all":true})).await.unwrap()["added"],
            20
        );
        assert_eq!(fixture.lock().unwrap().requests.len(), 20);
        ctx.stop.cancel();
        let requests = fixture.lock().unwrap().requests.len();
        assert!(Douban.sync(&app, &ctx, json!({})).await.is_err());
        assert_eq!(requests, fixture.lock().unwrap().requests.len());
    }
}
