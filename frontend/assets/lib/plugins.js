import { store } from './store.js';

export async function loadPluginAssets() {
 for (const [id, plugin] of Object.entries(store.state.plugins || {})) {
  if (plugin.style && !document.querySelector(`link[href="${CSS.escape(plugin.style)}"]`)) {
   const link = document.createElement('link');
   link.rel = 'stylesheet';
   link.href = plugin.style;
   document.head.appendChild(link);
  }
  if (plugin.module && !window.MKBLOG._bundled?.[id]) await import(plugin.module);
 }
}
