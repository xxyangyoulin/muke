<?php
declare(strict_types=1);

// RSS 订阅插件契约测试：时间线可见性、多源同步幂等、墓碑、历史上限、退订清史、数据端点与静态导出。
// 用临时 HTTP 服务提供本地 Atom 夹具（含图片），不依赖外网。

$sandbox = $argv[1];
$dataDir = $sandbox . '/rss-data';
mkdir($dataDir);
putenv('MKBLOG_DATA_DIR=' . $dataDir);

$fixtureDir = $dataDir . '/fixture';
mkdir($fixtureDir);
$port = random_int(21000, 39999);
$entries = '';
foreach ([['3', '2026-11-03T08:00:00Z', true], ['2', '2026-11-02T08:00:00Z', false], ['1', '2026-11-01T08:00:00Z', false]] as [$n, $date, $withImage]) {
    $image = $withImage ? '<p><img src="http://127.0.0.1:' . $port . '/pixel.png"/></p>' : '';
    $entries .= "<entry><title>契约第 {$n} 条</title><link rel=\"alternate\" type=\"text/html\" href=\"http://example.test/{$n}\"/><id>tag:example.test,2026:{$n}</id><published>{$date}</published><updated>{$date}</updated><content type=\"html\"><![CDATA[<p>第 {$n} 条正文内容。</p>{$image}]]></content></entry>";
}
file_put_contents($fixtureDir . '/atom.xml', '<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>契约测试源</title><id>tag:example.test,2026:feed</id><updated>2026-11-03T08:00:00Z</updated>' . $entries . '</feed>');
copy(dirname(__DIR__) . '/tests/fixtures/rss/pixel.png', $fixtureDir . '/pixel.png');
$server = proc_open([PHP_BINARY, '-S', "127.0.0.1:{$port}", '-t', $fixtureDir], [['pipe', 'r'], ['file', '/dev/null', 'w'], ['file', '/dev/null', 'w']], $pipes);
$probe = function (string $url): bool {
    $ch = curl_init($url);
    curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_TIMEOUT => 2]);
    $body = curl_exec($ch);
    $ok = curl_getinfo($ch, CURLINFO_RESPONSE_CODE) === 200 && str_contains((string)$body, '<feed');
    curl_close($ch);
    return $ok;
};
for ($i = 0; $i < 40 && !$probe("http://127.0.0.1:{$port}/atom.xml"); $i++) usleep(100000);
if ($i === 40) { proc_terminate($server); exit("夹具服务未启动\n"); }

require $sandbox . '/app.php';

function verify(bool $condition, string $message): void {
    if (!$condition) throw new RuntimeException($message);
}

$count = fn(string $sql): int => (int)$db->query($sql)->fetchColumn();
try {
    verify($count("SELECT COUNT(*) FROM rss_feeds") === 0, '订阅表未初始化为空');
    verify(feed_source_filter($db) === '', '停用插件时时间线被过滤');
    verify(!isset(public_plugins($db)['rss']['page']), '停用插件的页面信息公开');
    verify(!isset(plugin_pages($db)['/rss']), '停用插件页面仍公开');

    save_setting($db, 'plugin:rss:enabled', '1');
    save_setting($db, 'plugin:rss:max_items', '2');
    verify(isset(plugin_pages($db)['/rss']), '启用后页面未注册');
    verify(feed_source_filter($db) === " AND p.source NOT IN ('rss')", '默认未并入时间线');
    verify(public_plugins($db)['rss']['page'] === ['route' => '/rss', 'title' => 'RSS 订阅', 'tab' => true], '首页标签声明不完整');
    save_setting($db, 'plugin:rss:in_feed', '1');
    verify(feed_source_filter($db) === '', '开启并入后时间线仍被过滤');
    save_setting($db, 'plugin:rss:in_feed', '0');

    $plugin = plugin_list()['rss'];
    $config = plugin_config($db, 'rss');
    verify((string)$config['in_feed'] === '0' && (int)$config['history_limit'] === 30, '插件配置默认值未生效');

    verify((string)$config['auto_delete'] === '1' && (int)$config['retention_days'] === 30, '自动删除默认值错误');
    save_setting($db, 'plugin:rss:auto_delete', '0');

    // 添加订阅：验证源 → 登记 → 立即同步 max_items 条
    $added = ($plugin['admin_op'])($db, 'add', ['url' => "http://127.0.0.1:{$port}/atom.xml"], plugin_config($db, 'rss'));
    verify($added['added'] === 2, '添加时未按 max_items 导入');
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 2, '订阅动态未入库');
    verify($added['title'] === '契约测试源', '源内标题未发现');
    $feedId = $added['id'];
    // 正文图片走内容衍生 URL 通道：默认阻断内网（SSRF 防护），夹具的 127.0.0.1 图片应被过滤
    $first = $db->query("SELECT images FROM posts WHERE source = 'rss' ORDER BY created_at DESC, id DESC LIMIT 1")->fetchColumn();
    verify(json_decode((string)$first, true) === [], '内容衍生内网图片未被过滤');
    try { ($plugin['admin_op'])($db, 'add', ['url' => "http://127.0.0.1:{$port}/atom.xml"], plugin_config($db, 'rss')); verify(false, '重复订阅未被拒绝'); }
    catch (InvalidArgumentException $e) { verify(str_contains($e->getMessage(), '已在列表'), '重复订阅报错信息异常'); }

    // 再次同步：幂等去重
    verify(($plugin['admin_op'])($db, 'sync', ['feed_id' => $feedId], plugin_config($db, 'rss'))['added'] === 0, '重复同步未去重');

    // 墓碑：删除一条（模拟用户删除动态——立墓碑并清理其图片），之后同步不得复活
    $stale = $db->query("SELECT id, source_key, images FROM posts WHERE source = 'rss' ORDER BY created_at DESC, id DESC LIMIT 1")->fetch(PDO::FETCH_ASSOC);
    $db->prepare('DELETE FROM posts WHERE id = ?')->execute([$stale['id']]);
    foreach (json_decode($stale['images'], true) ?: [] as $ref) delete_image_file($ref);
    $db->prepare("INSERT OR IGNORE INTO plugin_tombstones (key, plugin, deleted_at) VALUES (?, 'rss', ?)")->execute([$stale['source_key'], date(DATE_ATOM)]);
    $items = ($plugin['sync'])($db, plugin_config($db, 'rss'));
    verify(plugin_sync_items($db, 'rss', $items) === 0, '墓碑条目被复活');
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 1, '墓碑后数量变化');

    // 历史上限：all 模式按 history_limit 每源取回
    save_setting($db, 'plugin:rss:history_limit', '3');
    $items = ($plugin['sync'])($db, plugin_config($db, 'rss'), ['all' => true]);
    verify(plugin_sync_items($db, 'rss', $items) === 1, '历史同步未补齐条目');
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 2, '历史同步后数量不符');

    // 来源身份：favicon 拉取（Google s2）在沙箱不可达且本地地址被 SSRF 防护拦截，
    // 直接注入 up: 引用验证链路——已有源带图标时，新收集的条目烙上 avatar/handle/name
    if (!is_dir(DATA_DIR . '/uploads')) mkdir(DATA_DIR . '/uploads', 0755, true);
    copy(dirname(__DIR__) . '/tests/fixtures/rss/pixel.png', DATA_DIR . '/uploads/' . ($iconName = bin2hex(random_bytes(16)) . '.png'));
    $db->exec("UPDATE rss_feeds SET icon = " . $db->quote('up:' . $iconName));
    $db->exec("DELETE FROM posts WHERE id = (SELECT id FROM posts WHERE source = 'rss' ORDER BY created_at DESC, id DESC LIMIT 1)");
    $items = ($plugin['sync'])($db, plugin_config($db, 'rss'));
    verify(plugin_sync_items($db, 'rss', $items) === 1, '来源身份测试：条目未重新收集');
    $fresh = json_decode((string)$db->query("SELECT meta FROM posts WHERE source = 'rss' ORDER BY id DESC LIMIT 1")->fetchColumn(), true) ?: [];
    verify(($fresh['avatar'] ?? '') === 'up:' . $iconName && ($fresh['handle'] ?? '') === '127.0.0.1' && ($fresh['name'] ?? '') === '契约测试源', '新条目未带来源身份三件套');
    verify(!isset(json_decode((string)$db->query("SELECT meta FROM posts WHERE source = 'rss' ORDER BY id ASC LIMIT 1")->fetchColumn(), true)['avatar']), '未重新收集的旧条目被意外回填');
    verify(post_avatar($db, $fresh) !== null, 'post_avatar 未解析图标引用');
    $_GET = ['limit' => '30'];
    $resolved = array_values(array_filter(($plugin['data'])($db, false)['posts'], fn($p) => ($p['meta']['avatar'] ?? null) !== null));
    verify(count($resolved) === 1 && is_array($resolved[0]['meta']['avatar']) && str_contains((string)$resolved[0]['meta']['avatar']['url'], '/image?upload='), '数据端点未把身份图标解析为描述符');
    $_GET = [];

    // 数据端点：源列表 + 分页 + 按源筛选
    $_GET = ['limit' => '1'];
    $data = ($plugin['data'])($db, true);
    verify(count($data['posts']) === 1 && $data['total'] === 2 && is_string($data['next_cursor']), '数据端点分页失效');
    verify($data['feeds'][0]['count'] === 2 && isset($data['feeds'][0]['url'], $data['feeds'][0]['last_error']), '管理员源信息缺失');
    verify(isset($data['posts'][0]['images']) && is_array($data['posts'][0]['images']) && isset($data['posts'][0]['meta']['card'], $data['posts'][0]['like_count']), '动态数据形状与 api_posts 不一致');
    $_GET = ['limit' => '30', 'feed' => $feedId];
    verify(count(($plugin['data'])($db, false)['posts']) === 2, '按源筛选失效');
    $_GET = ['limit' => '30', 'feed' => 'zzzzzzzzzzzz'];
    verify(count(($plugin['data'])($db, false)['posts']) === 2, '非法源标识未被忽略');
    $_GET = ['limit' => '30', 'cursor' => '!!!'];
    verify(count(($plugin['data'])($db, false)['posts']) === 2, '非法游标未被忽略');
    $_GET = [];

    // 首页时间线：并入关闭时 rss 动态退出、普通动态不受影响
    $db->exec("INSERT INTO posts (content, images, created_at, updated_at) VALUES ('普通动态', '[]', '2026-11-02T00:00:00+00:00', '2026-11-02T00:00:00+00:00')");
    verify($count('SELECT COUNT(*) FROM posts p WHERE 1 = 1' . feed_source_filter($db)) === 1, '时间线过滤把普通动态也滤掉了');

    // 退订（保留动态）：动态与墓碑保留，重订不重复
    ($plugin['admin_op'])($db, 'remove', ['feed_id' => $feedId], plugin_config($db, 'rss'));
    verify($count('SELECT COUNT(*) FROM rss_feeds') === 0, '退订后源未移除');
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 2, '退订误删了保留的动态');
    verify($count("SELECT COUNT(*) FROM plugin_tombstones WHERE plugin = 'rss'") === 1, '退订误清墓碑');
    ($plugin['admin_op'])($db, 'add', ['url' => "http://127.0.0.1:{$port}/atom.xml"], plugin_config($db, 'rss'));
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 2, '重订后墓碑条目或已有条目被重复导入');

    // 退订（连带删除）：动态 / 评论 / 点赞 / 墓碑 / 图片文件一并清理（现存条目已无图片，仅验证数据面）
    $feedId = $db->query('SELECT id FROM rss_feeds')->fetchColumn();
    ($plugin['admin_op'])($db, 'remove', ['feed_id' => $feedId, 'purge' => 1], plugin_config($db, 'rss'));
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 0 && $count("SELECT COUNT(*) FROM plugin_tombstones WHERE plugin = 'rss'") === 0, '清史退订未删净');
    verify(glob(DATA_DIR . '/uploads/*') === [], '清史退订后仍残留图片文件');

    // 静态导出：启用时打包订阅页数据与媒体；停用时返回空（动态回到时间线由核心解析）
    ($plugin['admin_op'])($db, 'add', ['url' => "http://127.0.0.1:{$port}/atom.xml"], plugin_config($db, 'rss'));
    // 直接给一条动态挂上 up: 引用与来源身份（模拟公网图片 / favicon 已入库），验证导出复制媒体与身份解析
    if (!is_dir(DATA_DIR . '/uploads')) mkdir(DATA_DIR . '/uploads', 0755, true);
    copy(dirname(__DIR__) . '/tests/fixtures/rss/pixel.png', DATA_DIR . '/uploads/' . ($uploadName = bin2hex(random_bytes(16)) . '.png'));
    $db->exec('UPDATE posts SET images = ' . $db->quote('["up:' . $uploadName . '"]') . " WHERE id = (SELECT id FROM posts WHERE source = 'rss' ORDER BY id LIMIT 1)");
    $metaRow = $db->query("SELECT id, meta FROM posts WHERE source = 'rss' ORDER BY id LIMIT 1")->fetch(PDO::FETCH_ASSOC);
    $meta = json_decode($metaRow['meta'] ?: '[]', true) ?: [];
    $meta['avatar'] = 'up:' . $uploadName;
    $db->prepare('UPDATE posts SET meta = ? WHERE id = ?')->execute([json_encode($meta, JSON_UNESCAPED_UNICODE), $metaRow['id']]);
    $dest = $dataDir . '/export';
    $files = [];
    $export = ($plugin['export'])($db, $dest, $files, true);
    verify(count($export['items']['posts']) === 2 && count($export['items']['feeds']) === 1, '导出条目不完整');
    verify(str_contains($export['html'], 'rss-static-item') && str_contains($export['html'], '契约'), '无 JS 页面缺失');
    $exported = array_values(array_filter($export['items']['posts'], fn($p) => is_array($p['meta']['avatar'] ?? null)));
    verify(count($exported) === 1 && str_contains((string)$exported[0]['meta']['avatar']['url'], '/media/uploads/'), '导出未解析来源身份图标');
    verify(substr_count($export['html'], 'class="avatar"') === 1, '无 JS 页面缺少 favicon 头像');
    if (shell_exec('command -v magick') !== null) {
        verify(count(array_unique($files)) === 2 && is_file($dest . '/media/uploads/' . $uploadName) && is_file($dest . '/thumbnails/up-' . $uploadName . '.webp'), '导出媒体未复制');
    }
    // 使用复杂噪声图验证字节硬上限，并检查过期清理的全部图片文件。
    require dirname(__DIR__) . '/plugins/rss/plugin.php';
    $noise = $dataDir . '/noise.png';
    exec('magick -size 1200x1200 plasma:fractal ' . escapeshellarg($noise), $output, $code);
    verify($code === 0, '测试图片生成失败');
    $rss_compress($noise, DATA_DIR . '/uploads/' . $uploadName, 100 * 1024, 1600);
    if (!is_dir(DATA_DIR . '/rss-thumbs')) mkdir(DATA_DIR . '/rss-thumbs');
    $rssThumb = DATA_DIR . '/rss-thumbs/' . $uploadName . '.webp';
    $rss_compress(DATA_DIR . '/uploads/' . $uploadName, $rssThumb, 50 * 1024, 800);
    verify(filesize(DATA_DIR . '/uploads/' . $uploadName) < 100 * 1024 && filesize($rssThumb) < 50 * 1024, 'RSS 图片超出大小限制');
    if (!is_dir(DATA_DIR . '/thumbs')) mkdir(DATA_DIR . '/thumbs');
    $imagePath = DATA_DIR . '/uploads/' . $uploadName;
    $cachedThumb = DATA_DIR . '/thumbs/' . hash('sha256', $imagePath . filemtime($imagePath) . filesize($imagePath)) . '.webp';
    exec(thumb_magick($imagePath, 'webp:' . $cachedThumb), $output, $code);
    verify($code === 0 && filesize($cachedThumb) < 50 * 1024, '图片接口未使用受限缩略图');
    $db->exec("UPDATE posts SET created_at = datetime('now', '-31 days') WHERE source = 'rss'");
    $rss_cleanup($db, ['auto_delete' => '0']);
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 2, '关闭自动删除仍删除条目');
    $rss_cleanup($db, ['auto_delete' => '1', 'retention_days' => 60]);
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 2, '自定义保留天数未生效');
    $rss_cleanup($db, []);
    verify($count("SELECT COUNT(*) FROM posts WHERE source = 'rss'") === 0, '过期条目未删除');
    verify(!is_file($imagePath) && !is_file($rssThumb) && !is_file($cachedThumb), '自动删除残留原图或缩略图');
    verify($count("SELECT COUNT(*) FROM posts WHERE source = ''") === 1, '自动删除影响普通动态');
    $xml = file_get_contents($fixtureDir . '/atom.xml');
    file_put_contents($fixtureDir . '/atom.xml', preg_replace('/2026-11-0[123]T08:00:00Z/', gmdate('Y-m-d\\TH:i:s\\Z', time() - 31 * 86400), $xml));
    $items = ($plugin['sync'])($db, ['auto_delete' => '1', 'retention_days' => 1]);
    verify($items === [], '过期订阅条目重新导入');

    save_setting($db, 'plugin:rss:enabled', '0');
    verify(($plugin['export'])($db, $dest, $files, false)['items'] === [], '停用后仍导出订阅页数据');

    echo "rss-plugin: 时间线可见性、多源同步、墓碑、历史上限、退订清史、数据端点与导出通过\n";
} finally {
    proc_terminate($server);
    proc_close($server);
}
