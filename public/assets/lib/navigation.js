let position = history.state?.mukePosition ?? 0;
let currentUrl = location.pathname + location.search;
let reverting = false;
history.replaceState({...history.state, mukePosition:position}, '', location.href);

export function pushHistory(url) {
 history.pushState({mukePosition:++position}, '', url);
 currentUrl = location.pathname + location.search;
}

export function acceptHistoryNavigation(allowLeave) {
 const next = history.state?.mukePosition;
 if (reverting) { reverting = false; return false; }
 const url = location.pathname + location.search;
 if (url === currentUrl) { if (Number.isInteger(next)) position = next; return false; }
 if (!allowLeave()) {
  if (Number.isInteger(next) && next !== position) {
   reverting = true;
   history.go(position - next);
  }
  return false;
 }
 position = next ?? position;
 currentUrl = url;
 return true;
}
