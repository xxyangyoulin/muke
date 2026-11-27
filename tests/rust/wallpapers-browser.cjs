const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async () => {
 const base=process.argv[2];
 const page=await openPage(base);
 const until=async expression=>{
  for(let i=0;i<150;i++) {
   if(await page.evalJs(expression))return;
   await new Promise(resolve=>setTimeout(resolve,50));
  }
  throw new Error('页面等待超时: '+expression);
 };
 try {
  await page.goto(base+'/wallpapers');
  await until('document.querySelectorAll(".media-tile").length===2');
  assert.ok(await page.evalJs(`!!document.querySelector('link[href="/plugins/wallpapers/plugin.css"]')`));
  await page.evalJs('document.querySelector("[data-filter=portrait]").click()');
  await until('document.querySelectorAll(".media-tile").length===1');
  assert.ok(await page.evalJs('document.querySelector(".wall-caption").textContent.includes("竖屏")'));
  await page.goto(base+'/admin');
  await until('!!document.querySelector("#auth-form")');
  await page.evalJs('document.querySelector("[name=username]").value="tester";document.querySelector("[name=password]").value="Password12345";document.querySelector("#auth-form").requestSubmit()');
  await until('!!document.querySelector("[data-admin-tab=plugins]")');
  await page.goto(base+'/wallpapers');
  await until('!!document.querySelector("#refresh-gallery")');
  await page.evalJs('document.querySelector("#refresh-gallery").click()');
  await until('document.querySelector("#toast").textContent.includes("后台任务 #")');
  assert.deepEqual(page.errors,[]);
  console.log('PASS: 壁纸图库、分类筛选和异步同步按钮');
 } finally {page.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
