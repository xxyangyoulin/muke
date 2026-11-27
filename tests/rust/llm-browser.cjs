const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async()=>{
 const base=process.argv[2], page=await openPage(base);
 const until=async(expression,attempts=200)=>{for(let i=0;i<attempts;i++){if(await page.evalJs(expression))return;await new Promise(r=>setTimeout(r,50));}throw new Error(expression+'\n'+await page.evalJs('document.body.innerText'));};
 try {
  await page.goto(base+'/admin?tab=models');
  await until('!!document.querySelector("#auth-form")');
  await page.evalJs('document.querySelector("[name=username]").value="tester";document.querySelector("[name=password]").value="Password12345";document.querySelector("#auth-form").requestSubmit()');
  await until('!!document.querySelector("[data-model-edit]")');
  await page.evalJs('document.querySelector("[data-model-edit]").click()');
  assert.equal(await page.evalJs('document.querySelector("#model-form [name=api_key]").value'),'');
  await page.evalJs('document.querySelector("#model-form [name=name]").value="Shared model";document.querySelector("#model-form").requestSubmit()');
  await until('document.querySelector("#model-list").textContent.includes("Shared model")');
  await page.evalJs('document.querySelector("[data-admin-tab=plugins]").click()');
  await until('!!document.querySelector("#plugin-install-toggle")');
  await page.evalJs('document.querySelector("#plugin-install-toggle").click()');
  await until('document.querySelector("#plugin-build-environment").textContent.includes("已就绪")');
  await page.evalJs(`const f=document.querySelector('#plugin-install-form');f.elements.source_kind.value='local';f.elements.source_kind.dispatchEvent(new Event('change'));f.elements.source.value=${JSON.stringify(process.argv[3])};document.querySelector('#plugin-scan').click()`);
  await until('document.querySelector("#plugin-scan-status").textContent.includes("找到 2 个插件")');
  await page.evalJs('document.querySelector("#plugin-install-form [name=path]").value="plugins/hello";document.querySelector("#plugin-install-form").requestSubmit()');
  await until('document.querySelector("#plugin-install-refresh")?.hidden===false',1800);
  assert.deepEqual(page.errors,[]);
  console.log('PASS: model settings UI keeps keys hidden; multi-plugin scan, selection and install');
 }finally{page.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
