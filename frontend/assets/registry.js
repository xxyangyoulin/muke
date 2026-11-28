// 插件注册表：必须以经典脚本先于所有模块加载（CSP 禁内联）
// 卡片（registerCard）/ 页面（registerPage）/ 首页侧栏小组件（registerWidget）
// 经典阻塞脚本在首帧绘制前执行，顺带打上 JS 可用标记：静态页据此隐藏烘焙兜底、改由 SPA 渲染
document.documentElement.classList.add('has-js');
window.MKBLOG = (function () {
  var cards = {}, pages = {}, widgets = {};
  function esc(v) { return String(v).replace(/[&<>"']/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]; }); }
  // 行内元素：先转义，代码片段用占位符保护后再处理链接与强调
  function inline(text) {
    var codes = [];
    var value = esc(text);
    value = value.replace(/`([^`]+)`/g, function (_, code) { codes.push('<code>' + code + '</code>'); return '\u0000' + (codes.length - 1) + '\u0000'; });
    value = value.replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, function (_, label, url) { return '<a href="' + url + '" target="_blank" rel="noopener">' + label + '</a>'; });
    value = value.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    value = value.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
    return value.replace(/\u0000(\d+)\u0000/g, function (_, index) { return codes[index]; });
  }
  // 极简 Markdown 渲染：标题（统一映射为 h4）、无序/有序列表、段落、行内代码、加粗、斜体与 http(s) 链接
  function md(source) {
    var lines = String(source).replace(/\r\n?/g, '\n').split('\n');
    var out = [], paragraph = [], list = null;
    function flushParagraph() { if (paragraph.length) { out.push('<p>' + paragraph.map(inline).join('<br>') + '</p>'); paragraph = []; } }
    function flushList() { if (list) { out.push('<' + list.tag + '>' + list.items.map(function (item) { return '<li>' + inline(item) + '</li>'; }).join('') + '</' + list.tag + '>'); list = null; } }
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];
      var heading = /^(#{1,6})\s+(.*)$/.exec(line);
      if (heading) { flushParagraph(); flushList(); out.push('<h4>' + inline(heading[2]) + '</h4>'); continue; }
      if (/^\s*$/.test(line)) { flushParagraph(); flushList(); continue; }
      var bullet = /^\s*[-*+]\s+(.*)$/.exec(line);
      var ordered = /^\s*\d+[.)]\s+(.*)$/.exec(line);
      if (bullet || ordered) {
        flushParagraph();
        var tag = bullet ? 'ul' : 'ol';
        if (!list || list.tag !== tag) { flushList(); list = { tag: tag, items: [] }; }
        list.items.push((bullet || ordered)[1]);
        continue;
      }
      flushList();
      paragraph.push(line);
    }
    flushParagraph(); flushList();
    return out.join('');
  }
  return {
    esc: esc, md: md, _cards: cards, _pages: pages, _widgets: widgets,
    registerCard: function (name, fn) { if (cards[name]) throw new Error('卡片重复注册：' + name); cards[name] = fn; },
    registerPage: function (route, page) { if (pages[route]) throw new Error('页面重复注册：' + route); pages[route] = page; },
    registerWidget: function (id, fn) { if (widgets[id]) throw new Error('组件重复注册：' + id); widgets[id] = fn; }
  };
})();
