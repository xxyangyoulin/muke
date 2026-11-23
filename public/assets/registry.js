// 插件注册表：必须以经典脚本先于所有模块加载（CSP 禁内联）
// 卡片（registerCard）/ 页面（registerPage）/ 首页侧栏小组件（registerWidget）
// 经典阻塞脚本在首帧绘制前执行，顺带打上 JS 可用标记：静态页据此隐藏烘焙兜底、改由 SPA 渲染
document.documentElement.classList.add('has-js');
window.MKBLOG = (function () {
  var cards = {}, pages = {}, widgets = {};
  function esc(v) { return String(v).replace(/[&<>"']/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]; }); }
  return {
    esc: esc, _cards: cards, _pages: pages, _widgets: widgets,
    registerCard: function (name, fn) { if (cards[name]) throw new Error('卡片重复注册：' + name); cards[name] = fn; },
    registerPage: function (route, page) { if (pages[route]) throw new Error('页面重复注册：' + route); pages[route] = page; },
    registerWidget: function (id, fn) { if (widgets[id]) throw new Error('组件重复注册：' + id); widgets[id] = fn; }
  };
})();
