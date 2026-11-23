<?php
declare(strict_types=1);

// 静态导出代码压缩（仅作用于 dist/ 产物，源文件不动）。三个字符串感知的扫描器：
//   minify_css —— 去注释、空白压缩（结构符旁空白剥离，calc()/and 等语义空白保留）、;} 折叠
//   minify_js  —— 去注释（块注释按原换行情况替换为空格/换行，保 ASI 语义）、缩进/行尾空白/空行删除，
//                 模板串 ${} 嵌套与正则字面量整段识别，保留换行结构（不冒险改写 ASI）
//   minify_html—— 去注释（保留 [if 条件注释）、文本空白折叠、标签内空白折叠；
//                 pre/textarea/非 JS type 的 script 原样，script(JS)/style 内容交给对应压缩器

/** CSS 压缩：仅剥离结构符（{};:,）旁空白——calc(100% - 20px)、"@media (a) and (b)" 等必需空白不受影响 */
function minify_css(string $css): string {
    $out = '';
    $len = strlen($css);
    $i = 0;
    $pendingSpace = false;
    $last = ''; // 上一输出字符
    while ($i < $len) {
        $c = $css[$i];
        if ($c === '/' && $i + 1 < $len && $css[$i + 1] === '*') { // 注释 → 单空格（可作 token 分隔，如 a/**/b 是后代选择器）
            $end = strpos($css, '*/', $i + 2);
            if ($end === false) break;
            $i = $end + 2;
            $pendingSpace = true;
            continue;
        }
        if ($c === "'" || $c === '"') { // 字符串原样（content:"a b" 等）
            $quote = $c;
            $j = $i + 1;
            while ($j < $len) {
                if ($css[$j] === '\\') { $j += 2; continue; }
                if ($css[$j] === $quote) { $j++; break; }
                $j++;
            }
            $out .= substr($css, $i, $j - $i);
            $last = $css[$j - 1];
            $i = $j;
            $pendingSpace = false;
            continue;
        }
        if ($c === ' ' || $c === "\t" || $c === "\n" || $c === "\r" || $c === "\f") { $pendingSpace = true; $i++; continue; }
        if ($pendingSpace) {
            // 仅当空白紧邻结构符时可安全省略（两侧都不是才输出一个空格）
            if (strpos('{;:,', $last) === false && strpos('{};:,', $c) === false) $out .= ' ';
            $pendingSpace = false;
        }
        $out .= $c;
        $last = $c;
        $i++;
    }
    $out = str_replace(';}', '}', $out);
    return trim($out);
}

/** JS 压缩：保换行（ASI 安全）。识别字符串/模板串（含 ${} 嵌套表达式）/正则字面量/行块注释 */
function minify_js(string $js): string {
    $len = strlen($js);
    $i = 0;
    $out = '';
    $pendingSpace = false;   // 待定空格（token 间）
    $lastChar = '';          // 上一输出的非空白字符（供正则判定）
    $lastWord = '';          // 上一完整标识符（关键字判定用）
    $wordPrevChar = '';      // 该标识符的前驱字符（排除 .prop 形式）
    $prevNonWord = '';       // word 攒取中的上一字符
    $emit = function (string $s) use (&$out, &$lastChar, &$lastWord, &$wordPrevChar, &$prevNonWord): void {
        if ($s === '') return;
        $out .= $s;
        for ($k = 0; $k < strlen($s); $k++) {
            $ch = $s[$k];
            if ($ch === ' ' || $ch === "\t" || $ch === "\n" || $ch === "\r") { $prevNonWord = $ch; continue; }
            $isWord = ($ch >= 'a' && $ch <= 'z') || ($ch >= 'A' && $ch <= 'Z') || ($ch >= '0' && $ch <= '9') || $ch === '_' || $ch === '$';
            if ($isWord) { $lastWord .= $ch; }
            else { if ($lastWord !== '') { $wordPrevChar = $prevNonWord; } $lastWord = ''; }
            $prevNonWord = $ch;
            $lastChar = $ch;
        }
    };
    $flushSpace = function (string $nextChar) use (&$pendingSpace, &$emit, &$out, &$lastChar): void {
        if (!$pendingSpace) return;
        $pendingSpace = false;
        if ($out === '' || substr($out, -1) === "\n" || $nextChar === '') return; // 行首缩进丢弃
        // 符号旁空白删除（token 间空白在符号旁无语义）；换行不在此处理（ASI 语义单独保留）
        if (strpos('=([{,;:!&|?+-*%^~<>)]}', $lastChar) !== false || strpos('=([{,;:!&|?+-*%^~<>)]}', $nextChar) !== false) return;
        $emit(' ');
    };
    $regexAllowed = function () use (&$lastChar, &$lastWord, &$wordPrevChar): bool {
        if ($lastChar === '' ) return true; // 文件起始
        if (strpos('=([{,;:!&|?+-*%^~<>', $lastChar) !== false) return true;
        static $keywords = ['return','typeof','instanceof','in','of','new','delete','void','throw','case','do','else','yield','await'];
        return in_array($lastWord, $keywords, true) && $wordPrevChar !== '.';
    };
    while ($i < $len) {
        $c = $js[$i];
        // 空白：攒 pendingSpace；换行直接输出（保留 ASI 判定所需的换行结构）
        if ($c === ' ' || $c === "\t" || $c === "\r") { $pendingSpace = true; $i++; continue; }
        if ($c === "\n") {
            $pendingSpace = false; // 行尾空白丢弃
            if (substr($out, -1) === "\n") { $i++; continue; } // 空行折叠为单行
            if (strpos(';{,', $lastChar) !== false) { $i++; continue; } // 行尾 ; { , 已终结 token，换行可安全删除
            $emit("\n");
            $i++;
            continue;
        }
        // 行注释：删除至行尾（保留换行语义）
        if ($c === '/' && $i + 1 < $len && $js[$i + 1] === '/') {
            $j = strpos($js, "\n", $i);
            $i = $j === false ? $len : $j;
            continue;
        }
        // 块注释：按原换行替换为换行或空格（注释可作 token 分隔；含换行时参与 ASI）
        if ($c === '/' && $i + 1 < $len && $js[$i + 1] === '*') {
            $end = strpos($js, '*/', $i + 2);
            if ($end === false) break;
            $hadNewline = substr_count(substr($js, $i, $end - $i), "\n") > 0;
            if ($hadNewline) { if (substr($out, -1) !== "\n") $emit("\n"); }
            else { $pendingSpace = true; }
            $i = $end + 2;
            continue;
        }
        // 字符串
        if ($c === "'" || $c === '"') {
            $flushSpace($c);
            $j = $i + 1;
            while ($j < $len) {
                if ($js[$j] === '\\') { $j += 2; continue; }
                if ($js[$j] === $c) { $j++; break; }
                if ($js[$j] === "\n") break; // 未闭合字符串由语法错误兜底，避免吃掉全文
                $j++;
            }
            $emit(substr($js, $i, $j - $i));
            $i = $j;
            continue;
        }
        // 模板串（含 ${} 嵌套表达式：回到主循环按代码规则扫描，深度归零后继续模板部分）
        if ($c === '`') {
            $flushSpace($c);
            $emit('`');
            $i++;
            $inTpl = 0;
            while ($i < $len) {
                $t = $js[$i];
                if ($inTpl === 0) {
                    if ($t === '\\') { $emit($t . ($js[$i + 1] ?? '')); $i += 2; continue; }
                    if ($t === '`') { $emit('`'); $i++; break; }
                    if ($t === '$' && ($js[$i + 1] ?? '') === '{') { $emit('${'); $i += 2; $inTpl = 1; continue; }
                    $emit($t); $i++; continue;
                }
                // ${ 表达式内部：整体按原始字符透传（含空白与注释，正确性优先），仅做 {} 平衡计数；
                // 字符串与嵌套模板串整段透传，避免其内部的 } 干扰计数
                if ($t === '{') { $inTpl++; $emit('{'); $i++; continue; }
                if ($t === '}') { $inTpl--; $emit('}'); $i++; continue; } // 归零后由上方 $inTpl===0 分支继续模板部分
                if ($t === "'" || $t === '"') {
                    $q = $t; $j = $i + 1;
                    while ($j < $len) {
                        if ($js[$j] === '\\') { $j += 2; continue; }
                        if ($js[$j] === $q || $js[$j] === "\n") { $j++; break; }
                        $j++;
                    }
                    $emit(substr($js, $i, $j - $i));
                    $i = $j;
                    continue;
                }
                if ($t === '`') { // 嵌套模板串整段透传（内部 ${} 一并原样）
                    $j = $i + 1; $depth = 0;
                    while ($j < $len) {
                        $n = $js[$j];
                        if ($n === '\\') { $j += 2; continue; }
                        if ($n === '$' && ($js[$j + 1] ?? '') === '{') { $depth++; $j += 2; continue; }
                        if ($n === '{') { $depth++; $j++; continue; }
                        if ($n === '}') { $depth--; $j++; continue; }
                        if ($n === '`' && $depth === 0) { $j++; break; }
                        $j++;
                    }
                    $emit(substr($js, $i, $j - $i));
                    $i = $j;
                    continue;
                }
                $emit($t); $i++;
            }
            continue;
        }
        // 正则字面量（整段原样，内部 \\ 与 [...] 中的 / 不终止）
        if ($c === '/' && $regexAllowed()) {
            $flushSpace($c);
            $j = $i + 1;
            $inClass = false;
            while ($j < $len) {
                $r = $js[$j];
                if ($r === '\\') { $j += 2; continue; }
                if ($r === '[') $inClass = true;
                elseif ($r === ']') $inClass = false;
                elseif ($r === '/' && !$inClass) { $j++; break; }
                elseif ($r === "\n") break; // 非法正则跨行，交给语法检查兜底
                $j++;
            }
            while ($j < $len && (($js[$j] >= 'a' && $js[$j] <= 'z') || $js[$j] === '_')) $j++; // flags
            $emit(substr($js, $i, $j - $i));
            $i = $j;
            continue;
        }
        // 普通字符
        $flushSpace($c);
        $emit($c);
        $i++;
    }
    return trim($out);
}

/** HTML 压缩：去注释（保留 [if 条件注释）、文本与标签内空白折叠；pre/textarea/非 JS script 原样 */
function minify_html(string $html): string {
    $out = '';
    $len = strlen($html);
    $i = 0;
    while ($i < $len) {
        $c = $html[$i];
        if ($c === '<' && substr($html, $i, 4) === '<!--') {
            $end = strpos($html, '-->', $i + 4);
            if ($end === false) { $out .= substr($html, $i); break; }
            $body = substr($html, $i + 4, $end - $i - 4);
            if (preg_match('/^\[if/i', ltrim($body))) $out .= substr($html, $i, $end + 3 - $i); // 条件注释保留
            // 普通注释删除（HTML 注释不产生空白，直接置空）
            $i = $end + 3;
            continue;
        }
        // 原样区块：pre / textarea / 非 JS 的 script（如 ld+json）
        if ($c === '<' && preg_match('#^<(pre|textarea|script|style)([\s>/])#i', substr($html, $i, 12), $m)) {
            $tag = strtolower($m[1]);
            $openEnd = strpos($html, '>', $i);
            $openTag = substr($html, $i, $openEnd - $i + 1);
            $closeAt = stripos($html, "</{$tag}", $openEnd);
            $closeTagAt = $closeAt === false ? $len : $closeAt;
            $inner = substr($html, $openEnd + 1, $closeTagAt - $openEnd - 1);
            $closeEnd = $closeAt === false ? $len : strpos($html, '>', $closeTagAt);
            $closeTag = $closeAt === false ? '' : substr($html, $closeTagAt, $closeEnd - $closeTagAt + 1);
            if ($tag === 'pre' || $tag === 'textarea') {
                $out .= $openTag . $inner . $closeTag;
            } elseif ($tag === 'style') {
                $out .= $openTag . minify_css($inner) . $closeTag;
            } else { // script：仅压缩无 type / JS 类型的，其余（ld+json、importmap 等）原样
                $isJs = !preg_match('/\btype\s*=\s*[\'"]?([^\'"\s>]+)/i', $openTag, $tm)
                    || preg_match('#^(module|text/javascript|application/javascript)$#i', $tm[1]);
                $out .= $openTag . ($isJs ? minify_js($inner) : $inner) . $closeTag;
            }
            $i = $closeEnd === false ? $len : $closeEnd + 1;
            continue;
        }
        if ($c === '<') { // 标签内部：属性间空白折叠，引号内原样
            $end = strpos($html, '>', $i);
            if ($end === false) { $out .= substr($html, $i); break; }
            $tag = substr($html, $i, $end - $i + 1);
            $squashed = '';
            $tl = strlen($tag);
            for ($k = 0; $k < $tl; $k++) {
                $tc = $tag[$k];
                if ($tc === '"' || $tc === "'") { // 属性值原样
                    $q = $tc;
                    $squashed .= $tc;
                    $k++;
                    while ($k < $tl) { $squashed .= $tag[$k]; if ($tag[$k] === $q) break; $k++; }
                    continue;
                }
                if ($tc === ' ' || $tc === "\t" || $tc === "\n" || $tc === "\r") {
                    while ($k + 1 < $tl && ($tag[$k + 1] === ' ' || $tag[$k + 1] === "\t" || $tag[$k + 1] === "\n" || $tag[$k + 1] === "\r")) $k++;
                    $next = $tag[$k + 1] ?? '';
                    if ($next !== '' && $next !== '>' && $squashed !== '' && substr($squashed, -1) !== '<') $squashed .= ' ';
                    continue;
                }
                $squashed .= $tc;
            }
            $out .= $squashed;
            $i = $end + 1;
            continue;
        }
        if ($c === ' ' || $c === "\t" || $c === "\n" || $c === "\r") { // 文本空白折叠为单空格（渲染语义等价）
            while ($i + 1 < $len && strpos(" \t\n\r", $html[$i + 1]) !== false) $i++;
            if ($i + 1 < $len) $out .= ' ';
            $i++;
            continue;
        }
        $out .= $c;
        $i++;
    }
    return trim($out);
}

/** 导出资产复制：.css/.js 压缩后写入，其余原样 */
function export_copy_asset(string $source, string $target): bool {
    $ext = strtolower(pathinfo($target, PATHINFO_EXTENSION));
    if ($ext === 'css') return file_put_contents($target, minify_css((string)file_get_contents($source))) !== false;
    if ($ext === 'js') return file_put_contents($target, minify_js((string)file_get_contents($source))) !== false;
    return copy($source, $target);
}
