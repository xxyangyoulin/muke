const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async () => {
 const base=process.argv[2];
 const page=await openPage(base);
 try {
  await page.goto(base+'/');
  let ready=false;
  for(let i=0;i<150;i++) {
   if(await page.evalJs('document.querySelectorAll(".post-weibo-card").length===2')) {ready=true;break;}
   await new Promise(resolve=>setTimeout(resolve,50));
  }
  assert.ok(ready,'微博卡片应由插件模块渲染');
  assert.ok(await page.evalJs(`!!document.querySelector('link[href="/plugins/weibo/plugin.css"]')`));
  assert.ok(await page.evalJs('document.querySelector("#main").textContent.includes("Weibo & fixture")'));
  assert.equal(await page.evalJs('document.querySelectorAll(".weibo-grid .post-image").length'),2);
  assert.equal(await page.evalJs('document.querySelectorAll("a[href*=\\"weibo.com/status\\"]").length'),0);
  assert.deepEqual(page.errors,[]);
  console.log('PASS: 微博插件卡片、图片与隐藏原文开关');
 } finally {page.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
