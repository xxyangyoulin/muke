const assert = require('node:assert/strict');
const {execFileSync} = require('node:child_process');
const {openPage} = require('./cdp.cjs');

(async () => {
 const [base,root] = process.argv.slice(2);
 const sql = query => execFileSync('sqlite3',[root+'/var/blog.sqlite',query],{encoding:'utf8'}).trim();
 sql(`WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100)
 INSERT INTO articles(title,content,tags,draft,created_at,updated_at)
 SELECT 'article-page-'||x,'search-body-'||x,CASE WHEN x=1 THEN 'rare' ELSE 'common' END,CASE WHEN x>65 THEN 1 ELSE 0 END,'2019-01-01','2019-01-01' FROM n;`);
 const get = async query => (await fetch(base+'/api?action=articles&'+query)).json();
 const p = await openPage(base);
 try {
  let cursor='', ids=[];
  do {
   const page = await get('q=article-page-&limit=17&cursor='+encodeURIComponent(cursor));
   assert.equal(page.total,65); assert.ok(page.articles.length<=17);
   ids.push(...page.articles.map(a=>a.id)); cursor=page.next_cursor;
  } while(cursor);
  assert.equal(new Set(ids).size,65);
  assert.equal((await get('tag=rare')).articles[0].title,'article-page-1');
  assert.equal((await get('tag=rar')).articles.length,0);
  assert.equal((await get('q=search-body-1')).total,11);
  assert.equal((await fetch(base+'/api?action=articles&draft=1')).status,401);
  assert.equal((await fetch(base+'/api?action=articles&id='+sql("SELECT id FROM articles WHERE title='article-page-100'"))).status,404);
  assert.equal((await fetch(base+'/api?action=articles&cursor=invalid')).status,422);
  const plan = sql("EXPLAIN QUERY PLAN SELECT * FROM articles WHERE draft=0 AND (created_at,id)<('2020',999) ORDER BY created_at DESC,id DESC LIMIT 31");
  assert.ok(plan.includes('idx_articles_timeline') && !plan.includes('TEMP B-TREE'));
  const postPlan = sql("EXPLAIN QUERY PLAN SELECT * FROM posts WHERE (created_at,id)<('2020',999) ORDER BY created_at DESC,id DESC LIMIT 31");
  assert.ok(postPlan.includes('idx_posts_timeline') && !postPlan.includes('TEMP B-TREE'));
  await p.goto(base+'/articles');
  assert.equal(await p.evalJs(`document.querySelectorAll('.article-teaser').length`),30);
  for(let i=0;i<4 && await p.evalJs(`!!document.querySelector('#more-articles')`);i++) {
   await p.evalJs(`document.querySelector('#more-articles').onclick()`);
  }
  assert.equal(await p.evalJs(`[...document.querySelectorAll('.article-teaser h3')].filter(e=>e.textContent.startsWith('article-page-')).length`),65);
  await p.goto(base+'/articles/?tag=rare');
  assert.equal(await p.evalJs(`document.querySelector('.article-teaser h3').textContent`),'article-page-1');
  await p.goto(base+'/');
  for(let i=0;i<18;i++) { await p.evalJs(`window.scrollTo(0,document.body.scrollHeight)`); await new Promise(r=>setTimeout(r,200)); }
  assert.equal(await p.evalJs(`[...document.querySelectorAll('.article-teaser h3')].filter(e=>e.textContent.startsWith('article-page-')).length`),65);
  await p.goto(base+'/admin');
  await p.evalJs(`document.querySelector('#auth-form input[name=username]').value='tester';document.querySelector('#auth-form input[name=password]').value='Password12345';document.querySelector('#auth-form').requestSubmit()`);
  await new Promise(r=>setTimeout(r,800));
  await p.evalJs(`document.querySelector('[data-admin-tab="articles"]').click()`);
  await p.evalJs(`document.querySelector('[data-more-articles="draft"]').onclick()`);
  assert.equal(await p.evalJs(`import('/assets/lib/store.js').then(({store})=>store.articlesDrafts.length)`),35);
  for(let i=0;i<4 && await p.evalJs(`!!document.querySelector('[data-more-articles="published"]')`);i++) await p.evalJs(`document.querySelector('[data-more-articles="published"]').onclick()`);
  assert.equal(await p.evalJs(`import('/assets/lib/store.js').then(({store})=>store.articles.filter(a=>a.title.startsWith('article-page-')).length)`),65);
  assert.deepEqual(p.errors,[]);
  console.log('article-pagination: 文章/草稿分页、标签、全文搜索、权限、首页混排、后台加载及索引通过');
 } finally { p.close(); sql("DELETE FROM articles WHERE title LIKE 'article-page-%'"); }
})().catch(error=>{console.error(error);process.exitCode=1;});
