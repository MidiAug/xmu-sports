export type PageScope = { current: () => boolean; onDispose: (cleanup: () => void) => void };
const pages = new WeakMap<Element, () => void>();
export function disposePage(panel: Element) { pages.get(panel)?.(); pages.delete(panel); }
/** Each panel has one owner; late responses cannot repaint a newer page. */
export function mountPage(panel: Element): PageScope {
  disposePage(panel);
  let active = true;
  const cleanups: Array<() => void> = [];
  pages.set(panel, () => { active = false; for (const cleanup of cleanups.splice(0)) cleanup(); });
  return { current: () => active && panel.isConnected,
    onDispose: (cleanup) => { if (active) cleanups.push(cleanup); else cleanup(); } };
}
