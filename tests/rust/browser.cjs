const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async () => {
 const [base,password] = process.argv.slice(2);
 const page = await openPage(base);
 async function until(expression) {
  for (let i=0;i<150;i++) {
   if (await page.evalJs(expression)) return;
   await new Promise(resolve=>setTimeout(resolve,50));
  }
  throw new Error('页面等待超时: '+expression+'; '+await page.evalJs(`document.querySelector('#main').textContent.slice(0,400)+' '+document.querySelector('#toast').textContent`));
 }
 try {
  if (password) {
   await page.goto(base+'/admin');
   await until(`!!document.querySelector('#auth-form')`);
   await page.evalJs(`document.querySelector('[name=username]').value='tester';document.querySelector('[name=password]').value=${JSON.stringify(password)};document.querySelector('#auth-form').requestSubmit()`);
   await until(`!!document.querySelector('#go-new-article')`);
   await page.evalJs(`document.querySelector('#go-new-article').click()`);
   await until(`!!document.querySelector('#article-content')`);
   await page.evalJs(`document.querySelector('#article-title').value='浏览器文章';document.querySelector('#article-content').value='正文通过 Rust 保存';document.querySelector('#article-save').click()`);
   await until(`!document.querySelector('#article-content') && document.querySelector('#toast').textContent.includes('已发布')`);
   await page.goto(base+'/articles');
   await until(`!!document.querySelector('.article-teaser')`);
   await page.evalJs(`[...document.querySelectorAll('.article-teaser')].find(a=>a.textContent.includes('浏览器文章')).click()`);
   await until(`!!document.querySelector('.article-content') && document.querySelector('.article-content').textContent.includes('正文通过 Rust 保存')`);
  } else {
   await page.goto(base);
   await until(`document.querySelectorAll('#post-list .post').length>=10`);
   assert.ok(await page.evalJs(`document.querySelector('.profile-name-col').textContent.includes('Rust 契约站')`));
   await page.evalJs(`const input=document.querySelector('#feed-search');input.value='猫';input.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('[data-search-type="post"]').click()`);
   await until(`document.querySelectorAll('.search-highlight').length>0`);
   assert.ok(await page.evalJs(`[...document.querySelectorAll('#post-list .post')].every(n=>n.textContent.includes('猫'))`));
   await page.evalJs(`document.querySelector('nav [data-nav="articles"]').click()`);
   await until(`document.querySelectorAll('#article-rows .post').length===4`);
   assert.ok(await page.evalJs(`!document.querySelector('#article-rows').textContent.includes('私人草稿')`));
   await page.evalJs(`document.querySelector('#article-rows a[href="/article/1/"]').click()`);
   await until(`!!document.querySelector('.article-content')`);
   assert.ok(await page.evalJs(`document.querySelector('.article-content').textContent.includes('第二行')`));
   await page.evalJs(`document.querySelector('.article-footer a').click()`);
   await until(`document.querySelectorAll('#article-rows .post').length===4`);
   await page.evalJs(`document.querySelector('nav [data-nav="articles"]').click();document.querySelector('nav [data-nav="feed"]').click()`);
   await until(`!!document.querySelector('#post-list') && document.querySelectorAll('#post-list .post').length>=10`);
  }
  assert.deepEqual(page.errors,[]);
  console.log('PASS: 原有前端通过 Rust '+(password?'登录、编辑器发布、列表与详情':'加载列表、搜索、草稿隔离、文章详情及快速切换'));
 } finally { page.close(); }
})().catch(error=>{console.error(error);process.exitCode=1;});
