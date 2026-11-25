<?php
declare(strict_types=1);

$sandbox = $argv[1];
define('DATA_DIR', $sandbox . '/spawn-data');
mkdir(DATA_DIR);
require $sandbox . '/src/Netlify.php';
$handles = [];
for ($i = 0; $i < 24; $i++) {
    $handle = fopen(DATA_DIR . '/fd-' . $i, 'c');
    flock($handle, LOCK_EX);
    $handles[] = $handle;
}
$script = DATA_DIR . '/child.php';
file_put_contents($script, '<?php file_put_contents(__DIR__ . "/started", "1"); $end = microtime(true) + 10; while (!is_file(__DIR__ . "/stop") && microtime(true) < $end) usleep(20000);');
try {
    deploy_spawn($script);
    foreach ($handles as $handle) fclose($handle);
    $deadline = microtime(true) + 5;
    while (!is_file(DATA_DIR . '/started') && microtime(true) < $deadline) usleep(20000);
    if (!is_file(DATA_DIR . '/started')) throw new RuntimeException('后台部署进程未启动');
    for ($i = 0; $i < 24; $i++) {
        $handle = fopen(DATA_DIR . '/fd-' . $i, 'c');
        if (!flock($handle, LOCK_EX | LOCK_NB)) throw new RuntimeException('后台进程仍继承文件描述符锁：' . $i);
        fclose($handle);
    }
    echo "deploy-spawn: 多位文件描述符下正常启动，继承锁全部释放\n";
} finally {
    file_put_contents(DATA_DIR . '/stop', '1');
}
