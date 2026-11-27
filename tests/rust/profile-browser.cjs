const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async () => {
 const [base,password,file]=process.argv.slice(2);
 const page=await openPage(base);
 const until=async expression=>{
  for(let i=0;i<200;i++) {
   if(await page.evalJs(expression))return;
   await new Promise(resolve=>setTimeout(resolve,50));
  }
  throw new Error('页面等待超时: '+expression);
 };
 try {
  await page.goto(base+'/admin?tab=settings');
  await until('!!document.querySelector("#auth-form")');
  await page.evalJs(`document.querySelector('[name=username]').value='tester';document.querySelector('[name=password]').value=${JSON.stringify(password)};document.querySelector('#auth-form').requestSubmit()`);
  for(const [kind,message] of [['avatar','头像已更新'],['cover','封面已更新']]) {
   await until(`!!document.querySelector('#${kind}-input')`);
   const {root}=await page.send('DOM.getDocument');
   const {nodeId}=await page.send('DOM.querySelector',{nodeId:root.nodeId,selector:'#'+kind+'-input'});
   await page.send('DOM.setFileInputFiles',{nodeId,files:[file]});
   await until(`document.querySelector('#toast').textContent.includes('${message}')`);
   assert.ok(await page.evalJs(`fetch('/api?action=state').then(response=>response.json()).then(state=>state.profile.${kind})`));
  }
  for(const [kind,message] of [['avatar','已恢复默认头像'],['cover','已移除封面']]) {
   await page.evalJs(`document.querySelector('#${kind}-remove').click()`);
   await until(`document.querySelector('#toast').textContent.includes('${message}')`);
   assert.equal(await page.evalJs(`fetch('/api?action=state').then(response=>response.json()).then(state=>state.profile.${kind})`),null);
  }
  assert.deepEqual(page.errors,[]);
  console.log('PASS: 浏览器头像/封面上传、资料状态更新与移除');
 } finally {page.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
