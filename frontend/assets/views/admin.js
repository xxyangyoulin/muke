import { store } from '../lib/store.js';
import { api, apiGet, morePosts } from '../lib/api.js';
import { beginView, onViewLeave } from '../lib/lifecycle.js';
import { pushHistory } from '../lib/navigation.js';
import { icon, esc, toast, dateLabel, empty, profileName, avatar, postsByDate, updateProfile, postHTML } from '../lib/ui.js';
import { composer, bindComposer } from '../components/composer.js';
import { feed } from './feed.js';
import { loadArticles, moreArticles, adminArticleRows, articleText } from './articles.js';
import { renderArticleEditor } from './editor.js';
import { tasksHTML, bindTasks } from './tasks.js';
import { modelsHTML, bindModels } from './models.js';
import { pluginInstallHTML, bindPluginInstall } from './plugin-install.js';

export function optionsFormHTML() {
 return `<form id="options-form" class="panel options-panel"><span class="eyebrow">SITE OPTIONS</span><h2>站点参数</h2><p class="small-label">保存即生效；未改动的项沿用配置文件默认值。</p>${
 [
  ['站点文案', [
   ['site_tagline','站点标语','text','浏览器标题后缀'],
   ['site_description','站点简介','text','搜索引擎摘要'],
   ['site_subtitle','侧栏副标题','text',''],
   ['locale','界面语言','text','如 zh-CN / en-US'],
  ]],
  ['日期与阅读', [
   ['date_format','阅读页日期格式','text','格式示例：Y-m-d H:i'],
   ['date_format_short','列表日期格式','text',''],
   ['feed_time_style','动态时间显示','select','', [['','中文时间（默认）'],['compact','紧凑数字 11-22 13:19'],['ago','相对时间 x 分钟前'],['date','只显示日期'],['none','不显示']]],
   ['reading_chars_per_minute','阅读速度（字/分钟）','number','估算阅读时长用'],
  ]],
  ['内容限额', [
   ['post_max_chars','动态字数上限','number',''],
   ['post_max_images','动态图片上限','number',''],
   ['upload_max_mb','单图体积上限（MB）','number',''],
   ['article_max_bytes','文章正文上限（字节）','number',''],
  ]],
  ['图片处理', [
   ['thumb_size','缩略图尺寸','number','长边像素'],
   ['thumb_quality','缩略图质量','number','1–100'],
   ['avatar_size','头像尺寸','number','正方形像素'],
  ]],
 ].map(([group, fields]) => `<div class="option-group"><h4>${group}</h4><div class="options-grid">${fields.map(([k, label, type, hint, options]) => options ? `<label>${label}<select name="${k}">${options.map(([v, l]) => `<option value="${v}" ${(store.state.options?.[k] ?? '') === v ? 'selected' : ''}>${l}</option>`).join('')}</select></label>` : `<label>${label}<input name="${k}" type="${type}" value="${esc(String(store.state.options?.[k] ?? ''))}" placeholder="${hint}"></label>`).join('')}</div></div>`).join('')
}<button class="secondary" type="submit">保存站点参数</button></form>`;
}

export function pluginFieldHTML(f) {
  const val = esc(String(f.value ?? ''));
  if (f.type === 'select') return `<label>${esc(f.label)}<select name="${esc(f.key)}">${Object.entries(f.options || {}).map(([v, l]) => `<option value="${esc(v)}" ${String(f.value) === v ? 'selected' : ''}>${esc(l)}</option>`).join('')}</select></label>`;
  return `<label>${esc(f.label)}<input name="${esc(f.key)}" type="${f.type === 'number' ? 'number' : 'text'}" value="${val}" ${f.min !== undefined ? `min="${f.min}"` : ''} ${f.max !== undefined ? `max="${f.max}"` : ''} placeholder="${esc(f.hint || '')}"></label>`;
}
export function pluginCardHTML(pl) {
 const lastSync = pl.last_sync ? new Date(pl.last_sync * 1000).toLocaleString('zh-CN', {year:'numeric', month:'2-digit', day:'2-digit', hour:'2-digit', minute:'2-digit', second:'2-digit', hourCycle:'h23'}) : '';
 return `<div class="panel plugin-card" data-plugin="${esc(pl.id)}"><div class="plugin-head"><div class="plugin-title"><h2>${esc(pl.name)}</h2><span class="small-label">已同步 ${pl.post_count} 条${lastSync ? ' · 上次 ' + esc(lastSync) : ''}</span></div><div class="plugin-controls"><button class="secondary plugin-settings" type="button">设置</button><label class="switch-line"><input type="checkbox" class="plugin-enabled" ${pl.enabled ? 'checked' : ''}> 启用</label></div></div><p class="small-label plugin-desc">${esc(pl.description)}</p>${pl.installation ? `<p class="small-label">版本 ${esc(pl.version)} · ${esc(pl.installation.source)}</p>` : ''}${pl.last_error ? `<p class="plugin-error small-label">上次失败：${esc(pl.last_error)}</p>` : ''}<div class="plugin-actions"><button class="text-link plugin-sync" type="button" ${pl.methods && !pl.methods.includes('sync') ? 'hidden' : ''}>立即同步</button>${pl.installation ? '<button class="text-link plugin-rebuild" type="button">更新并重新编译</button>' : ''}${pl.supports_history ? `<button class="text-link plugin-sync-all" type="button">同步历史</button>` : ''}<span class="plugin-danger"><button class="text-link plugin-purge" type="button">清空动态</button><button class="text-link plugin-remove" type="button">移除插件</button></span></div></div>`;
}

export function adminSectionHTML() {
 if (store.adminTab === 'models') return modelsHTML();
 if (store.adminTab === 'tasks') return tasksHTML();
 if (store.adminTab === 'posts') {
  const withImages = store.posts.filter(p => (p.images || []).length).length;
  const weekAgo = Date.now() - 7 * 86400000;
  const recentWeek = store.posts.filter(p => new Date(p.created_at).getTime() >= weekAgo).length;
  return `<div class="admin-cols"><div class="panel posts-panel"><div class="section-heading"><h2>已加载动态</h2><span>${store.posts.length} 条</span></div>${store.posts.length ? postsByDate().map(postHTML).join('') : empty('还没有动态', '在概览页的发布框写下第一条。')}</div><aside class="admin-side"><section class="side-card admin-side-card"><div class="section-heading"><h3>动态数据</h3></div><div class="admin-side-rows"><span>共 <strong>${store.posts.length}</strong> 条动态</span><span>含图片 <strong>${withImages}</strong> 条</span><span>近 7 天新增 <strong>${recentWeek}</strong> 条</span></div></section><section class="side-card admin-side-card"><div class="section-heading"><h3>快捷操作</h3></div><div class="admin-side-actions"><button class="secondary" type="button" data-admin-tab-jump="overview">去发布动态</button><button class="secondary" type="button" data-admin-tab-jump="publish">静态发布</button></div></section></aside></div>`;
 }
 if (store.adminTab === 'articles') {
  const drafts = store.articlesDrafts;
  const words = store.articlesLoaded ? store.articles.reduce((n, a) => n + (a.text_length ?? articleText(a).length), 0) : 0;
  const head = `<div class="section-heading"><h2>全部文章</h2><span>${store.articlesLoaded ? store.articlesTotal + ' 篇' + (drafts.length ? ' · ' + store.draftsTotal + ' 草稿' : '') : '…'}</span><button class="text-link" id="new-article" type="button">写新文章 ${icon('plus')}</button></div>`;
  const side = `<aside class="admin-side"><section class="side-card admin-side-card"><div class="section-heading"><h3>写作数据</h3></div><div class="admin-side-rows"><span>已发布 <strong>${store.articlesLoaded ? store.articlesTotal : '—'}</strong> 篇</span>${drafts.length ? `<span>草稿 <strong>${store.draftsTotal}</strong> 篇</span>` : ''}<span>已加载文章约 <strong>${store.articlesLoaded ? (words / 1000).toFixed(1) : '—'}</strong> 千字</span></div></section>${drafts.length ? `<section class="side-card admin-side-card"><div class="section-heading"><h3>草稿箱</h3><span>${store.draftsTotal}</span></div><div class="admin-draft-list">${drafts.map(a => `<button type="button" class="admin-draft-item" data-edit-article="${a.id}"><strong>${esc(a.title)}</strong><span>${esc(dateLabel(a.created_at))}</span></button>`).join('')}</div></section>` : ''}<section class="side-card admin-side-card"><div class="section-heading"><h3>快捷操作</h3></div><div class="admin-side-actions"><button class="primary" type="button" data-new-article>写新文章</button><button class="secondary" type="button" data-admin-tab-jump="publish">静态发布</button></div></section></aside>`;
  return `<div class="admin-cols"><div class="panel posts-panel">${head}${store.articlesLoaded ? adminArticleRows() : empty('正在载入文章…', '', 'edit')}</div>${side}</div>`;
 }
 if (store.adminTab === 'comments') {
  const d = store.commentsData;
  const opts = store.state.options || {};
  const provider = opts.comment_provider || '';
  const thirdParty = ['waline', 'twikoo', 'custom'].includes(provider);
  const embedHints = { waline: 'serverURL，如 https://waline.example.com', twikoo: 'envId 或部署地址', custom: 'iframe 页面地址' };
  const field = (name, label, input) => `<label>${label}${input}</label>`;
  const sel = (name, options, cur) => `<select name="${name}">${options.map(([v, l]) => `<option value="${v}" ${cur === v ? 'selected' : ''}>${l}</option>`).join('')}</select>`;
  const inp = (name, type, val, extra = '') => `<input name="${name}" type="${type}" value="${esc(String(val ?? ''))}" ${extra}>`;
  const settingsPanel = `<div class="panel comment-settings">
   <div class="section-heading"><h2>评论设置</h2></div>
   <form id="comment-provider-form">
    <div class="option-group"><h4>基础</h4><div class="options-grid">
     ${field('comment_provider', '评论系统', sel('comment_provider', [['','自带（原生）'],['off','关闭评论'],['giscus','giscus'],['waline','Waline'],['twikoo','Twikoo'],['custom','自定义 iframe']], provider))}
     ${field('comment_moderation', '审核策略', sel('comment_moderation', [['','先发后审（默认）'],['pre','先审后发']], opts.comment_moderation || ''))}
     ${field('comment_scope', '评论范围', sel('comment_scope', [['','文章 + 动态'],['article','仅文章']], opts.comment_scope || ''))}
     ${field('comment_rate_seconds', '频控窗口（秒）', inp('comment_rate_seconds', 'number', opts.comment_rate_seconds ?? 60, 'min="0" max="3600"'))}
    </div></div>
    <div class="option-group giscus-group${provider === 'giscus' ? '' : ' hidden'}"><h4>giscus</h4><p class="small-label">在 <a href="https://giscus.app/zh-CN" target="_blank" rel="noopener">giscus 配置页</a>选择公开仓库和 Announcements 类型分类，填写生成代码中的对应值。访客通过 GitHub 登录，评论在仓库 Discussions 管理。</p><div class="options-grid">
     ${field('giscus_repo', 'GitHub 仓库（data-repo）', inp('giscus_repo', 'text', opts.giscus_repo, 'placeholder="所有者/仓库名"'))}
     ${field('giscus_repo_id', '仓库 ID（data-repo-id）', inp('giscus_repo_id', 'text', opts.giscus_repo_id, 'placeholder="R_…"'))}
     ${field('giscus_category_id', '分类 ID（data-category-id）', inp('giscus_category_id', 'text', opts.giscus_category_id, 'placeholder="DIC_…"'))}
     ${field('giscus_refresh_hours', '计数刷新间隔（小时，0 = 每次导出都查询）', inp('giscus_refresh_hours', 'number', opts.giscus_refresh_hours ?? 24, 'min="0" max="720"'))}
    </div><p class="small-label">文章和动态分别关联独立讨论；原生评论的审核、频控、邮件及反垃圾设置不作用于 giscus。</p></div>
    <div class="option-group embed-group${thirdParty ? '' : ' hidden'}"><h4>第三方嵌入</h4><div class="options-grid">
     ${field('comment_embed_url', '嵌入地址', inp('comment_embed_url', 'text', opts.comment_embed_url, `placeholder="${embedHints[provider] || '见各系统文档'}"`))}
    </div></div>
    <div class="option-group"><h4>邮件通知 <span class="small-label">（可选，留空不启用）</span></h4><div class="options-grid">
     ${field('smtp_host', 'SMTP 服务器', inp('smtp_host', 'text', store.state.comment_settings?.smtp_host, 'placeholder="如 smtp.qq.com"'))}
     ${field('smtp_port', 'SMTP 端口', inp('smtp_port', 'number', store.state.comment_settings?.smtp_port ?? 465, 'min="1" max="65535"'))}
     ${field('smtp_user', 'SMTP 账号', inp('smtp_user', 'text', store.state.comment_settings?.smtp_user))}
     ${field('smtp_pass', 'SMTP 授权码', inp('smtp_pass', 'password', '', 'placeholder="留空则保持不变"'))}
     ${field('notify_email', '通知邮箱', inp('notify_email', 'text', store.state.comment_settings?.notify_email, 'placeholder="新评论通知到"'))}
    </div></div>
    <div class="option-group"><h4>反垃圾</h4><div class="options-grid">
     ${field('akismet_key', 'Akismet Key', inp('akismet_key', 'text', store.state.comment_settings?.akismet_key, 'placeholder="留空不启用"'))}
    </div></div>
    <div class="dialog-actions left"><button class="secondary" type="submit">保存评论设置</button></div>
   </form></div>`;
  const listPanel = !d ? '<div class="panel"><p class="small-label">正在载入评论…</p></div>' : (() => {
   const counts = d.counts || {all: 0, approved: 0, pending: 0, spam: 0};
   const filters = [['all', `全部 ${counts.all}`], ['approved', `已发 ${counts.approved}`], ['pending', `待审 ${counts.pending}`], ['spam', `垃圾 ${counts.spam}`]];
   const row = c => `<div class="comment-admin-row" data-cid="${c.id}"><div class="comment-admin-main"><strong>${esc(c.author)}</strong> <span class="small-label">评论 ${c.kind === 'article' ? '文章' : '动态'}「${esc(c.target_label)}」· ${esc(dateLabel(c.created_at))}</span>${c.status !== 'approved' ? `<em class="draft-badge">${c.status === 'pending' ? '待审' : '垃圾'}</em>` : ''}<p>${esc(c.content)}</p></div><div class="post-admin"><button class="icon-button" data-comment-spam="${c.id}" aria-label="标垃圾" title="标垃圾">${icon('trash')}</button>${c.status !== 'approved' ? `<button class="icon-button" data-comment-approve="${c.id}" aria-label="恢复显示" title="恢复显示">${icon('check')}</button>` : ''}<button class="icon-button" data-comment-delete="${c.id}" aria-label="彻底删除" title="彻底删除">${icon('close')}</button></div></div>`;
   return `<div class="panel comment-admin-panel"><div class="section-heading"><h2>评论管理</h2><span class="small-label">先发后审：标过垃圾的 IP 后续评论自动进待审</span></div><div class="comment-filters">${filters.map(([k, l]) => `<button data-comment-filter="${k}" class="${(store.commentsFilter || 'all') === k ? 'active' : ''}">${l}</button>`).join('')}</div><div class="comment-admin-list">${d.comments.length ? d.comments.map(row).join('') : empty('还没有评论', '访客评论会显示在这里。', 'chat')}</div></div>`;
  })();
  return `<div class="top-tabs" role="group" aria-label="评论管理与设置">${[['list', '评论管理'], ['settings', '评论设置']].map(([key, label]) => `<button type="button" class="tab${store.commentsTab === key ? ' selected' : ''}" data-comments-tab="${key}" aria-pressed="${store.commentsTab === key}">${label}</button>`).join('')}</div><div data-comments-panel="list" ${store.commentsTab === 'list' ? '' : 'hidden'}>${listPanel}</div><div data-comments-panel="settings" ${store.commentsTab === 'settings' ? '' : 'hidden'}>${settingsPanel}</div>`;
 }
 if (store.adminTab === 'publish') return `<div class="settings-cols"><div class="settings-left"><div class="panel export-panel"><span class="eyebrow">STATIC EXPORT</span><h2>生成静态站点</h2><p>导出纯静态 HTML，包含已加载动态、文章及插件内容；已审评论烘焙为只读。</p><div class="dialog-actions left"><button id="export-site" class="primary">${icon('download')}生成静态站点</button><button id="download-static" class="secondary" type="button">下载站点包 (zip)</button></div><p id="export-status" class="export-status" role="status">导出目录：项目内的 dist/；下载打包最近一次生成结果</p></div></div><div><div class="panel netlify-panel"><span class="eyebrow">ONE CLICK TO NETLIFY</span><h2>一键部署</h2>${store.state.netlify?.has_token ? `<p class="small-label">已绑定站点 ${esc(store.state.netlify?.site || '')}</p>` : '<p class="small-label">先保存访问令牌和站点 ID，之后点一下即可自动导出并上传。</p>'}<button id="deploy-site" class="primary">部署到 Netlify ${icon('arrow')}</button><button id="cancel-deploy" class="secondary" type="button" hidden>取消部署</button><button id="netlify-toggle" class="text-link" type="button" aria-expanded="false">${store.state.netlify?.has_token ? '修改配置' : '配置凭据'}</button><form id="auto-deploy-form"><label class="auto-deploy-check"><input type="checkbox" name="auto_deploy" ${store.state.netlify?.auto_deploy ? 'checked' : ''}>发布新动态后自动部署（默认关闭）</label><label>防抖等待（分钟）<input name="auto_deploy_debounce" type="number" min="1" max="1440" value="${store.state.netlify?.auto_deploy_debounce ?? 10}"></label><p class="small-label">静默指定分钟无新动态才执行，期间再发动态会顺延计时；仅手动发布的动态触发。</p><button class="secondary" type="submit">保存自动部署设置</button></form><div id="netlify-form-wrap" ${store.state.netlify?.has_token ? 'hidden' : ''}><form id="netlify-form"><label>访问令牌<input name="token" type="password" autocomplete="off" placeholder="${store.state.netlify?.has_token ? '已保存，留空则保持不变' : 'Netlify Access Token'}"></label><label>站点 ID<input name="site" value="${esc(store.state.netlify?.site || '')}" placeholder="例如 my-site-1234 或站点 UUID"></label><button class="secondary" type="submit">保存设置</button></form></div><p id="netlify-status" class="export-status" role="status">${store.state.netlify?.has_token ? '点击上方按钮：后台任务自动导出并上传，可离开页面。' : ''}</p><div id="deploy-progress" class="deploy-bar" hidden><i></i></div></div></div></div>`;
 if (store.adminTab === 'options') return optionsFormHTML();
 if (store.adminTab === 'plugins') {
  if (store.pluginsAdmin === null) return '<p class="panel">正在读取插件…</p>';
  const cards = (store.pluginsAdmin || []).filter(pl => !pl.removed);
  return pluginInstallHTML() + (cards.length ? `<div class="plugins-grid">${cards.map(pluginCardHTML).join('')}</div>` : empty('没有可用插件', '安装插件后在这里启用和配置。', 'grid'));
 }
 if (store.adminTab === 'settings') return `<div class="settings-cols"><div class="settings-left"><form id="profile-form" class="panel"><span class="eyebrow">PROFILE</span><h2>空间资料</h2><div class="avatar-edit"><span class="avatar-edit-preview" id="avatar-preview">${store.state.profile.avatar ? `<img src="${esc(store.state.profile.avatar)}" alt="">` : esc(store.state.profile.name.slice(0,1))}</span><div class="avatar-edit-actions"><button class="secondary" type="button" id="avatar-upload">更换头像</button><button class="text-link" type="button" id="avatar-remove">移除头像</button></div><input type="file" id="avatar-input" accept="image/jpeg,image/png,image/webp,image/gif,image/avif" hidden></div><label>昵称<input name="name" maxlength="40" value="${profileName()}" required></label><label>用户名<input name="handle" maxlength="30" pattern="[a-zA-Z0-9_]+" value="${esc(store.state.profile.handle)}" required></label><div class="cover-edit"><div class="cover-edit-preview" id="cover-preview">${store.state.profile.cover ? `<img src="${esc(store.state.profile.cover)}" alt="">` : '<span class="small-label">未设置封面</span>'}</div><div class="cover-edit-actions"><label class="secondary" style="cursor:pointer">上传封面<input type="file" id="cover-input" accept="image/jpeg,image/png,image/webp,image/gif,image/avif" hidden></label><button class="text-link" type="button" id="cover-remove">移除封面</button></div></div><label>封面角标文字（选填）<input name="cover_badge" maxlength="30" value="${esc(store.state.options.cover_badge || '')}" placeholder="如：日常 / 灵感 / 风景，留空不显示"></label><label>个人介绍<textarea name="bio" maxlength="160" required>${esc(store.state.profile.bio)}</textarea></label><button class="secondary" type="submit">保存资料</button></form><form id="password-form" class="panel"><span class="eyebrow">SECURITY</span><h2>修改密码</h2><label>当前密码<input name="current" type="password" required autocomplete="current-password"></label><label>新密码<input name="next" type="password" required minlength="10" maxlength="200" autocomplete="new-password" placeholder="至少 10 位"></label><label>确认新密码<input name="confirm" type="password" required minlength="10" maxlength="200" autocomplete="new-password"></label><button class="secondary" type="submit">更新密码</button></form><div class="panel"><span class="eyebrow">BACKUP</span><h2>备份与恢复</h2><p class="small-label">导出全部动态 / 文章 / 设置 / 上传图片为 JSON；导入将整体替换内容（登录账号保留）。不含壁纸原图及插件业务表，完整备份请使用 CLI。</p><div class="dialog-actions left"><button class="secondary" id="backup-export" type="button">导出备份</button><button class="secondary" id="backup-import-btn" type="button">导入备份</button><input type="file" id="backup-file" accept="application/json" hidden></div></div></div></div>`;
 const recentPosts = postsByDate().slice(0, 5);
 const recentArticles = [...store.articlesDrafts, ...store.articles].slice(0, 5);
 return `<div class="admin-hero"><span class="eyebrow">ADMIN CONSOLE · v${esc(store.state.version)}</span><h1>你好，${profileName()}<span>。</span></h1><p>动态、文章、评论与站点发布，都在这里管理。</p><div class="stat-grid"><div><span>已发布动态</span><strong>${store.postsTotal}<small>条记录</small></strong>${icon('edit')}</div><div><span>文章随笔</span><strong>${store.articlesLoaded ? store.articlesTotal : '—'}<small>篇${store.articlesDrafts.length ? ' · ' + store.draftsTotal + ' 草稿' : ''}</small></strong>${icon('edit')}</div><div><span>站点状态</span><strong class="status-text"><span class="online-dot"></span>运行中</strong><small>数据保存在本地</small></div></div></div>
 <div class="panel create-panel"><div class="section-heading"><h2>记录此刻</h2><button class="text-link" id="go-new-article" type="button">写文章更尽兴？去编辑器 ${icon('arrow')}</button></div>${composer()}</div>
 <div class="overview-grid"><div class="panel posts-panel"><div class="section-heading"><h2>最近动态</h2><button class="text-link" data-admin-tab-jump="posts" type="button">全部 ${store.posts.length} 条 ${icon('arrow')}</button></div>${recentPosts.length ? recentPosts.map(postHTML).join('') : empty('还没有动态', '在上方发布框写下第一条。')}</div>
 <div class="panel posts-panel"><div class="section-heading"><h2>最近文章</h2><button class="text-link" data-admin-tab-jump="articles" type="button">全部管理 ${icon('arrow')}</button></div>${recentArticles.length ? adminArticleRows() : empty('还没有文章', '支持 Markdown 和富文本。', 'edit')}</div></div>`;
}

export function admin() {
 beginView();
 onViewLeave(stopDeployPoll);
 if(!store.state.admin) {
  store.main.innerHTML = `<div class="auth-wrap"><div class="auth-card"><span class="auth-symbol">m·</span><span class="eyebrow">${store.state.setup ? 'A NEW BEGINNING' : 'WELCOME BACK'}</span><h1>${store.state.setup ? '开始使用' : '欢迎回来'}</h1><p>${store.state.setup ? '创建管理员账号，开始记录与分享。' : '输入账号与密码，进入管理后台。'}</p><form id="auth-form"><label>用户名<input name="username" required minlength="2" maxlength="40" autocomplete="username" placeholder="你的管理员用户名"></label><label>密码<input name="password" type="password" required minlength="${store.state.setup ? 10 : 1}" maxlength="200" autocomplete="${store.state.setup ? 'new-password' : 'current-password'}" placeholder="${store.state.setup ? '至少 10 位密码' : '输入密码'}"></label><p id="auth-error" class="form-error" role="alert"></p><button class="primary" type="submit">${store.state.setup ? '创建并进入后台' : '登录管理后台'} ${icon('arrow')}</button></form><small>${store.state.setup ? '首次设置仅限服务器本机访问。' : '仅管理员可访问本页。'}</small></div></div>`;
  document.querySelector('#auth-form').onsubmit = async e => { e.preventDefault(); const button=e.target.querySelector('button');button.disabled=true;try { await api(store.state.setup ? 'setup' : 'login',Object.fromEntries(new FormData(e.target))); store.state=await api('state'); await loadArticles(true); updateProfile(); admin(); } catch(error) { document.querySelector('#auth-error').textContent=error.message; } finally {button.disabled=false;} }; return;
 }
 if (store.editingArticle !== false) { renderArticleEditor(); return; }
 if (!store.articlesLoaded) loadArticles().then(() => { if (store.page === 'admin' && store.editingArticle === false) admin(); }).catch(() => {});
 const tabs = [['overview', '概览'], ['posts', '动态'], ['articles', '文章'], ['plugins', '插件'], ['comments', '评论'], ['tasks', '任务'], ['models', '模型'], ['options', '参数'], ['settings', '设置'], ['publish', '发布']];
 const urlTab = new URLSearchParams(location.search).get('tab');
 if (urlTab && tabs.some(([key]) => key === urlTab)) store.adminTab = urlTab; // 刷新/直达时按 URL 恢复页签
 store.main.innerHTML = `<div class="admin-page">
 <div class="top-tabs admin-tabs">${tabs.map(([key, label]) => `<button class="tab${store.adminTab === key ? ' selected' : ''}" data-admin-tab="${key}" type="button">${label}</button>`).join('')}<span class="admin-exit"><button id="logout" class="text-link" type="button">退出登录</button></span></div>
 <div id="admin-body">${adminSectionHTML()}</div></div>`;
 document.querySelectorAll('[data-admin-tab]').forEach(button => button.onclick = () => { store.adminTab = button.dataset.adminTab; pushHistory('/admin?tab=' + store.adminTab); admin(); });
 bindAdminSection();
}

// 一键部署轮询：后台任务独立于页面运行，进入发布页签或点部署后开始，切走切回自动恢复显示
let deployTimer = null, deployToast = false, deployStarting = false;
export function stopDeployPoll() { if (deployTimer) { clearInterval(deployTimer); deployTimer = null; } deployToast = false; deployStarting = false; }
function deployPhaseText(st) {
 if (st.state === 'exporting') return '正在导出静态站点…（可离开本页）';
 if (st.state === 'uploading') return `正在上传文件 ${st.files_done}/${st.files_total}${st.current ? '（' + esc(st.current.split('/').pop()) + '）' : ''}，可离开本页`;
 if (st.state === 'waiting') return st.files_total > 0 ? `已上传 ${st.files_total} 个文件，等待 Netlify 构建生效…` : '内容无变化，等待 Netlify 构建生效…';
 return '';
}
export function pollDeploy(started) {
 if (started) { deployToast = true; deployStarting = true; }
 if (deployTimer) return;
 const tick = async () => {
  let d = null;
  try { d = await apiGet('deploy-status'); } catch { return; }
  const s2 = document.querySelector('#netlify-status');
  if (!s2) return; // 不在发布页签：轮询保持，切回来自动恢复渲染
  const st = d?.status, pd = d?.pending;
  const cancel = document.querySelector('#cancel-deploy');
  if (cancel) { cancel.hidden = !['exporting', 'uploading', 'waiting'].includes(st?.state); cancel.disabled = !!d?.cancelling; cancel.textContent = d?.cancelling ? '正在取消…' : '取消部署'; }
  const bar = document.querySelector('#deploy-progress'), btn = document.querySelector('#deploy-site');
  if (!st || !['exporting','uploading','waiting'].includes(st.state)) {
   if (deployStarting) return; // 启动请求在途：旧终态（上次 done/error）不渲染、不停轮询，等新占位落盘再显示
   if (bar) bar.hidden = true;
   if (btn) btn.disabled = false;
   let html;
   if (st?.state === 'done') { html = `部署完成：<a href="${esc(st.url)}" target="_blank" rel="noopener">${esc(st.url)}</a>`; if (deployToast) toast('已部署到 Netlify'); }
   else if (st?.state === 'cancelled') html = '部署已取消' + (st.error ? '：' + esc(st.error) : '');
   else if (st?.state === 'error') html = '部署失败：' + esc(st.error || '未知错误');
   else html = esc(store.state.netlify?.has_token ? '点击上方按钮：后台任务自动导出并上传，可离开页面。' : '先保存访问令牌和站点 ID，之后点一下即可自动导出并上传。');
   if (pd) html += ` · 自动部署约 ${Math.max(1, Math.ceil(pd.in / 60))} 分钟后执行（新动态顺延）`;
   s2.innerHTML = html;
   deployToast = false; deployStarting = false;
   if (!pd) { clearInterval(deployTimer); deployTimer = null; } // 无排队任务才停轮询；有则继续刷倒计时
   return;
  }
  if (btn) btn.disabled = true;
  if (bar) {
   const pct = st.files_total > 0 ? Math.round(100 * st.files_done / st.files_total) : 0;
   bar.hidden = st.state !== 'uploading' || st.files_total === 0;
   bar.firstElementChild.style.width = pct + '%';
  }
  s2.textContent = deployPhaseText(st);
 };
 tick();
 deployTimer = setInterval(tick, 2000);
}

export function bindAdminSection() {
 if (store.adminTab === 'models') bindModels();
 if (store.adminTab === 'tasks') bindTasks();
 if (store.adminTab === 'articles') {
  const panel = document.querySelector('.posts-panel');
  for (const [draft, next, label] of [[false, store.articlesNext, '文章'], [true, store.draftsNext, '草稿']]) {
   if (!next) continue;
   const button = document.createElement('button');
   button.type = 'button'; button.className = 'secondary'; button.textContent = '加载更多' + label;
   button.dataset.moreArticles = draft ? 'draft' : 'published';
   panel.append(button);
   button.onclick = async () => {
    button.disabled = true;
    try { await moreArticles(draft); if (panel.isConnected) admin(); }
    catch (e) { toast(e.message); button.disabled = false; }
   };
  }
 }
 if (store.adminTab === 'posts' && store.postsNext) {
  const panel = document.querySelector('.posts-panel');
  panel.insertAdjacentHTML('beforeend', '<button type="button" class="secondary" id="admin-more-posts">加载更多动态</button>');
  document.querySelector('#admin-more-posts').onclick = async e => {
   e.target.disabled = true;
   try { await morePosts(); if (panel.isConnected) admin(); } catch (error) { toast(error.message); e.target.disabled = false; }
  };
 }
 bindComposer();
 document.querySelector('#logout').onclick = async () => {try{await api('logout',{});store.state=await api('state'); await loadArticles(true);admin();}catch(e){toast(e.message);}};
 document.querySelectorAll('[data-admin-tab-jump]').forEach(button => button.onclick = () => { store.adminTab = button.dataset.adminTabJump; pushHistory('/admin?tab=' + store.adminTab); admin(); });
 const goNew = document.querySelector('#go-new-article');
 if (goNew) goNew.onclick = () => { store.editingArticle = 'new'; store.editorFormat = 'md'; admin(); };
 const newBtns = document.querySelectorAll('[data-new-article],#new-article');
 newBtns.forEach(b => b.onclick = () => { store.editingArticle = 'new'; store.editorFormat = 'md'; admin(); });
 const profileForm = document.querySelector('#profile-form');
 if (profileForm) profileForm.onsubmit = async e => {e.preventDefault();try{await api('profile',Object.fromEntries(new FormData(e.target)));store.state=await api('state');updateProfile();toast('空间资料已保存');}catch(error){toast(error.message);}};
 const optionsForm = document.querySelector('#options-form');
 const syncFeedTimeUI = () => {
  if (!optionsForm) return;
  const sel = optionsForm.querySelector('select[name=feed_time_style]');
  if (!sel) return;
  const show = sel.value === 'custom';
  const custom = optionsForm.querySelector('input[name=feed_time_custom]')?.closest('label');
  if (custom) custom.style.display = show ? '' : 'none';
  const legend = optionsForm.querySelector('[data-feed-legend]');
  if (legend) legend.style.display = show ? '' : 'none';
 };
 if (optionsForm) syncFeedTimeUI();
 if (optionsForm) optionsForm.addEventListener('change', syncFeedTimeUI);
 if (optionsForm) optionsForm.onsubmit = async e => {
  e.preventDefault();
  try {
   const fd = new FormData(e.target);
   const hk = e.target.querySelector('input[name=feed_hide_time]');
   if (hk) fd.set('feed_hide_time', hk.checked ? '1' : '0');
   await api('save-settings', Object.fromEntries(fd));
   store.state = await api('state');
   toast('站点参数已保存');
   admin();
  } catch (error) { toast(error.message); }
 };
 if (store.adminTab === 'plugins' && store.pluginsAdmin === null) {
  let active = true;
  onViewLeave(() => { active = false; });
  loadPlugins().then(() => { if (active) admin(); });
  return;
 }
 if (store.adminTab === 'comments' && !store.commentsData) { loadCommentsAdmin().then(() => admin()).catch(() => admin()); return; }
 document.querySelectorAll('[data-comments-tab]').forEach(button => button.onclick = () => {
  store.commentsTab = button.dataset.commentsTab;
  document.querySelectorAll('[data-comments-tab]').forEach(tab => {
   const selected = tab.dataset.commentsTab === store.commentsTab;
   tab.classList.toggle('selected', selected);
   tab.setAttribute('aria-pressed', String(selected));
  });
  document.querySelectorAll('[data-comments-panel]').forEach(panel => { panel.hidden = panel.dataset.commentsPanel !== store.commentsTab; });
 });
 document.querySelectorAll('[data-comment-filter]').forEach(b => b.onclick = () => { store.commentsFilter = b.dataset.commentFilter; loadCommentsAdmin(store.commentsFilter).then(() => admin()); });
 document.querySelectorAll('[data-comment-spam],[data-comment-approve],[data-comment-delete]').forEach(b => b.onclick = async () => {
  const op = b.dataset.commentSpam ? 'spam' : b.dataset.commentApprove ? 'approve' : 'delete';
  const id = b.dataset.commentSpam || b.dataset.commentApprove || b.dataset.commentDelete;
  try { await api('moderate-comment', {id: Number(id), op}); await loadCommentsAdmin(store.commentsFilter || 'all'); admin(); toast(op === 'delete' ? '已删除' : op === 'spam' ? '已标垃圾' : '已恢复'); } catch (e) { toast(e.message); }
 });
 bindPluginInstall(() => { store.adminTab = 'tasks'; pushHistory('/admin?tab=tasks'); admin(); });
 document.querySelectorAll('.plugin-card').forEach(card => {
  const id = card.dataset.plugin;
  const pl = (store.pluginsAdmin || []).find(x => x.id === id);
  const rebuild = card.querySelector('.plugin-rebuild');
  if (rebuild) rebuild.onclick = async () => {
   rebuild.disabled = true;
   try {
    const result = await api('install-plugin', {source_kind:pl.installation.kind, source:pl.installation.source, path:pl.installation.path || ''});
    toast(`已加入编译任务 #${result.task_id}`);
    store.adminTab = 'tasks'; pushHistory('/admin?tab=tasks'); admin();
   } catch (error) { toast(error.message); rebuild.disabled = false; }
  };
  card.querySelector('.plugin-enabled').onchange = async e => {
   try {
    await api('save-plugin', {id, enabled: e.target.checked ? 1 : 0, values: {}});
    await loadPlugins();
    toast(e.target.checked ? '插件已启用' : '插件已停用');
   } catch (error) { toast(error.message); e.target.checked = !e.target.checked; }
  };
  card.querySelector('.plugin-purge').onclick = () => {
   store.pendingPluginPurge = id;
   const d = document.querySelector('#delete-dialog');
   d.querySelector('h2').textContent = '清空该插件的已加载动态？';
   d.querySelector('p').textContent = '将删除已同步的动态与对应图片并清除墓碑（之后可重新同步）；插件配置保留。';
   d.showModal();
  };
  card.querySelector('.plugin-settings').onclick = () => {
   const form = document.querySelector('#plugin-settings-form');
   form.dataset.plugin = id;
   document.querySelector('#plugin-settings-title').textContent = `「${pl.name}」设置`;
   document.querySelector('#plugin-settings-desc').textContent = pl.description || '';
   document.querySelector('#plugin-settings-fields').innerHTML = pl.fields.map(pluginFieldHTML).join('');
   document.querySelector('#plugin-settings-dialog').showModal();
  };
  card.querySelector('.plugin-sync').onclick = async e => {
   const b = e.currentTarget;
   b.disabled = true;
   try { const r = await api('sync-plugin', {id}); toast(`已加入后台任务 #${r.task_id}`); await loadPlugins(); admin(); }
   catch (error) { toast(error.message); } finally { b.disabled = false; }
  };
  const syncAllBtn = card.querySelector('.plugin-sync-all');
  if (syncAllBtn) syncAllBtn.onclick = async e => {
   const b = e.currentTarget;
   b.disabled = true;
   b.textContent = '正在拉取历史…';
   try { const r = await api('sync-plugin', {id, all: 1}); toast(`历史同步已加入后台任务 #${r.task_id}`); await loadPlugins(); admin(); }
   catch (error) { toast(error.message); } finally { b.disabled = false; }
  };
  card.querySelector('.plugin-remove').onclick = () => {
   store.pendingPluginRemove = id;
   document.querySelector('#plugin-remove-title').textContent = `移除「${pl ? pl.name : id}」？`;
   document.querySelector('#plugin-remove-purge').checked = pl ? pl.post_count > 0 : false;
   document.querySelector('#plugin-remove-dialog').showModal();
  };
 });
 const backupBtn = document.querySelector('#backup-export');
 if (backupBtn) backupBtn.onclick = async () => {
  try {
   const data = await api('backup-export');
   const blob = new Blob([JSON.stringify(data)], {type: 'application/json'});
   const a = document.createElement('a');
   a.href = URL.createObjectURL(blob);
   a.download = `mkblog-backup-${new Date().toISOString().slice(0, 10)}.json`;
   a.click();
   URL.revokeObjectURL(a.href);
   toast('备份已下载');
  } catch (e) { toast(e.message); }
 };
 const backupFile = document.querySelector('#backup-file');
 if (backupFile) {
  document.querySelector('#backup-import-btn').onclick = () => backupFile.click();
  backupFile.onchange = async () => {
   const file = backupFile.files[0];
   backupFile.value = '';
   if (!file) return;
   if (!confirm('导入将整体替换现有动态 / 文章 / 设置（登录账号保留），确定继续？')) return;
   try {
    const response = await fetch('/api?action=backup-import', {method: 'POST', headers: {'Content-Type': 'application/json', 'X-CSRF-Token': store.state.csrf}, body: await file.text()});
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || '导入失败');
    toast(`导入完成：${result.posts} 条动态、${result.articles} 篇文章`);
    store.state = await api('state');
    store.posts = (await api('posts')).posts;
    await loadArticles(true);
    admin();
   } catch (e) { toast(e.message); }
  };
 }
 const settingsForm = document.querySelector('#plugin-settings-form');
 if (settingsForm) settingsForm.onsubmit = async e => {
  e.preventDefault();
  const values = {};
  settingsForm.querySelectorAll('input[name],select[name]').forEach(el => { values[el.name] = el.value; });
  try {
   await api('save-plugin', {id: settingsForm.dataset.plugin, values});
   document.querySelector('#plugin-settings-dialog').close();
   await loadPlugins();
   admin();
   toast('插件设置已保存');
  } catch (error) { toast(error.message); }
 };
 const passwordForm = document.querySelector('#password-form');
 if (passwordForm) passwordForm.onsubmit = async e => {
  e.preventDefault();
  const data = Object.fromEntries(new FormData(e.target));
  if (data.next !== data.confirm) { toast('两次输入的新密码不一致'); return; }
  try { await api('change-password', data); e.target.reset(); toast('密码已更新，下次登录使用新密码'); } catch (error) { toast(error.message); }
 };
 const avatarInput = document.querySelector('#avatar-input');
 if (avatarInput) {
  document.querySelector('#avatar-upload').onclick = () => avatarInput.click();
  avatarInput.onchange = async () => {
   const file = avatarInput.files[0];
   avatarInput.value = '';
   if (!file) return;
   if (file.size > 15 * 1024 * 1024) { toast('图片不能超过 15MB'); return; }
   toast('正在处理头像…');
   try {
    const data = new FormData();
    data.append('avatar', file);
    const response = await fetch('/api?action=save-avatar', {method: 'POST', headers: {'X-CSRF-Token': store.state.csrf}, body: data});
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || '上传失败');
    store.state = await api('state');
    updateProfile();
    admin();
    toast('头像已更新');
   } catch (error) { toast(error.message); }
  };
  document.querySelector('#avatar-remove').onclick = async () => {
   try { await api('remove-avatar', {}); store.state = await api('state'); updateProfile(); admin(); toast('已恢复默认头像'); } catch (e) { toast(e.message); }
  };
 }
 const coverInput = document.querySelector('#cover-input');
 if (coverInput) {
  coverInput.onchange = async () => {
   const file = coverInput.files[0];
   coverInput.value = '';
   if (!file) return;
   if (file.size > 15 * 1024 * 1024) { toast('图片不能超过 15MB'); return; }
   toast('正在处理封面…（自动居中裁剪为 3:1）');
   try {
    const data = new FormData();
    data.append('cover', file);
    const response = await fetch('/api?action=save-cover', {method: 'POST', headers: {'X-CSRF-Token': store.state.csrf}, body: data});
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || '上传失败');
    store.state = await api('state');
    admin();
    toast('封面已更新');
   } catch (error) { toast(error.message); }
  };
  document.querySelector('#cover-remove').onclick = async () => {
   try { await api('remove-cover', {}); store.state = await api('state'); admin(); toast('已移除封面'); } catch (e) { toast(e.message); }
  };
 }
 const exportBtn = document.querySelector('#export-site');
 if (exportBtn) exportBtn.onclick = async e => {const b=e.currentTarget;b.disabled=true;const s=document.querySelector('#export-status');s.textContent='已加入后台任务，正在生成静态站点…（可离开本页，进度见任务页）';try{const {task_id}=await api('export',{});for(;;){await new Promise(r=>setTimeout(r,1000));const jobs=await apiGet('tasks');const job=(jobs.jobs||[]).find(j=>j.id===task_id);if(!job)throw new Error('任务记录丢失');if(job.state==='running'&&job.phase)s.textContent=`${job.phase}…（可离开本页）`;if(!['queued','running'].includes(job.state)){if(job.state==='succeeded'){const extra=Object.entries(job.result?.plugins||{}).map(([id,n])=>`${n} 个 ${id}`).join('、');s.textContent=`已导出 ${job.result.posts} 条动态${extra?'、'+extra:''}。`;toast('静态站点已生成');}else s.textContent=job.error||'导出未完成';break;}}}catch(error){s.textContent=error.message;}finally{b.disabled=false;}};
 const netlifyForm = document.querySelector('#netlify-form');
 if (netlifyForm) netlifyForm.onsubmit = async e => {e.preventDefault();try{await api('netlify-save',Object.fromEntries(new FormData(e.target)));store.state=await api('state');toast('Netlify 设置已保存');admin();}catch(error){toast(error.message);}};
 const autoDeployForm = document.querySelector('#auto-deploy-form');
 if (autoDeployForm) autoDeployForm.onsubmit = async e => {e.preventDefault();const d=Object.fromEntries(new FormData(e.target));d.auto_deploy=d.auto_deploy?'1':'0';try{await api('save-settings',d);store.state=await api('state');toast('自动部署设置已保存');admin();}catch(error){toast(error.message);}};
 const commentForm = document.querySelector('#comment-provider-form');
 if (commentForm) commentForm.onsubmit = async e => {e.preventDefault();try{await api('save-settings',Object.fromEntries(new FormData(e.target)));store.state=await api('state');toast('评论设置已保存');admin();}catch(error){toast(error.message);}};
 const providerSelect = document.querySelector('#comment-provider-form select[name=comment_provider]');
 if (providerSelect) providerSelect.onchange = () => {
  const v = providerSelect.value;
  const hints = { waline: 'serverURL，如 https://waline.example.com', twikoo: 'envId 或部署地址', custom: 'iframe 页面地址' };
  const group = document.querySelector('.embed-group');
  const embedInput = document.querySelector('#comment-provider-form input[name=comment_embed_url]');
  if (group) group.classList.toggle('hidden', !['waline', 'twikoo', 'custom'].includes(v));
  document.querySelector('.giscus-group')?.classList.toggle('hidden', v !== 'giscus');
  if (embedInput) embedInput.placeholder = hints[v] || '';
 };
 const deployBtn = document.querySelector('#deploy-site');
 const dlBtn = document.querySelector('#download-static');
 const netlifyToggle = document.querySelector('#netlify-toggle');
 if (netlifyToggle) netlifyToggle.onclick = () => {
  const wrap = document.querySelector('#netlify-form-wrap');
  wrap.hidden = !wrap.hidden;
  netlifyToggle.setAttribute('aria-expanded', String(!wrap.hidden));
 };
 if (dlBtn) dlBtn.onclick = () => { window.location = '/api?action=download-static'; };
 const cancelDeploy = document.querySelector('#cancel-deploy');
 if (cancelDeploy) cancelDeploy.onclick = async () => {
  cancelDeploy.disabled = true;
  try { await api('cancel-deploy', {}); cancelDeploy.textContent = '正在取消…'; pollDeploy(false); }
  catch (error) { toast(error.message); cancelDeploy.disabled = false; }
 };
 if (deployBtn) deployBtn.onclick = async e => {const b=e.currentTarget;b.disabled=true;const s2=document.querySelector('#netlify-status');s2.textContent='正在启动后台部署任务…';pollDeploy(true);try{await api('netlify-deploy',{});deployStarting=false;}catch(error){stopDeployPoll();s2.textContent=error.message;b.disabled=false;}};
 if (deployBtn) pollDeploy();
}

export async function loadPlugins(){try{store.pluginsAdmin=(await api('plugins')).plugins;}catch{store.pluginsAdmin=[];}}
export async function loadCommentsAdmin(status = 'all'){try{const d=await apiGet('comments-admin',{status});store.commentsData=d;}catch{store.commentsData={comments:[],counts:{all:0,approved:0,pending:0,spam:0}};}}
