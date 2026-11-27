const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async () => {
 const base = process.argv[2];
 const page = await openPage(base);
 const until = async (expression, attempts=150) => {
  for (let i=0;i<attempts;i++) {
   if (await page.evalJs(expression)) return;
   await new Promise(resolve => setTimeout(resolve,50));
  }
  throw new Error('页面等待超时: '+expression+'\n'+await page.evalJs('document.body.innerText')+'\n'+JSON.stringify(page.errors));
 };
 try {
  if (process.argv[3] === '--static') {
   await page.goto(base+'/hello/');
   await until('document.querySelector(".hello-plugin")?.textContent.includes("Hello independent plugin") && !document.querySelector("#baked-body")');
   await page.send('Emulation.setScriptExecutionDisabled', {value:true});
   await page.goto(base+'/hello/');
   assert.ok(await page.evalJs('document.querySelector("#baked-body")?.textContent.includes("Hello independent plugin")'));
   assert.notEqual(await page.evalJs('getComputedStyle(document.querySelector("#baked-body")).display'), 'none');
   assert.deepEqual(page.errors, []);
   console.log('PASS: standalone static plugin page with and without JavaScript');
   return;
  }
  await page.goto(base+'/hello');
  await until('document.querySelector(".hello-plugin")?.textContent.includes("Hello independent plugin")');
  await page.goto(base+'/admin');
  await until('!!document.querySelector("#auth-form")');
  await page.evalJs('document.querySelector("[name=username]").value="tester";document.querySelector("[name=password]").value="Password12345";document.querySelector("#auth-form").requestSubmit()');
  await until('!!document.querySelector("[data-admin-tab=plugins]")');
  await page.evalJs('document.querySelector("[data-admin-tab=plugins]").click()');
  await until('!!document.querySelector("#plugin-install-toggle")');
  await page.evalJs('document.querySelector("#plugin-install-toggle").click()');
  await until('document.querySelector("#plugin-build-environment").textContent.includes("已就绪")');
  await page.evalJs('const select=document.querySelector("#plugin-install-form [name=source_kind]");select.value="local";select.dispatchEvent(new Event("change"));');
  assert.ok(await page.evalJs('document.querySelector("#plugin-source-hint").textContent.includes("绝对路径")'));
  assert.ok(await page.evalJs('!!document.querySelector("[data-plugin=hello] .plugin-rebuild")'));
  await page.evalJs(`document.querySelector('#plugin-install-form [name=source]').value=${JSON.stringify(process.argv[3])};document.querySelector('#plugin-install-form').requestSubmit()`);
  await until('document.querySelector("#task-log-output")?.textContent.includes("Compiling")', 1800);
  await until('document.querySelector("#plugin-install-refresh")?.hidden === false', 1800);
  assert.deepEqual(page.errors, []);
  console.log('PASS: standalone plugin page, installer submission, environment/source controls, rebuild button and live build logs');
 } finally { page.close(); }
})().catch(error => { console.error(error); process.exitCode=1; });
