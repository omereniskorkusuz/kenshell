/**
 * Terminal output export (P5-T06): the whole scrollback as plain text or as a
 * standalone, coloured HTML page. Works on xterm's buffer API through a minimal
 * structural type, so it is testable without a DOM and the replay player can use it
 * too. Colours come from the terminal theme (design tokens) at export time.
 */
import type { ITheme } from '@xterm/xterm';

export interface CellLike {
  getChars(): string;
  getWidth(): number;
  isFgDefault(): boolean;
  isFgPalette(): boolean;
  isFgRGB(): boolean;
  getFgColor(): number;
  isBgDefault(): boolean;
  isBgPalette(): boolean;
  isBgRGB(): boolean;
  getBgColor(): number;
  isBold(): number;
  isItalic(): number;
  isDim(): number;
  isUnderline(): number;
  isInverse(): number;
  isInvisible(): number;
  isStrikethrough(): number;
}

export interface LineLike {
  readonly isWrapped: boolean;
  readonly length: number;
  getCell(x: number): CellLike | undefined;
  translateToString(trimRight?: boolean): string;
}

export interface BufferLike {
  readonly length: number;
  getLine(y: number): LineLike | undefined;
}

/** Plain text; soft-wrapped rows are joined back into one line, trailing blanks dropped. */
export function bufferToText(b: BufferLike): string {
  const out: string[] = [];
  for (let y = 0; y < b.length; y++) {
    const line = b.getLine(y);
    if (!line) continue;
    const text = line.translateToString(true);
    if (line.isWrapped && out.length) out[out.length - 1] += text;
    else out.push(text);
  }
  while (out.length && out[out.length - 1] === '') out.pop();
  return out.join('\n') + '\n';
}

const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'] as const;
const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
const hex = (n: number) => `#${n.toString(16).padStart(6, '0')}`;

/** xterm's 256-colour palette: 16 theme colours, the 6×6×6 cube, 24 greys. */
export function palette(theme: ITheme): string[] {
  const t = theme as Record<string, string | undefined>;
  const p: string[] = [];
  for (const prefix of ['', 'bright']) {
    for (const name of ANSI) {
      const key = prefix ? `bright${cap(name)}` : name;
      p.push(t[key] ?? t.foreground ?? 'inherit');
    }
  }
  const level = (v: number) => (v === 0 ? 0 : 55 + v * 40);
  for (let i = 0; i < 216; i++) {
    const r = level(Math.floor(i / 36));
    const g = level(Math.floor(i / 6) % 6);
    const b = level(i % 6);
    p.push(hex((r << 16) | (g << 8) | b));
  }
  for (let i = 0; i < 24; i++) {
    const v = 8 + i * 10;
    p.push(hex((v << 16) | (v << 8) | v));
  }
  return p;
}

const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

function color(cell: CellLike, fg: boolean, pal: string[]): string | null {
  if (fg ? cell.isFgRGB() : cell.isBgRGB()) return hex(fg ? cell.getFgColor() : cell.getBgColor());
  if (fg ? cell.isFgPalette() : cell.isBgPalette())
    return pal[fg ? cell.getFgColor() : cell.getBgColor()] ?? null;
  return null;
}

function style(cell: CellLike, pal: string[], theme: ITheme): string {
  let fg = color(cell, true, pal);
  let bg = color(cell, false, pal);
  if (cell.isInverse()) {
    [fg, bg] = [bg ?? theme.background ?? null, fg ?? theme.foreground ?? null];
  }
  const css: string[] = [];
  if (fg) css.push(`color:${fg}`);
  if (bg) css.push(`background:${bg}`);
  if (cell.isBold()) css.push('font-weight:bold');
  if (cell.isItalic()) css.push('font-style:italic');
  if (cell.isDim()) css.push('opacity:.6');
  const deco = [cell.isUnderline() && 'underline', cell.isStrikethrough() && 'line-through']
    .filter(Boolean)
    .join(' ');
  if (deco) css.push(`text-decoration:${deco}`);
  if (cell.isInvisible()) css.push('visibility:hidden');
  return css.join(';');
}

/** A standalone HTML page with the buffer's text and colours. */
export function bufferToHtml(b: BufferLike, theme: ITheme, title: string, font: string): string {
  const pal = palette(theme);
  const rows: string[] = [];
  for (let y = 0; y < b.length; y++) {
    const line = b.getLine(y);
    if (!line) continue;
    // Trailing blank cells are not exported.
    const used = line.translateToString(true).length;
    let html = '';
    let run = '';
    let runStyle = '';
    const flush = () => {
      if (!run) return;
      html += runStyle ? `<span style="${runStyle}">${escapeHtml(run)}</span>` : escapeHtml(run);
      run = '';
    };
    for (let x = 0; x < line.length && x < used; x++) {
      const cell = line.getCell(x);
      if (!cell || cell.getWidth() === 0) continue;
      const s = style(cell, pal, theme);
      if (s !== runStyle) {
        flush();
        runStyle = s;
      }
      run += cell.getChars() || ' ';
    }
    flush();
    if (line.isWrapped && rows.length) rows[rows.length - 1] += html;
    else rows.push(html);
  }
  while (rows.length && rows[rows.length - 1] === '') rows.pop();
  const bg = theme.background ?? 'inherit';
  const fg = theme.foreground ?? 'inherit';
  return `<!doctype html>
<html><head><meta charset="utf-8"><title>${escapeHtml(title)}</title>
<style>body{margin:0;background:${bg};color:${fg}}pre{margin:0;padding:16px;font:13px/1.35 ${font};white-space:pre-wrap;word-break:break-all}</style>
</head><body><pre>${rows.join('\n')}</pre></body></html>
`;
}
