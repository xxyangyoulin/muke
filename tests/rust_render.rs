use muke::render;
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn article_rendering_and_summaries_match_php_fixtures() {
    let upload = "a".repeat(32) + ".avif";
    let fixtures = json!([
        {"format":"md", "content":r"\: \*literal\* \\: \@ \= \/ and **\:bold**"},
        {"format":"md", "content":r"https\://example.org/escaped and https://example.org/actual"},
        {"format":"md", "content":"See https://example.org/path?a=1&b=2, HTTP://example.org/ and (https://example.org/test)."},
        {"format":"md", "content":"https://example.org/中文/ **https://example.org/bold**\n\n<https://example.org/explicit>"},
        {"format":"md", "content":"`https://example.org/code`\n\n```text\nhttps://example.org/block\n```\n\n[https://example.org/label](https://example.org/target)\n\n![https://example.org/alt](/image.png)"},
        {"format":"md", "content":"## 标题\n\n第一行\n第二行 **加粗** *斜体* ~~删除~~\n\n> 引用\n\n3. 三\n4. 四"},
        {"format":"md", "content":"```rust\nlet value = \"<正文>\";\n```\n\n`inline`"},
        {"format":"md", "content":"| 左 | 右 | 中 |\n| :--- | ---: | :---: |\n| A | B | C |"},
        {"format":"md", "content":"[链接][ref]\n\n![图片](/image?upload=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.avif&v=3)\n\n[ref]: https://example.org/?a=1&b=2 \"标题\""},
        {"format":"html", "content":format!("<h2>标题</h2><p>正文 &amp; &lt;标签&gt; &#x1F600;</p><img><img src='/image?upload={upload}&thumb=0&v=2' width='50%'>")},
        {"format":"html", "content":"<p><b>粗体</b> <i>斜体</i> <u>下划线</u><br><code>代码</code></p><ol start='3'><li>列表</li></ol><blockquote>引用</blockquote>"},
        {"format":"html", "content":"<img src='HTTPS://example.org/image.png'>"},
        {"format":"html", "content":"<img src='//example.org/image.png'><img src='/fallback.png'>"},
        {"format":"html", "content":format!("<p>{}</p>", "中😀 &amp; ".repeat(30))},
        {"format":"md", "content":"# 一级标题\n\n###### 六级标题\n\n####### 七级不是标题"},
        {"format":"md", "content":"标题一\n======\n\n标题二\n------\n\n---\n\n***\n\n- - -"},
        {"format":"md", "content":"***全部粗斜*** 与 **粗 *内斜* 粗** 和 _词间斜体_ 与 mid_word_不应斜_"},
        {"format":"md", "content":"- [ ] 待办事项\n- [x] 已完成"},
        {"format":"md", "content":"- 外层一\n- 外层二\n  - 内层甲\n  - 内层乙\n- 外层三"},
        {"format":"md", "content":"行一  \n行二"},
        {"format":"md", "content":"版权 &copy; 2026 与 &amp; 和 &lt;实体&gt; 及 &#169; &#x1F600;"},
        {"format":"md", "content":"前置 <br> 后置 <div>块</div> <span style='color:red'>样式</span> 尾部"},
        {"format":"md", "content":"    缩进代码行一\n    缩进代码行二"},
        {"format":"md", "content":"~~~js\nconst value = 1;\n~~~"},
        {"format":"md", "content":"![替代](/image?upload=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.avif \"图片说明\")"},
        {"format":"md", "content":"[![替代文本](/pic.png)](https://example.org/frame)"},
        {"format":"md", "content":"> 外层引用\n> > 嵌套引用\n> 回到外层"},
        {"format":"md", "content":"访问 https://example.org/page. 以及 https://example.org/end?q=1 结束"},
        {"format":"html", "content":"<h1>一</h1><h4>四</h4><ul><li>项</li></ul><pre><code>代码块</code></pre><hr><a href='https://example.org/' title='提示'>链接</a>"},
        {"format":"html", "content":"<h5>五</h5><h6>六</h6><dl><dt>词</dt><dd>义</dd></dl>"},
    ]);
    let mut php = Command::new("php").args(["-r",
        "require $argv[1]; $out=[]; foreach(json_decode(stream_get_contents(STDIN),true) as $a) {$out[]=['html'=>render_article($a),'summary'=>article_summary($a)];} echo json_encode($out);",
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/Articles.php")])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    php.stdin
        .take()
        .unwrap()
        .write_all(fixtures.to_string().as_bytes())
        .unwrap();
    let output = php.wait_with_output().unwrap();
    assert!(output.status.success());
    let expected: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    for (index, (fixture, expected)) in fixtures
        .as_array()
        .unwrap()
        .iter()
        .zip(expected)
        .enumerate()
    {
        let mut actual = fixture.clone();
        render::summarize(&mut actual);
        for key in ["excerpt", "text_length", "cover"] {
            assert_eq!(
                actual[key], expected["summary"][key],
                "fixture {index}: {key}"
            );
        }
        let actual = scraper::Html::parse_fragment(&render::article(fixture));
        let expected = scraper::Html::parse_fragment(expected["html"].as_str().unwrap());
        for tag in [
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "strong",
            "em",
            "del",
            "blockquote",
            "li",
            "br",
            "code",
            "pre",
            "th",
            "td",
            "a",
            "img",
            "ol",
            "ul",
            "hr",
            "dl",
            "dt",
            "dd",
            "input",
            "span",
        ] {
            let selector = scraper::Selector::parse(tag).unwrap();
            let signature = |doc: &scraper::Html| {
                doc.select(&selector)
                    .map(|node| {
                        let text = node.text().collect::<String>();
                        let text = if matches!(tag, "ul" | "ol" | "li" | "blockquote") {
                            text.split_whitespace().collect::<Vec<_>>().join(" ")
                        } else {
                            text.trim_end().to_owned()
                        };
                        (
                            text,
                            ["href", "src", "title", "class", "width", "start", "style"].map(
                                |attr| {
                                    node.value().attr(attr).map(|value| {
                                        if matches!(attr, "href" | "src") {
                                            url::Url::parse(value)
                                                .map(|url| url.to_string())
                                                .unwrap_or_else(|_| value.to_owned())
                                        } else if attr == "style" {
                                            value.replace(" ", "").trim_end_matches(';').to_owned()
                                        } else {
                                            value.to_owned()
                                        }
                                    })
                                },
                            ),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                signature(&actual),
                signature(&expected),
                "fixture {index}: {tag}"
            );
        }
    }
}

#[test]
fn table_alignment_does_not_allow_unrelated_styles_or_active_content() {
    let html = render::article(
        &json!({"format":"html", "content":"<table><tr><td style='text-align:right;position:fixed;background-image:url(https://example.org/track)' onclick='alert(1)'>safe</td></tr></table><script>alert(1)</script><a href='jav&#x61;script:alert(1)'>link</a>"}),
    );
    assert!(html.contains("text-align:right"));
    for forbidden in [
        "position",
        "background",
        "onclick",
        "<script",
        "javascript:",
    ] {
        assert!(!html.contains(forbidden), "{html}");
    }
}
