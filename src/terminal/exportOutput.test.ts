import { describe, expect, it } from 'vitest';
import {
  bufferToHtml,
  bufferToText,
  palette,
  type BufferLike,
  type CellLike,
} from './exportOutput';

interface Attr {
  fg?: number | string;
  bg?: number;
  bold?: boolean;
  inverse?: boolean;
}

function cell(ch: string, a: Attr = {}): CellLike {
  const rgb = typeof a.fg === 'string';
  return {
    getChars: () => ch,
    getWidth: () => 1,
    isFgDefault: () => a.fg === undefined,
    isFgPalette: () => typeof a.fg === 'number',
    isFgRGB: () => rgb,
    getFgColor: () => (rgb ? parseInt((a.fg as string).slice(1), 16) : ((a.fg as number) ?? 0)),
    isBgDefault: () => a.bg === undefined,
    isBgPalette: () => a.bg !== undefined,
    isBgRGB: () => false,
    getBgColor: () => a.bg ?? 0,
    isBold: () => (a.bold ? 1 : 0),
    isItalic: () => 0,
    isDim: () => 0,
    isUnderline: () => 0,
    isInverse: () => (a.inverse ? 1 : 0),
    isInvisible: () => 0,
    isStrikethrough: () => 0,
  };
}

function buffer(rows: { cells: CellLike[]; wrapped?: boolean }[], width = 10): BufferLike {
  return {
    length: rows.length,
    getLine: (y) => {
      const r = rows[y];
      if (!r) return undefined;
      const text = r.cells.map((c) => c.getChars()).join('');
      return {
        isWrapped: !!r.wrapped,
        length: width,
        getCell: (x) => r.cells[x] ?? cell(' '),
        translateToString: (trim) => (trim ? text.replace(/\s+$/, '') : text.padEnd(width)),
      };
    },
  };
}

const text = (s: string, a?: Attr) => [...s].map((c) => cell(c, a));
const THEME = { background: '#101010', foreground: '#e0e0e0', red: '#ff5555', green: '#50fa7b' };

describe('export', () => {
  const b = buffer([
    { cells: [...text('sw1#'), ...text('up', { fg: 2, bold: true })] },
    { cells: text('long-line-') },
    { cells: text('continued'), wrapped: true },
    { cells: text('<x> & "y"', { fg: '#123456' }) },
    { cells: text('inv', { inverse: true }) },
    { cells: [] },
  ]);

  it('exports text with wrapped rows joined', () => {
    expect(bufferToText(b)).toBe('sw1#up\nlong-line-continued\n<x> & "y"\ninv\n');
  });

  it('exports coloured, escaped HTML', () => {
    const html = bufferToHtml(b, THEME, 'core-sw1 <log>', 'monospace');
    expect(html).toContain('<title>core-sw1 &lt;log&gt;</title>');
    expect(html).toContain('background:#101010');
    expect(html).toContain('sw1#<span style="color:#50fa7b;font-weight:bold">up</span>');
    expect(html).toContain('long-line-continued');
    expect(html).toContain('<span style="color:#123456">&lt;x&gt; &amp; &quot;y&quot;</span>');
    expect(html).toContain('<span style="color:#101010;background:#e0e0e0">inv</span>');
    expect(html.trimEnd().endsWith('inv</span></pre></body></html>')).toBe(true);
  });

  it('builds the 256-colour palette', () => {
    const p = palette(THEME);
    expect(p).toHaveLength(256);
    expect(p[1]).toBe('#ff5555');
    expect(p[16]).toBe('#000000');
    expect(p[231]).toBe('#ffffff');
    expect(p[232]).toBe('#080808');
    expect(p[255]).toBe('#eeeeee');
  });
});
