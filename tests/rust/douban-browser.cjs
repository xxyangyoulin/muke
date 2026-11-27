const assert = require('node:assert/strict');
const {openPage} = require('../cdp.cjs');
(async () => {
 const base=process.argv[2];
 const page=await openPage(base);
 try {
  await page.goto(base+'/');
  let ready=false;
  for(let i=0;i<150;i++) {
   if(await page.evalJs('document.querySelectorAll(".post-movie-card").length===1')) {ready=true;break;}
   await new Promise(resolve=>setTimeout(resolve,50));
  }
  assert.ok(ready,'电影卡片应由豆瓣插件渲染');
  assert.ok(await page.evalJs(`!!document.querySelector('link[href="/plugins/douban/plugin.css"]')`));
  assert.equal(await page.evalJs('document.querySelector(".post-movie-card").href'),'https://movie.douban.com/subject/123/');
  assert.equal(await page.evalJs('document.querySelector(".post-movie-info strong").textContent'),'电影 <script>fixture</script>');
  assert.equal(await page.evalJs('document.querySelectorAll(".post-movie-card script").length'),0);
  assert.equal(await page.evalJs('document.querySelector(".movie-stars").textContent'),'★★★★☆');
  assert.equal(await page.evalJs('document.querySelector(".movie-date").textContent'),'2026-11-26 想看');
  assert.ok(await page.evalJs('document.querySelector(".movie-comment").textContent.includes("很好 & 好看")'));
  assert.ok(await page.evalJs('document.querySelector(".post-movie-card img").complete && document.querySelector(".post-movie-card img").naturalWidth>0'));
  assert.deepEqual(page.errors,[]);
  console.log('PASS: 豆瓣电影卡片、封面、评分、分类与文本转义');
 } finally {page.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
