<?php
// 环境自检：php bin/check.php
$fail = 0;
echo 'MUKE ' . trim(file_get_contents(__DIR__ . '/../VERSION')) . "\n";
function check(string $name, bool $ok, string $hint = ''): void {
    global $fail;
    echo ($ok ? '[ OK ] ' : '[FAIL] ') . $name . ($ok || $hint === '' ? '' : " —— {$hint}") . "\n";
    if (!$ok) $fail++;
}

check('PHP 版本 >= 8.1', PHP_VERSION_ID >= 80100, '当前 ' . PHP_VERSION);
foreach (['pdo_sqlite', 'sqlite3', 'curl', 'mbstring', 'fileinfo', 'json', 'zip'] as $ext) {
    check("扩展 {$ext}", extension_loaded($ext), '请在 php.ini 中启用');
}
$magick = trim((string)shell_exec('command -v magick 2>/dev/null'));
check('ImageMagick (magick)', $magick !== '', '缩略图需要；Arch: pacman -S imagemagick');
check('项目目录可写', is_writable(__DIR__ . '/..'), 'var/ 与 dist/ 需要写权限');
@mkdir(__DIR__ . '/../var', 0755, true);
check('数据目录 var/ 可写', is_writable(__DIR__ . '/../var'));
check('插件目录 plugins/ 存在', is_dir(__DIR__ . '/../plugins'));
check('文档根为 public/', is_file(__DIR__ . '/../public/index.php'));

echo $fail === 0 ? "\n环境就绪 ✓\n" : "\n有 {$fail} 项未通过\n";
exit($fail === 0 ? 0 : 1);
