<?php
require dirname(__DIR__) . '/src/Articles.php';

function check_summary(bool $ok, string $message): void {
    if (!$ok) throw new RuntimeException($message);
}

$article = article_summary(['content' => "# 标题\n\n**加粗** 与 [链接](https://example.com) &amp; `代码`\n\n![图](https://example.com/image.jpg?a=1&b=2)", 'format' => 'md']);
check_summary($article['excerpt'] === '标题 加粗 与 链接 & 代码', 'Markdown 摘要或实体解码错误');
check_summary($article['cover'] === 'https://example.com/image.jpg?a=1&b=2', '远程 Markdown 封面丢失');
check_summary(!isset($article['content'], $article['html']), '摘要包含正文');
$article = article_summary(['content' => '<p>你好 &lt;世界&gt; &nbsp; &amp;</p><img src=\'https://example.com/a.jpg?a=1&amp;b=2\'>', 'format' => 'html']);
check_summary($article['excerpt'] === '你好 <世界> &', 'HTML 摘要错误');
check_summary($article['cover'] === 'https://example.com/a.jpg?a=1&b=2', 'HTML 封面错误');
$name = str_repeat('a', 32) . '.jpg';
$article = article_summary(['content' => "![](/image?upload=$name&thumb=1)"]);
check_summary($article['cover'] === '/image?upload=' . $name . '&thumb=1', '本地封面缩略图错误');
foreach (['javascript:alert(1)', 'data:image/png;base64,abc', ''] as $src) {
    check_summary(article_summary(['content' => '<img src="' . $src . '">', 'format' => 'html'])['cover'] === null, '不安全或空封面被接受');
}
echo "article-summary: Markdown、HTML、实体、远程/本地封面和危险协议通过\n";
