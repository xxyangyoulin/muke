// PM2 进程配置，可用环境变量调整：MKBLOG_NAME、MKBLOG_HOST、MKBLOG_PORT、PHP_BIN。
const name = process.env.MKBLOG_NAME || 'mkblog';
const host = process.env.MKBLOG_HOST || '0.0.0.0';
const port = process.env.MKBLOG_PORT || '8765';
const php = process.env.PHP_BIN || '/usr/bin/php';

module.exports = {
  apps: [{
    name,
    cwd: __dirname,
    script: php,
    args: [
      '-S', `${host}:${port}`,
      '-d', 'upload_max_filesize=16M',
      '-d', 'post_max_size=68M',
      '-t', 'public',
      'public/index.php'
    ],
    interpreter: 'none',
    autorestart: true,
    restart_delay: 2000,
    max_restarts: 10,
    time: true,
    out_file: './var/server-out.log',
    error_file: './var/server-error.log'
  }]
};
