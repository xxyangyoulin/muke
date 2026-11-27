use super::{media, snapshot::Snapshot};
use crate::{api::App, plugins, render};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
use tokio_util::sync::CancellationToken;

pub struct Data {
    pub content: Value,
    pub site_url: String,
    pub articles: Value,
}

pub async fn build(
    app: &App,
    mut snapshot: Snapshot<'_>,
    destination: &Path,
    stop: &CancellationToken,
) -> Result<Data> {
    let copies = media::copy_sources(app, &mut snapshot, destination, stop).await?;
    let Snapshot {
        state,
        site_url,
        posts,
        articles,
        comments,
        hidden_sources,
        plugins: plugin_snapshots,
        _guard,
    } = snapshot;
    drop(_guard);
    let media = copies.finish(app, destination, stop).await?;
    let counts = if state["options"]["comment_provider"] == "giscus" {
        let mut terms: Vec<_> = posts
            .iter()
            .filter(|post| {
                !hidden_sources
                    .iter()
                    .any(|source| post["source"].as_str() == Some(source.as_str()))
            })
            .map(|post| format!("post:{}", post["id"]))
            .collect();
        terms.extend(
            articles
                .iter()
                .map(|article| format!("article:{}", article["id"])),
        );
        for id in plugin_snapshots.keys() {
            terms.extend(
                posts
                    .iter()
                    .filter(|post| post["source"].as_str() == Some(id.as_str()))
                    .map(|post| format!("post:{}", post["id"])),
            );
        }
        crate::comments::giscus_counts(
            &app.network,
            state["options"]["giscus_repo"].as_str().unwrap_or(""),
            &terms,
            stop,
        )
        .await?
    } else {
        BTreeMap::new()
    };
    let destination = destination.to_owned();
    let stop = stop.clone();
    tokio::task::spawn_blocking(move || {
        let mut posts = posts;
        for post in &mut posts {
            if stop.is_cancelled() { bail!("任务已取消"); }
            let references: Vec<String> = serde_json::from_str(post["images"].as_str().unwrap_or("[]")).unwrap_or_default();
            post["images"] = json!(references.iter().filter_map(|reference| media.get(reference)).collect::<Vec<_>>());
            let mut meta: Value = serde_json::from_str(post["meta"].as_str().unwrap_or("")).unwrap_or(json!([]));
            if let Some(reference) = meta.get("avatar").and_then(Value::as_str) {
                meta["avatar"] = media.get(reference).cloned().unwrap_or(Value::Null);
            }
            post["meta"] = meta;
            post["comments"] = json!(comments.get(&format!("post:{}", post["id"])).cloned().unwrap_or_default());
            if let Some(value) = counts.get(&format!("post:{}", post["id"])) {
                post["giscus_counts"] = value.clone();
            }
        }
        let mut articles = articles;
        for article in &mut articles {
            if stop.is_cancelled() { bail!("任务已取消"); }
            let html = rewrite_html(&render::article(article), &media)?;
            render::summarize(article);
            if let Some(cover) = article["cover"].as_str() {
                article["cover"] = json!(static_url(cover, &media)?);
            }
            article["html"] = json!(html);
            article["comments"] = json!(comments.get(&format!("article:{}", article["id"])).cloned().unwrap_or_default());
            if let Some(value) = counts.get(&format!("article:{}", article["id"])) {
                article["giscus_counts"] = value.clone();
            }
        }
        let (mut plugin_items, plugin_meta) = plugins::export_data(&plugin_snapshots, &posts, &media);
        for item in plugin_items.as_object_mut().into_iter().flat_map(|items| items.values_mut()) {
            if let Some(items) = item["posts"].as_array_mut() {
                for post in items {
                    if let Some(value) = counts.get(&format!("post:{}", post["id"])) {
                        post["giscus_counts"] = value.clone();
                    }
                }
            }
        }
        posts.retain(|post| !hidden_sources.iter().any(|source| post["source"] == *source));
        let data = Data {
            site_url,
            content: json!({"state":state,"posts":posts,"plugins":plugin_items,"plugins_meta":plugin_meta}),
            articles: json!({"articles":articles}),
        };
        if stop.is_cancelled() { bail!("任务已取消"); }
        std::fs::create_dir_all(destination.join("assets"))?;
        std::fs::write(destination.join("assets/content.json"), serde_json::to_vec(&data.content)?)?;
        std::fs::write(destination.join("assets/articles.json"), serde_json::to_vec(&data.articles)?)?;
        if stop.is_cancelled() { bail!("任务已取消"); }
        Ok(data)
    }).await?
}

fn static_url(url: &str, media: &BTreeMap<String, Value>) -> Result<String> {
    let decoded = url.replace("&amp;", "&");
    let Some(query) = decoded.strip_prefix("/image?") else {
        return Ok(url.into());
    };
    let params: BTreeMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).collect();
    let Some(name) = params
        .get("upload")
        .filter(|name| crate::api::upload_name(name))
    else {
        return Ok(url.into());
    };
    let Some(descriptor) = media.get(&format!("up:{name}")) else {
        bail!("文章引用的图片不存在：{name}");
    };
    let field = if params.get("thumb").is_some_and(|value| value == "1") {
        "thumb"
    } else {
        "url"
    };
    Ok(descriptor[field].as_str().unwrap().to_owned())
}

fn rewrite_html(html: &str, media: &BTreeMap<String, Value>) -> Result<String> {
    let mut document = scraper::Html::parse_fragment(html);
    for node in document.tree.values_mut() {
        if let scraper::Node::Element(element) = node {
            for (name, value) in &mut element.attrs {
                if matches!(name.local.as_ref(), "src" | "href") {
                    *value = static_url(value.as_ref(), media)?.into();
                }
            }
        }
    }
    Ok(document.root_element().inner_html())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        database,
        export::{artifact::Artifact, snapshot},
        network, tasks,
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn static_json_resolves_media_filters_timeline_and_preserves_article_text() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&root.path().join("work")).await.unwrap(),
            network: network::Client::new(None),
            data,
            config: Arc::new(Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        plugins::migrate(&app).await.unwrap();
        sqlx::query("INSERT INTO settings VALUES('plugin:rss:enabled','1')")
            .execute(&app.db)
            .await
            .unwrap();
        let name = format!("{}.png", "a".repeat(32));
        image::RgbImage::new(160, 120)
            .save(app.data.join("uploads").join(&name))
            .unwrap();
        for source in ["", "rss"] {
            sqlx::query("INSERT INTO posts(content,source,images,meta,created_at,updated_at) VALUES('post',?,?,?,'2026','2026')")
                .bind(source).bind(json!([format!("up:{name}")]).to_string())
                .bind(json!({"avatar":format!("up:{name}")}).to_string()).execute(&app.db).await.unwrap();
        }
        let content = format!(
            "<p>正文 &amp; text</p><img src='/image?upload={name}&amp;v=1'><a href='/image?upload={name}&amp;thumb=1'>thumbnail</a><code>&lt;img src=\"/image?upload={name}\"&gt;</code>"
        );
        sqlx::query("INSERT INTO articles(title,content,format,created_at,updated_at) VALUES('title',?,'html','2026','2026')").bind(content).execute(&app.db).await.unwrap();
        sqlx::query("INSERT INTO articles(title,content,draft,created_at,updated_at) VALUES('secret-draft','private',1,'2026','2026')").execute(&app.db).await.unwrap();
        sqlx::query("INSERT INTO comments(kind,target_id,author,content,created_at) VALUES('article',1,'reader','approved','2026')").execute(&app.db).await.unwrap();
        let stop = CancellationToken::new();
        let artifact = Artifact::new(&root.path().join("export")).unwrap();
        let snapshot = snapshot::capture(&app, &stop).await.unwrap();
        let output = build(&app, snapshot, artifact.path(), &stop).await.unwrap();
        assert!(app.content_lock.try_write().is_ok());
        assert_eq!(output.content["posts"].as_array().unwrap().len(), 1);
        assert_eq!(
            output.content["plugins"]["rss"]["posts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            output.content["posts"][0]["images"][0]["url"]
                .as_str()
                .unwrap()
                .starts_with("/media/uploads/")
        );
        assert!(
            output.content["posts"][0]["meta"]["avatar"]["thumb"]
                .as_str()
                .unwrap()
                .starts_with("/thumbnails/")
        );
        let article = &output.articles["articles"][0];
        assert_eq!(output.articles["articles"].as_array().unwrap().len(), 1);
        assert!(article.get("content").is_none());
        assert_eq!(article["comments"][0]["content"], "approved");
        assert!(
            article["cover"]
                .as_str()
                .unwrap()
                .starts_with("/thumbnails/")
        );
        let html = scraper::Html::parse_fragment(article["html"].as_str().unwrap());
        for tag in ["img", "a"] {
            let selector = scraper::Selector::parse(tag).unwrap();
            let element = html.select(&selector).next().unwrap();
            let url = element
                .value()
                .attr(if tag == "img" { "src" } else { "href" })
                .unwrap();
            assert!(artifact.path().join(&url[1..]).is_file());
        }
        let selector = scraper::Selector::parse("code").unwrap();
        assert_eq!(
            html.select(&selector)
                .next()
                .unwrap()
                .text()
                .collect::<String>(),
            format!("<img src=\"/image?upload={name}\">")
        );
        for (file, expected) in [
            ("content.json", &output.content),
            ("articles.json", &output.articles),
        ] {
            let actual: Value = serde_json::from_slice(
                &std::fs::read(artifact.path().join("assets").join(file)).unwrap(),
            )
            .unwrap();
            assert_eq!(&actual, expected);
        }
        let missing = format!("{}.png", "f".repeat(32));
        assert!(
            rewrite_html(
                &format!("<img src=\"/image?upload={missing}\">"),
                &BTreeMap::new()
            )
            .is_err()
        );
        let snapshot = snapshot::capture(&app, &stop).await.unwrap();
        stop.cancel();
        let other = Artifact::new(&root.path().join("other")).unwrap();
        assert!(build(&app, snapshot, other.path(), &stop).await.is_err());
        assert!(!other.path().join("assets/content.json").exists());
        assert!(app.content_lock.try_write().is_ok());
    }

    fn giscus_address(term: &str) -> String {
        url::Url::parse_with_params(
            "https://giscus.app/api/discussions",
            &[
                ("repo", "owner/repo"),
                ("term", term),
                ("strict", "true"),
                ("first", "50"),
                ("after", ""),
            ],
        )
        .unwrap()
        .to_string()
    }

    #[tokio::test]
    async fn giscus_counts_are_fetched_and_attached_to_exported_items() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let fixture = Arc::new(std::sync::Mutex::new(network::TestResponses::default()));
        let mut client = network::Client::new(None);
        client.fixture = Some(fixture.clone());
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&root.path().join("work")).await.unwrap(),
            network: client,
            data,
            config: Arc::new(Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        plugins::migrate(&app).await.unwrap();
        for (key, value) in [
            ("plugin:rss:enabled", "1"),
            ("plugin:rss:in_feed", "0"),
            ("comment_provider", "giscus"),
            ("giscus_repo", "owner/repo"),
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
        for (term, body) in [
            (
                "post:1",
                r#"{"discussion":{"totalCommentCount":2,"totalReplyCount":1,"reactions":{"HEART":{"count":4}},"pageInfo":{"hasNextPage":false}}}"#,
            ),
            ("post:2", r#"{"error":"Discussion not found"}"#),
            (
                "article:1",
                r#"{"discussion":{"totalCommentCount":6,"totalReplyCount":3,"reactions":{"HEART":{"count":9}},"pageInfo":{"hasNextPage":false}}}"#,
            ),
        ] {
            let mut responses = fixture.lock().unwrap();
            if term == "post:2" {
                responses.statuses.insert(giscus_address(term), 404);
            }
            responses
                .responses
                .insert(giscus_address(term), body.as_bytes().to_vec());
        }
        for source in ["", "rss"] {
            sqlx::query("INSERT INTO posts(content,source,images,meta,created_at,updated_at) VALUES('post',?,'[]','{}','2026','2026')")
                .bind(source)
                .execute(&app.db)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO articles(title,content,format,created_at,updated_at) VALUES('title','body','html','2026','2026')")
            .execute(&app.db)
            .await
            .unwrap();
        let stop = CancellationToken::new();
        let artifact = Artifact::new(&root.path().join("export")).unwrap();
        let snapshot = snapshot::capture(&app, &stop).await.unwrap();
        let output = build(&app, snapshot, artifact.path(), &stop).await.unwrap();
        assert_eq!(output.content["posts"].as_array().unwrap().len(), 1);
        assert_eq!(
            output.content["posts"][0]["giscus_counts"],
            json!({"comments":3,"likes":4})
        );
        assert_eq!(
            output.articles["articles"][0]["giscus_counts"],
            json!({"comments":9,"likes":9})
        );
        assert_eq!(
            output.content["plugins"]["rss"]["posts"][0]["giscus_counts"],
            json!({"comments":0,"likes":0})
        );
        let written: Value = serde_json::from_slice(
            &std::fs::read(artifact.path().join("assets").join("content.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            written["posts"][0]["giscus_counts"],
            json!({"comments":3,"likes":4})
        );
        assert_eq!(
            written["plugins"]["rss"]["posts"][0]["giscus_counts"],
            json!({"comments":0,"likes":0})
        );
        assert!(
            fixture
                .lock()
                .unwrap()
                .requests
                .iter()
                .filter(|(url, _)| url.contains("giscus.app"))
                .count()
                == 3
        );
    }
}
