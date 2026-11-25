<?php
declare(strict_types=1);

/** Content-Length 必发：响应自定界，客户端收完即完成——不依赖连接关闭（后台派生进程可能短暂持有连接 socket） */
function json_response(array $data, int $status = 200): never {
    $body = json_encode($data, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
    http_response_code($status);
    header('Content-Type: application/json; charset=utf-8');
    header('Content-Length: ' . strlen($body));
    header('Cache-Control: no-store');
    echo $body;
    exit;
}

function http_fetch(string $url, int $timeout = 20, array $headers = [], bool $allowPrivate = false): string {
    $base = ['User-Agent: Mozilla/5.0', 'Accept-Language: zh-CN,zh;q=0.9'];
    foreach ($headers as $name => $value) $base[] = "$name: $value";
    for ($redirect = 0; $redirect <= 5; $redirect++) {
        $parts = parse_url($url);
        $scheme = strtolower($parts['scheme'] ?? '');
        $host = trim($parts['host'] ?? '', '[]');
        if (!in_array($scheme, ['http', 'https'], true) || $host === '' || isset($parts['user']) || isset($parts['pass'])) throw new InvalidArgumentException('仅允许无凭据的 HTTP/HTTPS 地址');
        $port = (int)($parts['port'] ?? ($scheme === 'https' ? 443 : 80));
        $ip = filter_var($host, FILTER_VALIDATE_IP) ? $host : gethostbyname($host);
        if (!filter_var($ip, FILTER_VALIDATE_IP)) throw new RuntimeException('主机解析失败');
        if (!$allowPrivate && (!filter_var($ip, FILTER_VALIDATE_IP, FILTER_FLAG_NO_PRIV_RANGE | FILTER_FLAG_NO_RES_RANGE) || str_starts_with(strtolower($ip), '::ffff:'))) throw new RuntimeException('目标地址为内网/保留地址，已阻断');
        $address = str_contains($ip, ':') ? "[$ip]" : $ip;
        $resolveHost = str_contains($host, ':') ? "[$host]" : $host;
        $ch = curl_init($url);
        $proxy = (string)opt('http_proxy');
        $options = [
            CURLOPT_RETURNTRANSFER => true, CURLOPT_TIMEOUT => $timeout,
            CURLOPT_FOLLOWLOCATION => false, CURLOPT_HTTPHEADER => $base,
            CURLOPT_PROTOCOLS => CURLPROTO_HTTP | CURLPROTO_HTTPS,
            CURLOPT_PROXY => $proxy,
            CURLOPT_RESOLVE => ["$resolveHost:$port:$address"],
        ];
        // 代理也连接已校验的 IP，避免代理重新解析域名绕过检查。
        if ($proxy !== '') {
            $options[CURLOPT_HTTPPROXYTUNNEL] = true;
            $options[CURLOPT_CONNECT_TO] = ["$resolveHost:$port:$address:$port"];
        }
        curl_setopt_array($ch, $options);
        try {
            for ($attempt = 1; ; $attempt++) {
                $body = curl_exec($ch);
                if ($body !== false || $attempt >= 3) break;
                sleep($attempt);
            }
            if ($body === false) throw new RuntimeException('远程请求失败：' . curl_error($ch));
            $status = (int)curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
            $next = (string)curl_getinfo($ch, CURLINFO_REDIRECT_URL);
            if ($status >= 300 && $status < 400 && $next !== '') {
                if ($redirect === 5) throw new RuntimeException('重定向次数过多');
                $nextParts = parse_url($next);
                $nextPort = $nextParts['port'] ?? (strtolower($nextParts['scheme'] ?? '') === 'https' ? 443 : 80);
                if (strtolower($nextParts['host'] ?? '') !== strtolower((string)$parts['host']) || strtolower($nextParts['scheme'] ?? '') !== $scheme || $nextPort !== $port) $base = ['User-Agent: Mozilla/5.0', 'Accept-Language: zh-CN,zh;q=0.9'];
                $url = $next;
                continue;
            }
            if ($status >= 400) throw new RuntimeException('远程请求失败（HTTP ' . $status . '）'); // 勿用 "$status）" 插值：全角括号首字节会被变量名解析吞掉
            return (string)$body;
        } finally { curl_close($ch); }
    }
    throw new RuntimeException('重定向失败');
}


/** application/x-www-form-urlencoded POST（如 Akismet comment-check），返回响应体或空串 */
function http_post_form(string $url, array $fields, int $timeout = 15): string {
    $ch = curl_init($url);
    $proxy = (string)opt('http_proxy');
    $options = [CURLOPT_RETURNTRANSFER => true, CURLOPT_TIMEOUT => $timeout, CURLOPT_POST => true,
        CURLOPT_POSTFIELDS => http_build_query($fields),
        CURLOPT_HTTPHEADER => ['User-Agent: mkblog/1.0', 'Content-Type: application/x-www-form-urlencoded']];
    if ($proxy !== '') $options[CURLOPT_PROXY] = $proxy;
    curl_setopt_array($ch, $options);
    for ($attempt = 1; ; $attempt++) {
        $body = curl_exec($ch);
        if ($body !== false && $body !== '') { curl_close($ch); return (string)$body; }
        if ($attempt >= 2) { curl_close($ch); return ''; }
        usleep(500000);
    }
}
