<?php
// 一键部署到 Netlify：php bin/deploy-netlify.php
// 后台「一键部署」按钮派生的也是本进程：任务独立于页面运行，全程写 var/deploy.status.json 供前端轮询。
require dirname(__DIR__) . '/app.php';
if ((string)opt('netlify_token') === '' || (string)opt('netlify_site') === '') {
    netlify_status_write(['state' => 'error', 'error' => '尚未配置 Netlify：请先在后台保存访问令牌和站点 ID']);
    fwrite(STDERR, "尚未配置 Netlify：请先在后台「静态发布 → 一键部署」保存访问令牌和站点 ID。\n");
    exit(1);
}
exit(netlify_deploy_run($db) ? 0 : 1);
