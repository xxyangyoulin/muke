// PM2 仅守护后台任务；Web 由 Nginx + PHP-FPM 提供。
const name = process.env.MKBLOG_NAME || 'mkblog';
const php = process.env.PHP_BIN || '/usr/bin/php';

module.exports = {
  apps: [{
    name: `${name}-sync`,
    cwd: __dirname,
    script: php,
    args: ['bin/sync.php', '--watch'],
    interpreter: 'none',
    autorestart: true,
    restart_delay: 2000,
    time: true,
    out_file: './var/sync-out.log',
    error_file: './var/sync-error.log'
  }]
};
