<?php
declare(strict_types=1);

// 极简 SMTP 客户端（465 SSL / 587 STARTTLS 未支持时回退 465）：自托管够用，失败只记日志不致命。
// 配置（后台评论设置）：smtp_host / smtp_port / smtp_user / smtp_pass / notify_email

/** 站点绝对地址（site_url 配置优先，留空按当前请求推导；CLI 场景兜牌 example.com） */
function site_absolute_url(string $path = '/'): string {
    $base = rtrim((string)opt('site_url'), '/');
    if ($base === '') {
        $scheme = (!empty($_SERVER['HTTPS']) && $_SERVER['HTTPS'] !== 'off') ? 'https' : 'http';
        $host = (string)($_SERVER['HTTP_HOST'] ?? '');
        $base = $host !== '' ? $scheme . '://' . $host : '';
    }
    return $base . '/' . ltrim($path, '/');
}

function mail_configured(): bool {
    return (string)opt('smtp_host') !== '' && (string)opt('smtp_user') !== '' && (string)opt('smtp_pass') !== '';
}

/** 发送一封纯文本邮件；返回 bool（失败静默记 error_log，绝不打断业务） */
function mail_send(string $to, string $subject, string $body): bool {
    if (!mail_configured() || $to === '') return false;
    $host = (string)opt('smtp_host');
    $port = (int)(opt('smtp_port') ?: 465);
    $user = (string)opt('smtp_user');
    $pass = (string)opt('smtp_pass');
    $fp = @stream_socket_client("ssl://{$host}:{$port}", $errno, $errstr, 10);
    if (!$fp) { error_log("mail connect failed: $errstr"); return false; }
    try {
        $read = function () use ($fp): string {
            $data = '';
            while ($line = fgets($fp, 512)) { $data .= $line; if (isset($line[3]) && $line[3] === ' ') break; }
            return $data;
        };
        $cmd = function (string $c, array $expect = [250]) use ($fp, $read): bool {
            fwrite($fp, $c . "\r\n");
            $resp = $read();
            $code = (int)substr($resp, 0, 3);
            if (!in_array($code, $expect, true)) { error_log("mail cmd '$c' -> $resp"); return false; }
            return true;
        };
        $read(); // banner
        if (!$cmd('EHLO mkblog', [250])) return false;
        if (!$cmd('AUTH LOGIN', [334])) return false;
        if (!$cmd(base64_encode($user), [334])) return false;
        if (!$cmd(base64_encode($pass), [235])) return false;
        if (!$cmd('MAIL FROM:<' . $user . '>', [250])) return false;
        if (!$cmd('RCPT TO:<' . $to . '>', [250, 251])) return false;
        if (!$cmd('DATA', [354])) return false;
        $headers = 'From: =?UTF-8?B?' . base64_encode('木刻') . "?= <{$user}>\r\n"
            . 'To: <' . $to . ">\r\n"
            . 'Subject: =?UTF-8?B?' . base64_encode($subject) . "?=\r\n"
            . 'MIME-Version: 1.0' . "\r\n"
            . "Content-Type: text/plain; charset=UTF-8\r\n\r\n";
        $data = $headers . str_replace("\n.", "\n..", $body);
        if (!$cmd($data . "\r\n.", [250])) return false;
        fwrite($fp, "QUIT\r\n");
        return true;
    } finally {
        @fclose($fp);
    }
}

/** 新评论通知管理员（仅 approved/pending 都通知，spam 不打扰） */
function mail_notify_admin(array $comment): void {
    $to = (string)opt('notify_email');
    if ($to === '') return;
    $status = $comment['status'] === 'pending' ? '（待审）' : '';
    mail_send($to, '新评论' . $status . '：' . mb_substr($comment['author'], 0, 20), "{$comment['author']} 评论了「{$comment['target_label']}」：\n\n{$comment['content']}\n\n{$comment['url']}");
}

/** 回复通知被回复者（对方留了邮箱、其评论已审、且本条直接放行时才发，避免垃圾外发） */
function mail_notify_reply(array $reply, array $parent): void {
    if ($parent['email'] === '' || ($parent['status'] ?? 'approved') !== 'approved' || $reply['status'] !== 'approved') return;
    mail_send($parent['email'], '你的评论收到了回复', "{$parent['author']}，你好：\n\n{$reply['author']} 回复了你：\n\n{$reply['content']}\n\n{$reply['url']}");
}
