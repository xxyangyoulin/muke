<?php
declare(strict_types=1);

function render_article(array $article): string {
    if (($article['format'] ?? 'md') === 'html') {
        // 服务端纵深防御：剥离 script/事件属性/javascript: URL（客户端保存时已消毒；此处兜底防恶意备份导入）
        return preg_replace([
            '#<script\b[^>]*>.*?</script>#is',
            '#<script\b[^>]*/?>#is',
            '#\son\w+\s*=\s*(?:"[^"]*"|\'[^\']*\'|[^\s>]+)#is',
            '#\s(?:href|src)\s*=\s*(?:"\s*javascript:[^"]*"|\'\s*javascript:[^\']*\'|\s*javascript:[^\s>]*)#is',
            '#<(/?)(iframe|object|embed|form|base)\b[^>]*>#is',
        ], ['$1', '', '', '', ''], $article['content']);
    }
    require_once dirname(__DIR__) . '/lib/Parsedown.php';
    $parser = new Parsedown();
    $parser->setBreaksEnabled(true);
    $parser->setMarkupEscaped(true); // 转义 Markdown 中的原始 HTML：防恶意备份导入的存储型 XSS（需要 HTML 时用 html 格式）
    return $parser->text($article['content']);
}

function normalize_tags(string $input): string {
    $tags = [];
    foreach (preg_split('/[,，、\s]+/u', trim($input)) ?: [] as $tag) {
        $tag = trim($tag);
        if ($tag === '' || mb_strlen($tag) > 20) continue;
        $tags[] = $tag;
        if (count($tags) === 5) break;
    }
    return implode(',', array_values(array_unique($tags)));
}

function article_image_refs(string $content): array {
    preg_match_all('#(?:/image\?upload=|/media/uploads/)([a-f0-9]{32}\.(?:jpg|png|webp|gif|avif))#', $content, $matches);
    return array_values(array_unique($matches[1]));
}

function article_summary(array $article): array {
    $html = render_article($article);
    $text = trim(preg_replace('/\s+/u', ' ', html_entity_decode(strip_tags($html), ENT_QUOTES | ENT_HTML5, 'UTF-8')));
    $article['excerpt'] = mb_substr($text, 0, 110) . (mb_strlen($text) > 110 ? '…' : '');
    $article['text_length'] = mb_strlen($text);
    $article['cover'] = null;
    if (preg_match('~<img\b[^>]*\ssrc\s*=\s*(?:"([^"]*)"|\'([^\']*)\'|([^\s>]+))~i', $html, $match)) {
        $src = html_entity_decode(($match[1] ?? '') ?: (($match[2] ?? '') ?: ($match[3] ?? '')), ENT_QUOTES | ENT_HTML5, 'UTF-8');
        if (preg_match('#^/image\?upload=([a-f0-9]{32}\.(?:jpg|png|webp|gif|avif))(?:&|$)#', $src, $upload)) {
            $article['cover'] = '/image?upload=' . $upload[1] . '&thumb=1';
        } elseif (preg_match('#^(?:https?://|/(?!/))#i', $src)) {
            $article['cover'] = $src;
        }
    }
    unset($article['content'], $article['images']);
    return $article;
}

function post_images(PDO $db, string $json): array {
    $result = [];
    foreach (json_decode($json, true) ?: [] as $ref) {
        if (!is_string($ref) || !str_contains($ref, ':')) continue;
        [$kind, $value] = explode(':', $ref, 2);
        if ($kind === 'up') {
            if (!preg_match('/^[a-f0-9]{32}\.(jpg|png|webp|gif|avif)$/', $value) || !is_file(DATA_DIR . '/uploads/' . $value)) continue;
            $info = @getimagesize(DATA_DIR . '/uploads/' . $value);
            if (!$info) continue;
            $result[] = ['ref' => $ref, 'url' => '/image?upload=' . $value, 'thumb' => '/image?upload=' . $value . '&thumb=1', 'width' => $info[0], 'height' => $info[1], 'bytes' => filesize(DATA_DIR . '/uploads/' . $value)];
        } elseif ($kind !== 'http') {
            // 插件自定义引用协议（如壁纸的 wall:<id>）→ 插件解析
            $hit = plugin_resolve_ref($db, $ref);
            if (is_array($hit)) { $hit['ref'] = $ref; $result[] = $hit; }
        }
    }
    return $result;
}
