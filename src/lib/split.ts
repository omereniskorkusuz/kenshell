/**
 * Split view model (P2-T19), kept pure for tests. A split is the ordered list of tab keys
 * shown side by side (2–4); fewer than 2 means single view (only the active tab).
 *
 * Grid: 2 or 3 panes sit in one row ('cols') or one column ('rows'); 4 panes form 2×2.
 */

export const MAX_PANES = 4;
export type SplitDir = 'cols' | 'rows';
export type DropZone = 'left' | 'right' | 'top' | 'bottom' | 'center';
export type Arrow = 'left' | 'right' | 'up' | 'down';

export interface SplitView {
  keys: string[];
  dir: SplitDir;
}

export function gridOf(n: number, dir: SplitDir): { cols: number; rows: number } {
  if (n <= 1) return { cols: 1, rows: 1 };
  if (n >= 4) return { cols: 2, rows: Math.ceil(n / 2) };
  return dir === 'cols' ? { cols: n, rows: 1 } : { cols: 1, rows: n };
}

export function cellOf(i: number, n: number, dir: SplitDir): { row: number; col: number } {
  const { cols } = gridOf(n, dir);
  return { row: Math.floor(i / cols), col: i % cols };
}

/** Pane box in percent of the workspace. */
export function cellRect(i: number, n: number, dir: SplitDir) {
  const { cols, rows } = gridOf(n, dir);
  const { row, col } = cellOf(i, n, dir);
  return {
    left: (col / cols) * 100,
    top: (row / rows) * 100,
    width: 100 / cols,
    height: 100 / rows,
  };
}

/** Index of the pane next to pane `i` in direction `arrow`, or null at the edge. */
export function neighbor(i: number, n: number, dir: SplitDir, arrow: Arrow): number | null {
  const { cols, rows } = gridOf(n, dir);
  const { row, col } = cellOf(i, n, dir);
  const r = row + (arrow === 'down' ? 1 : arrow === 'up' ? -1 : 0);
  const c = col + (arrow === 'right' ? 1 : arrow === 'left' ? -1 : 0);
  if (r < 0 || c < 0 || r >= rows || c >= cols) return null;
  const j = r * cols + c;
  return j < n ? j : null;
}

/** Drop zone under the pointer: the nearest edge within 25 % of the pane, else center. */
export function zoneAt(
  rect: { left: number; top: number; width: number; height: number },
  x: number,
  y: number,
): DropZone {
  const fx = (x - rect.left) / rect.width;
  const fy = (y - rect.top) / rect.height;
  const edges: [DropZone, number][] = [
    ['left', fx],
    ['right', 1 - fx],
    ['top', fy],
    ['bottom', 1 - fy],
  ];
  const [zone, dist] = edges.reduce((a, b) => (b[1] < a[1] ? b : a));
  return dist < 0.25 ? zone : 'center';
}

/** Moves `arr[from]` to position `to` (index in the result). */
export function moveItem<T>(arr: readonly T[], from: number, to: number): T[] {
  const out = [...arr];
  const [item] = out.splice(from, 1);
  if (item === undefined) return out;
  out.splice(Math.max(0, Math.min(to, out.length)), 0, item);
  return out;
}

/** Activating a tab outside the split puts it into the focused pane's slot. */
export function activateIn(view: SplitView, active: string | null, key: string): SplitView {
  if (view.keys.length < 2 || view.keys.includes(key)) return view;
  const at = active === null ? -1 : view.keys.indexOf(active);
  if (at < 0) return view;
  const keys = [...view.keys];
  keys[at] = key;
  return { ...view, keys };
}

export function removePane(view: SplitView, key: string): SplitView {
  const keys = view.keys.filter((k) => k !== key);
  return { ...view, keys: keys.length < 2 ? [] : keys };
}

/**
 * Adds `key` next to `beside` (single view: the active tab becomes the first pane).
 * Returns null when nothing changes (same tab, or already shown) or the split is full.
 */
export function addPane(
  view: SplitView,
  active: string,
  key: string,
  dir: SplitDir,
  after = true,
  beside: string = active,
): SplitView | null {
  if (key === beside) return null;
  const keys = view.keys.length < 2 ? [active] : [...view.keys];
  if (keys.includes(key) || keys.length >= MAX_PANES) return null;
  const at = keys.indexOf(beside);
  if (at < 0) return null;
  keys.splice(after ? at + 1 : at, 0, key);
  // The direction only matters while 2–3 panes share one row/column.
  return { keys, dir: view.keys.length < 2 ? dir : view.dir };
}

/**
 * Drops tab `drag` on pane `target` (a key shown now) at `zone`:
 * - center → `drag` takes the target's slot (swapping if it was shown elsewhere)
 * - edge   → `drag` is placed beside the target (moved, if already shown); when the split
 *            is full, it replaces the target instead.
 */
export function dropOn(
  view: SplitView,
  active: string,
  drag: string,
  target: string,
  zone: DropZone,
): SplitView {
  const shown = view.keys.length < 2 ? [active] : view.keys;
  if (!shown.includes(target) || drag === target) return view;
  const from = shown.indexOf(drag);
  const to = shown.indexOf(target);
  if (zone === 'center') {
    if (shown.length < 2) return view; // single view: the caller just activates `drag`
    const keys = [...shown];
    keys[to] = drag;
    if (from >= 0) keys[from] = target;
    return { ...view, keys };
  }
  const after = zone === 'right' || zone === 'bottom';
  const dir: SplitDir = zone === 'left' || zone === 'right' ? 'cols' : 'rows';
  if (from >= 0) {
    const rest = shown.filter((k) => k !== drag);
    const at = rest.indexOf(target) + (after ? 1 : 0);
    rest.splice(at, 0, drag);
    return { keys: rest, dir: rest.length < 4 ? dir : view.dir };
  }
  if (shown.length >= MAX_PANES) {
    const keys = [...shown];
    keys[to] = drag;
    return { ...view, keys };
  }
  const keys = [...shown];
  keys.splice(after ? to + 1 : to, 0, drag);
  return { keys, dir: keys.length < 4 ? dir : view.dir };
}
