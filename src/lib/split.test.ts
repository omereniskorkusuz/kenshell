import { describe, expect, it } from 'vitest';
import {
  activateIn,
  addPane,
  cellRect,
  dropOn,
  gridOf,
  moveItem,
  neighbor,
  removePane,
  zoneAt,
  type SplitView,
} from './split';

const single: SplitView = { keys: [], dir: 'cols' };

describe('split grid', () => {
  it('lays out 1–4 panes', () => {
    expect(gridOf(1, 'cols')).toEqual({ cols: 1, rows: 1 });
    expect(gridOf(2, 'cols')).toEqual({ cols: 2, rows: 1 });
    expect(gridOf(3, 'rows')).toEqual({ cols: 1, rows: 3 });
    expect(gridOf(4, 'rows')).toEqual({ cols: 2, rows: 2 });
    expect(cellRect(3, 4, 'cols')).toEqual({ left: 50, top: 50, width: 50, height: 50 });
    expect(cellRect(1, 2, 'rows')).toEqual({ left: 0, top: 50, width: 100, height: 50 });
  });

  it('finds neighbours by arrow', () => {
    expect(neighbor(0, 4, 'cols', 'right')).toBe(1);
    expect(neighbor(0, 4, 'cols', 'down')).toBe(2);
    expect(neighbor(3, 4, 'cols', 'up')).toBe(1);
    expect(neighbor(0, 2, 'cols', 'down')).toBeNull();
    expect(neighbor(1, 3, 'rows', 'down')).toBe(2);
    expect(neighbor(2, 3, 'rows', 'down')).toBeNull();
  });

  it('picks the nearest edge zone or the center', () => {
    const r = { left: 0, top: 0, width: 400, height: 200 };
    expect(zoneAt(r, 20, 100)).toBe('left');
    expect(zoneAt(r, 390, 100)).toBe('right');
    expect(zoneAt(r, 200, 10)).toBe('top');
    expect(zoneAt(r, 200, 190)).toBe('bottom');
    expect(zoneAt(r, 200, 100)).toBe('center');
  });
});

describe('split edits', () => {
  it('moves items', () => {
    expect(moveItem(['a', 'b', 'c'], 0, 2)).toEqual(['b', 'c', 'a']);
    expect(moveItem(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b']);
    expect(moveItem(['a', 'b'], 0, 9)).toEqual(['b', 'a']);
  });

  it('adds panes up to four, keeping the first direction', () => {
    const two = addPane(single, 'a', 'b', 'rows');
    expect(two).toEqual({ keys: ['a', 'b'], dir: 'rows' });
    const three = addPane(two as SplitView, 'a', 'c', 'cols');
    expect(three).toEqual({ keys: ['a', 'c', 'b'], dir: 'rows' });
    const four = addPane(three as SplitView, 'b', 'd', 'cols');
    expect(four?.keys).toEqual(['a', 'c', 'b', 'd']);
    expect(addPane(four as SplitView, 'a', 'e', 'cols')).toBeNull();
    expect(addPane(single, 'a', 'a', 'cols')).toBeNull();
    expect(addPane(two as SplitView, 'a', 'b', 'cols')).toBeNull();
  });

  it('removes panes and falls back to single view', () => {
    const v: SplitView = { keys: ['a', 'b', 'c'], dir: 'cols' };
    expect(removePane(v, 'b').keys).toEqual(['a', 'c']);
    expect(removePane({ keys: ['a', 'b'], dir: 'cols' }, 'a').keys).toEqual([]);
  });

  it('activating an outside tab replaces the focused pane', () => {
    const v: SplitView = { keys: ['a', 'b'], dir: 'cols' };
    expect(activateIn(v, 'b', 'c').keys).toEqual(['a', 'c']);
    expect(activateIn(v, 'b', 'a')).toBe(v);
    expect(activateIn(single, 'a', 'c')).toBe(single);
  });

  it('drops tabs on pane edges and centers', () => {
    expect(dropOn(single, 'a', 'b', 'a', 'left')).toEqual({ keys: ['b', 'a'], dir: 'cols' });
    expect(dropOn(single, 'a', 'b', 'a', 'bottom')).toEqual({ keys: ['a', 'b'], dir: 'rows' });
    expect(dropOn(single, 'a', 'b', 'a', 'center')).toBe(single);
    const v: SplitView = { keys: ['a', 'b', 'c'], dir: 'cols' };
    expect(dropOn(v, 'a', 'c', 'a', 'center').keys).toEqual(['c', 'b', 'a']);
    expect(dropOn(v, 'a', 'd', 'b', 'center').keys).toEqual(['a', 'd', 'c']);
    expect(dropOn(v, 'a', 'c', 'a', 'left').keys).toEqual(['c', 'a', 'b']);
    expect(dropOn(v, 'a', 'd', 'c', 'right').keys).toEqual(['a', 'b', 'c', 'd']);
    const full: SplitView = { keys: ['a', 'b', 'c', 'd'], dir: 'cols' };
    expect(dropOn(full, 'a', 'e', 'b', 'left').keys).toEqual(['a', 'e', 'c', 'd']);
    expect(dropOn(v, 'a', 'x', 'zz', 'left')).toBe(v);
  });
});
