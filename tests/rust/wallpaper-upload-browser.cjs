const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async () => {
 const [base,file]=process.argv.slice(2);
 const page=await openPage(base);
 const until=async expression=>{
  for(let i=0;i<200;i++) {
   if(await page.evalJs(expression))return;
   await new Promise(resolve=>setTimeout(resolve,50));
  }
  throw new Error('页面等待超时: '+expression);
 };
 try {
  await page.goto(base+'/admin');
  await until('!!document.querySelector("#auth-form")');
  await page.evalJs('document.querySelector("[name=username]").value="tester";document.querySelector("[name=password]").value="Password12345";document.querySelector("#auth-form").requestSubmit()');
  await until('!!document.querySelector("[data-admin-tab=plugins]")');
  await page.goto(base+'/wallpapers');
  await until('document.querySelectorAll(".media-tile").length===3 && !!document.querySelector("#wallpaper-file")');
  const {root}=await page.send('DOM.getDocument');
  const {nodeId}=await page.send('DOM.querySelector',{nodeId:root.nodeId,selector:'#wallpaper-file'});
  await page.send('DOM.setFileInputFiles',{nodeId,files:[file]});
  await until('document.querySelectorAll(".media-tile").length===4');
  assert.ok(await page.evalJs('document.querySelector("#toast").textContent.includes("已上传 1 张壁纸")'));
  await page.evalJs(`document.querySelector('[data-wall-name="browser.png"] [data-wall-delete]').click()`);
  await until('document.querySelector("#wall-delete-dialog")?.open');
  await page.evalJs('document.querySelector("[data-wall-confirm]").click()');
  await until('document.querySelectorAll(".media-tile").length===3 && !document.querySelector("#wall-delete-dialog").open');
  assert.ok(await page.evalJs('document.querySelector("#toast").textContent.includes("壁纸已删除")'));
  assert.deepEqual(page.errors,[]);
  console.log('PASS: 浏览器壁纸文件选择上传、图库更新与管理员删除');
 } finally {page.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
