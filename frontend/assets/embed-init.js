// 评论嵌入页初始化器：配置经自身 <script> 的 data-* 传入（CSP 禁内联脚本）
// Waline 与 Twikoo 的脚本由本文件动态注入（嵌入页 CSP 已放行对应 CDN）
(function () {
  const me = document.currentScript;
  const provider = me?.dataset.provider || '';
  const url = me?.dataset.url || '';
  const root = document.getElementById('embed-root');
  if (!root || !url) return;

  function loadScript(src) {
    return new Promise((resolve, reject) => {
      const s = document.createElement('script');
      s.src = src; s.defer = true;
      s.onload = resolve; s.onerror = () => reject(new Error('脚本加载失败'));
      document.head.appendChild(s);
    });
  }

  if (provider === 'waline') {
    root.innerHTML = '<div id="waline"></div>';
    loadScript('https://unpkg.com/@waline/client@v3/dist/widget.js')
      .then(() => window.Waline?.init({ el: '#waline', serverURL: url, dark: 'html:not([data-theme=light])', lang: 'zh-CN' }))
      .catch(() => { root.innerHTML = '<p style="padding:16px;font-size:13px;color:#939ba9">评论组件加载失败，请稍后再试。</p>'; });
  } else if (provider === 'twikoo') {
    root.innerHTML = '<div id="twikoo"></div>';
    loadScript('https://cdn.jsdelivr.net/npm/twikoo@1.6.44/dist/twikoo.all.min.js')
      .then(() => window.twikoo?.init({ el: '#twikoo', envId: url }))
      .catch(() => { root.innerHTML = '<p style="padding:16px;font-size:13px;color:#939ba9">评论组件加载失败，请稍后再试。</p>'; });
  }
})();
