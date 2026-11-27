use muke::export::{assets, data::Data, pages};
use serde_json::json;
use std::{path::Path, process::Command};
use tokio_util::sync::CancellationToken;

fn fixture() -> Data {
    let mut data = Data {
        site_url: "https://example.org".into(),
        content: json!({"state":{"admin":false,"setup":false,"profile":{"name":"静态测试站","bio":"简介","handle":"tester","avatar":null,"cover":null},"options":{"comment_provider":"off","site_tagline":"记录","site_description":"静态站点"},"plugins":{}},"posts":[{"id":1,"content":"静态动态正文","created_at":"2026-11-26T08:00:00+08:00","updated_at":"2026-11-26T08:00:00+08:00","images":[],"meta":{},"source":"","likes":2}],"plugins":{},"plugins_meta":{}}),
        articles: json!({"articles":[{"id":7,"title":"静态文章","html":"<p>静态文章正文</p>","excerpt":"静态文章正文","tags":"测试","created_at":"2026-11-26T08:00:00+08:00","updated_at":"2026-11-26T08:00:00+08:00","comments":[]}]}),
    };
    for id in ["rss", "wallpapers", "weibo", "douban"] {
        let manifest = muke::plugins::get(id).unwrap().manifest();
        let mut plugin = json!({"name":manifest["name"],"module":format!("/plugins/{id}/plugin.js"),"style":format!("/plugins/{id}/plugin.css")});
        if let Some(page) = manifest.get("page") {
            plugin["page"] = page.clone();
        }
        data.content["state"]["plugins"][id] = plugin;
    }
    let image = json!({"url":"/fixture.svg","thumb":"/fixture.svg","width":1,"height":1});
    let rss = json!({"id":2,"source":"rss","source_key":"rss:feed1:entry","content":"订阅正文","link":"https://example.org/?x=1&y=2","created_at":"2026-11-26T08:00:00+08:00","updated_at":"2026-11-26T08:00:00+08:00","images":[image],"meta":{"card":"rss","name":"订阅站点","title":"订阅标题 <img id='injected'>","summary":"订阅摘要","avatar":image}});
    data.content["plugins"]["rss"] =
        json!({"feeds":[{"id":"feed1","title":"订阅站点","count":1}],"posts":[rss]});
    data.content["plugins"]["wallpapers"] = json!([{"id":"wall1","name":"壁纸 <img id='injected'>","width":1,"height":1,"bytes":100,"mtime":1795651200,"original":"/fixture.svg","thumbnail":"/fixture.svg"}]);
    data.content["plugins_meta"] = json!({"rss":{"total":1},"wallpapers":{"batch":24}});
    for (id, source, meta) in [
        (
            3,
            "douban",
            json!({"card":"movie","title":"电影 <img id='injected'>","intro":"介绍","rating":"★★★★★","date":"2026-11-26","category":"看过","comment":"影评"}),
        ),
        (
            4,
            "weibo",
            json!({"card":"weibo","text":"微博正文 <script>bad()</script>"}),
        ),
    ] {
        data.content["posts"].as_array_mut().unwrap().push(json!({"id":id,"source":source,"meta":meta,"content":"历史动态","link":"javascript:alert(1)","created_at":"2026-11-26T08:00:00+08:00","updated_at":"2026-11-26T08:00:00+08:00","images":[image]}));
    }
    data
}

#[test]
fn plugin_pages_and_historical_cards_are_baked() {
    let root = tempfile::tempdir().unwrap();
    let frontend = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend");
    let data = fixture();
    pages::write(&frontend, root.path(), &data, &CancellationToken::new()).unwrap();
    for (file, selector, expected) in [
        ("index.html", ".post-movie-card", "电影 <img id='injected'>"),
        (
            "index.html",
            ".post-weibo-card",
            "微博正文 <script>bad()</script>",
        ),
        (
            "rss/index.html",
            ".post-rss-card",
            "订阅标题 <img id='injected'>",
        ),
    ] {
        let html = scraper::Html::parse_document(
            &std::fs::read_to_string(root.path().join(file)).unwrap(),
        );
        let query = scraper::Selector::parse(selector).unwrap();
        assert!(
            html.select(&query)
                .next()
                .unwrap()
                .text()
                .collect::<String>()
                .contains(expected)
        );
        let query = scraper::Selector::parse("#injected, a[href^='javascript:']").unwrap();
        assert!(html.select(&query).next().is_none());
        // 左侧导航只收录声明 page.nav 的插件（壁纸）；RSS 走首页顶栏标签
        let query = scraper::Selector::parse("nav a[href='/wallpapers/']").unwrap();
        assert_eq!(html.select(&query).count(), 1);
        let query = scraper::Selector::parse("nav a[href='/rss/']").unwrap();
        assert_eq!(html.select(&query).count(), 0);
    }
    let gallery = scraper::Html::parse_document(
        &std::fs::read_to_string(root.path().join("wallpapers/index.html")).unwrap(),
    );
    let query = scraper::Selector::parse(".tile-preview").unwrap();
    assert_eq!(
        gallery.select(&query).next().unwrap().value().attr("href"),
        Some("/fixture.svg")
    );
    let home = std::fs::read_to_string(root.path().join("index.html")).unwrap();
    let home = scraper::Html::parse_document(&home);
    let query = scraper::Selector::parse("#baked-body").unwrap();
    assert!(
        !home
            .select(&query)
            .next()
            .unwrap()
            .text()
            .collect::<String>()
            .contains("订阅标题")
    );
    let mut disabled = fixture();
    disabled.content["state"]["plugins"]["rss"]
        .as_object_mut()
        .unwrap()
        .remove("page");
    disabled.content["posts"]
        .as_array_mut()
        .unwrap()
        .push(data.content["plugins"]["rss"]["posts"][0].clone());
    disabled.content["plugins"]
        .as_object_mut()
        .unwrap()
        .remove("rss");
    let target = root.path().join("disabled");
    pages::write(&frontend, &target, &disabled, &CancellationToken::new()).unwrap();
    assert!(!target.join("rss/index.html").exists());
    let html =
        scraper::Html::parse_document(&std::fs::read_to_string(target.join("index.html")).unwrap());
    let query = scraper::Selector::parse(".post-rss-card").unwrap();
    assert_eq!(html.select(&query).count(), 1);
}

#[tokio::test]
#[ignore = "requires Node, Chrome and local HTTP listening"]
async fn static_pages_in_browser() {
    let root = tempfile::tempdir().unwrap();
    let frontend = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend");
    let stop = CancellationToken::new();
    let data = fixture();
    let enabled: Vec<String> = data.content["state"]["plugins"]
        .as_object()
        .map(|plugins| plugins.keys().cloned().collect())
        .unwrap_or_default();
    assets::copy(&frontend, root.path(), &enabled, &stop)
        .await
        .unwrap();
    std::fs::write(
        root.path().join("fixture.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#,
    )
    .unwrap();
    pages::write(&frontend, root.path(), &data, &stop).unwrap();
    let status = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/rust/export.cjs"
        ))
        .arg(root.path())
        .status()
        .unwrap();
    assert!(status.success());
}
