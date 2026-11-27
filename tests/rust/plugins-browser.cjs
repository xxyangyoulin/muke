const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async () => {
 const base=process.argv[2];
 const page=await openPage(base);
 const until=async expression=>{
  for(let i=0;i<150;i++){
   if(await page.evalJs(expression))return;
   await new Promise(resolve=>setTimeout(resolve,50));
  }
  throw new Error('页面等待超时: '+expression+' '+await page.evalJs('document.querySelector("#main").textContent.slice(0,500)'));
 };
 try {
  await page.goto(base+'/rss');
  await until('document.querySelectorAll("#rss-list .post").length===4');
  assert.ok(await page.evalJs(`!!document.querySelector('link[href="/plugins/rss/plugin.css"]')`));
  assert.ok(await page.evalJs('document.querySelector("#rss-list").textContent.includes("Summary & text")'));
  await page.goto(base+'/admin');
  await until('!!document.querySelector("#auth-form")');
  await page.evalJs('document.querySelector("[name=username]").value="tester";document.querySelector("[name=password]").value="Password12345";document.querySelector("#auth-form").requestSubmit()');
  await until('!!document.querySelector("[data-admin-tab=plugins]")');
  await page.evalJs('document.querySelector("[data-admin-tab=plugins]").click()');
  await until('!!document.querySelector("[data-plugin=rss] .plugin-sync")');
  await page.evalJs('document.querySelector("[data-plugin=rss] .plugin-sync").click()');
  await until('document.querySelector("#toast").textContent.includes("后台任务 #")');
  await page.evalJs('document.querySelector("[data-admin-tab=tasks]").click()');
  await until('document.querySelector("#tasks-connection")?.textContent.includes("已连接") && document.querySelectorAll("#task-status tbody tr").length>0');
  await until('document.querySelector("#task-status tbody tr")?.textContent.includes("已完成")');
  await page.goto(base+'/rss');
  await until('document.querySelectorAll("#rss-list .post").length===4 && !!document.querySelector("#rss-add-form")');
  assert.deepEqual(page.errors,[]);
  console.log('PASS: 插件清单加载资源、RSS 卡片/管理面板、异步同步和 SSE 任务页');
 } finally {page.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
