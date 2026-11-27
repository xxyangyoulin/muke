import { apiGet } from '/assets/lib/api.js';
import { store } from '/assets/lib/store.js';

MKBLOG.registerPage('/hello', {
 async render() {
  const data = store.isStatic
   ? store.pluginsData.hello
   : (await apiGet('plugin-data', { id: 'hello' })).data;
  document.querySelector('#main').innerHTML = `<section class="hello-plugin panel"><h1>独立插件</h1><p>${MKBLOG.esc(data.message)}</p></section>`;
 }
});
