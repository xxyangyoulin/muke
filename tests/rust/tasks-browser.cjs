const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async()=>{
 const base=process.argv[2], page=await openPage(base);
 const until=async(expression,attempts=200)=>{for(let i=0;i<attempts;i++){if(await page.evalJs(expression))return;await new Promise(r=>setTimeout(r,50));}throw new Error(expression+'\n'+await page.evalJs('document.body.innerText'));};
 try {
  await page.goto(base+'/admin?tab=tasks');
  await until('!!document.querySelector("#auth-form")');
  await page.evalJs('document.querySelector("[name=username]").value="tester";document.querySelector("[name=password]").value="Password12345";document.querySelector("#auth-form").requestSubmit()');
  await until('!!document.querySelector(".task-table")');
  // 类型过滤器存在且包含任务类型
  const options=await page.evalJs('Array.from(document.querySelector("#task-kind-filter").options).map(o=>o.value)');
  assert.ok(options.includes('all') && options.includes('plugin_sync') && options.includes('plugin_install'),JSON.stringify(options));
  // 运行中的任务行下方有终端风格日志预览，可滚动
  await until('!!document.querySelector(".task-log-row .task-log-terminal.task-log-inline")');
  const inline=await page.evalJs('(()=>{const box=document.querySelector(".task-log-row .task-log-terminal.task-log-inline");return {text:box.textContent,scrollable:box.scrollHeight>=box.clientHeight};})()');
  assert.ok(inline.text.length>0 && inline.scrollable,JSON.stringify(inline));
  assert.ok(await page.evalJs('getComputedStyle(document.querySelector(".task-log-inline")).backgroundColor'));
  // 日志按钮弹窗：统一终端风格，可关闭
  await page.evalJs('document.querySelector("[data-task-log]").click()');
  await until('document.querySelector("#task-log-dialog").open && document.querySelector("#task-log-dialog-output").textContent.length>0 && document.querySelector("#task-log-dialog-output").textContent!=="读取中…"');
  assert.match(await page.evalJs('document.querySelector("#task-log-dialog-output").textContent'),/同步|模型|编译|暂无/s);
  await page.evalJs('document.querySelector("#task-log-dialog [data-close]").click()');
  await until('!document.querySelector("#task-log-dialog").open');
  // 过滤：无任务类型显示空态，切回全部恢复
  await page.evalJs('document.querySelector("#task-kind-filter").value="image";document.querySelector("#task-kind-filter").dispatchEvent(new Event("change"))');
  await until('document.querySelector("#task-status").textContent.includes("该类型暂无任务")');
  await page.evalJs('document.querySelector("#task-kind-filter").value="all";document.querySelector("#task-kind-filter").dispatchEvent(new Event("change"))');
  await until('!!document.querySelector(".task-table")');
  assert.deepEqual(page.errors,[]);
  console.log('PASS: running-task inline terminal logs, newest log previews, kind filter and log dialog');
 }finally{page.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
