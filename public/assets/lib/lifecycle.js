const cleanup = new Set();
let version = 0;

export function beginView() {
 for (const fn of cleanup) fn();
 cleanup.clear();
 return ++version;
}

export function onViewLeave(fn) {
 cleanup.add(fn);
 return () => { cleanup.delete(fn); fn(); };
}

export function currentView(token) { return token === version; }
