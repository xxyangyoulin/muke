use super::{data::Data, pages::escape};
use anyhow::{Context, Result, bail};
use chrono::DateTime;
use std::path::Path;
use tokio_util::sync::CancellationToken;

fn xml(value: &str) -> String {
    escape(
        &value
            .chars()
            .filter(|c| {
                matches!(*c, '\t' | '\n' | '\r')
                    || (*c >= ' ' && !matches!(*c, '\u{fffe}' | '\u{ffff}'))
            })
            .collect::<String>(),
    )
}

pub fn write(destination: &Path, data: &Data, stop: &CancellationToken) -> Result<()> {
    if stop.is_cancelled() {
        bail!("任务已取消");
    }
    let base = if data.site_url.is_empty() {
        None
    } else {
        let base = url::Url::parse(&data.site_url).context("站点地址无效")?;
        if !matches!(base.scheme(), "http" | "https") {
            bail!("站点地址必须使用 HTTP 或 HTTPS");
        }
        Some(base)
    };
    let url = |path: &str| {
        base.as_ref()
            .map(|base| base.join(path).unwrap().to_string())
            .unwrap_or_else(|| path.into())
    };
    let profile = &data.content["state"]["profile"];
    let articles = data.articles["articles"]
        .as_array()
        .context("文章静态数据无效")?;
    let posts = data.content["posts"]
        .as_array()
        .context("动态静态数据无效")?;
    let mut items = Vec::new();
    for (article, item) in articles
        .iter()
        .map(|item| (true, item))
        .chain(posts.iter().map(|item| (false, item)))
    {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let date = DateTime::parse_from_rfc3339(item["created_at"].as_str().unwrap())
            .context("内容发布时间无效")?;
        let (title, description, link) = if article {
            (
                item["title"].as_str().unwrap_or("").to_owned(),
                item["excerpt"].as_str().unwrap_or("").to_owned(),
                url(&format!("/article/{}/", item["id"])),
            )
        } else {
            let description = scraper::Html::parse_fragment(item["content"].as_str().unwrap_or(""))
                .root_element()
                .text()
                .collect::<String>();
            (
                description.chars().take(40).collect(),
                description,
                url(&format!("/#post-{}", item["id"])),
            )
        };
        items.push((date, title, description, link));
    }
    items.sort_by(|a, b| b.0.cmp(&a.0));
    let mut rss = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>{}</title><link>{}</link><description>{}</description>"#,
        xml(profile["name"].as_str().unwrap_or("")),
        xml(&url("/")),
        xml(profile["bio"].as_str().unwrap_or(""))
    );
    for (date, title, description, link) in items.into_iter().take(30) {
        rss.push_str(&format!("<item><title>{}</title><link>{}</link><pubDate>{}</pubDate><description>{}</description></item>", xml(&title), xml(&link), date.to_rfc2822(), xml(&description)));
    }
    rss.push_str("</channel></rss>");
    let mut routes = vec![("/".to_owned(), None), ("/articles/".to_owned(), None)];
    if let Some(plugins) = data.content["state"]["plugins"].as_object() {
        for plugin in plugins.values() {
            if let Some(route) = plugin["page"]["route"].as_str() {
                routes.push((format!("{}/", route.trim_end_matches('/')), None));
            }
        }
    }
    for article in articles {
        let date = DateTime::parse_from_rfc3339(article["updated_at"].as_str().unwrap())
            .context("文章修改时间无效")?;
        routes.push((
            format!("/article/{}/", article["id"]),
            Some(date.format("%Y-%m-%d").to_string()),
        ));
    }
    let mut sitemap = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#,
    );
    for (route, date) in routes {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        sitemap.push_str(&format!("<url><loc>{}</loc>", xml(&url(&route))));
        if let Some(date) = date {
            sitemap.push_str(&format!("<lastmod>{date}</lastmod>"));
        }
        sitemap.push_str("</url>");
    }
    sitemap.push_str("</urlset>");
    if stop.is_cancelled() {
        bail!("任务已取消");
    }
    std::fs::write(destination.join("rss.xml"), rss)?;
    std::fs::write(destination.join("sitemap.xml"), sitemap)?;
    std::fs::write(
        destination.join("robots.txt"),
        format!(
            "User-agent: *\nAllow: /\n\nSitemap: {}\n",
            url("/sitemap.xml")
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Data {
        Data {
            site_url: "https://example.org".into(),
            content: json!({
                "state": {
                    "profile": {"name": "测试 <站点> & \"引号\"", "bio": "简介 & <b>标签</b>"},
                    "options": {},
                    "plugins": {"rss": {"page": {"route": "/rss"}}, "wallpapers": {"page": {"route": "/wallpapers"}}}
                },
                "posts": [
                    {"id": 1, "content": "<p>较早的动态 &amp; 文字</p>", "created_at": "2026-11-25T08:00:00+08:00"},
                    {"id": 2, "content": "<p>较晚的动态</p>", "created_at": "2026-11-26T09:30:00+08:00"}
                ],
                "plugins": {"rss": {"posts": [{"id": 3, "content": "隐藏来源动态", "created_at": "2026-11-27T08:00:00+08:00"}]}},
                "plugins_meta": {}
            }),
            articles: json!({"articles": [
                {"id": 7, "title": "文章 <tag> & \"引号\"", "excerpt": "摘要 & 文字", "created_at": "2026-11-26T10:00:00+08:00", "updated_at": "2026-11-26T12:00:00+08:00"},
                {"id": 8, "title": "最旧文章", "excerpt": "", "created_at": "2026-11-24T08:00:00+08:00", "updated_at": "2026-11-24T08:00:00+08:00"}
            ]}),
        }
    }

    fn texts(node: roxmltree::Node, tag: &str) -> Vec<String> {
        node.children()
            .filter(|child| child.is_element() && child.tag_name().name() == tag)
            .map(|child| child.text().unwrap_or("").to_owned())
            .collect()
    }

    fn read(path: std::path::PathBuf) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    fn parse(source: &str) -> roxmltree::Document<'_> {
        roxmltree::Document::parse(source).unwrap()
    }

    fn find_channel<'a>(document: &'a roxmltree::Document<'a>) -> roxmltree::Node<'a, 'a> {
        document
            .root_element()
            .children()
            .find(|node| node.tag_name().name() == "channel")
            .unwrap()
    }

    fn items<'a>(channel: roxmltree::Node<'a, 'a>) -> Vec<roxmltree::Node<'a, 'a>> {
        channel
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "item")
            .collect()
    }

    #[test]
    fn feeds_merge_sort_exclude_hidden_and_escape() {
        let root = tempfile::tempdir().unwrap();
        let mut data = fixture();
        data.content["posts"].as_array_mut().unwrap().push(json!({
            "id": 4,
            "content": format!("控制字符\u{1}过滤<char>{}", "长".repeat(50)),
            "created_at": "2026-11-23T08:00:00+08:00"
        }));
        write(root.path(), &data, &CancellationToken::new()).unwrap();
        let source = read(root.path().join("rss.xml"));
        let rss = parse(&source);
        let channel = find_channel(&rss);
        assert_eq!(texts(channel, "title"), ["测试 <站点> & \"引号\""]);
        let items = items(channel);
        // 时间排序：文章、动态混合后按时间倒序；隐藏来源与插件动态不进入聚合
        let links = items
            .iter()
            .flat_map(|item| texts(*item, "link"))
            .collect::<Vec<_>>();
        assert_eq!(
            links,
            [
                "https://example.org/article/7/",
                "https://example.org/#post-2",
                "https://example.org/#post-1",
                "https://example.org/article/8/",
                "https://example.org/#post-4"
            ]
        );
        assert_eq!(
            texts(items[1], "pubDate"),
            ["Thu, 26 Nov 2026 09:30:00 +0800"]
        );
        // 动态标题取正文文本前 40 字符，控制字符被过滤，标签不计入
        assert_eq!(
            texts(items[4], "title"),
            ["控制字符过滤".to_owned() + &"长".repeat(33)]
        );
        assert_eq!(
            texts(items[4], "description"),
            ["控制字符过滤".to_owned() + &"长".repeat(50)]
        );
        assert_eq!(texts(items[0], "description"), ["摘要 & 文字"]);
        let source = read(root.path().join("sitemap.xml"));
        let sitemap = roxmltree::Document::parse(&source).unwrap();
        let entries = sitemap
            .root_element()
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "url")
            .map(|node| {
                (
                    texts(node, "loc")[0].clone(),
                    texts(node, "lastmod").into_iter().next(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            [
                ("https://example.org/".to_owned(), None),
                ("https://example.org/articles/".to_owned(), None),
                ("https://example.org/rss/".to_owned(), None),
                ("https://example.org/wallpapers/".to_owned(), None),
                (
                    "https://example.org/article/7/".to_owned(),
                    Some("2026-11-26".to_owned())
                ),
                (
                    "https://example.org/article/8/".to_owned(),
                    Some("2026-11-24".to_owned())
                ),
            ]
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("robots.txt")).unwrap(),
            "User-agent: *\nAllow: /\n\nSitemap: https://example.org/sitemap.xml\n"
        );
    }

    #[test]
    fn feeds_without_site_url_limit_and_cancel() {
        let root = tempfile::tempdir().unwrap();
        let mut data = fixture();
        data.site_url = String::new();
        write(root.path(), &data, &CancellationToken::new()).unwrap();
        let source = read(root.path().join("rss.xml"));
        let rss = parse(&source);
        assert_eq!(texts(find_channel(&rss), "link"), ["/"]);
        assert_eq!(
            std::fs::read_to_string(root.path().join("robots.txt")).unwrap(),
            "User-agent: *\nAllow: /\n\nSitemap: /sitemap.xml\n"
        );
        // 最新 30 条上限
        let mut capped = fixture();
        for id in 10..50 {
            capped.articles["articles"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "id": id, "title": format!("文章 {id}"), "excerpt": "",
                    "created_at": format!("2026-11-20T08:00:{id:02}+08:00"),
                    "updated_at": format!("2026-11-20T08:00:{id:02}+08:00")
                }));
        }
        let target = root.path().join("capped");
        std::fs::create_dir_all(&target).unwrap();
        write(&target, &capped, &CancellationToken::new()).unwrap();
        let capped_source = read(target.join("rss.xml"));
        let rss = parse(&capped_source);
        assert_eq!(items(find_channel(&rss)).len(), 30);
        // 非法站点地址与取消
        let mut invalid = fixture();
        invalid.site_url = "ftp://example.org".into();
        assert!(
            write(
                &root.path().join("invalid"),
                &invalid,
                &CancellationToken::new()
            )
            .is_err()
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(write(&root.path().join("cancelled"), &fixture(), &cancelled).is_err());
        assert!(!root.path().join("cancelled/rss.xml").exists());
    }
}
