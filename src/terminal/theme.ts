/** xterm theme read from the CSS design tokens (no literal colors in code). */
import type { ITheme } from '@xterm/xterm';

const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'] as const;

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

export function readTerminalTheme(root: Element = document.documentElement): ITheme {
  const cs = getComputedStyle(root);
  const v = (name: string) => cs.getPropertyValue(name).trim() || undefined;
  const theme: Record<string, string | undefined> = {
    background: v('--bg-terminal'),
    foreground: v('--text-terminal'),
    cursor: v('--cursor-terminal'),
    cursorAccent: v('--bg-terminal'),
    selectionBackground: v('--selection-terminal'),
    scrollbarSliderBackground: v('--scrollbar-thumb'),
    scrollbarSliderHoverBackground: v('--border-strong'),
    scrollbarSliderActiveBackground: v('--text-muted'),
  };
  for (const name of ANSI) {
    theme[name] = v(`--ansi-${name}`);
    theme[`bright${cap(name)}`] = v(`--ansi-bright-${name}`);
  }
  // Drop unresolved entries so xterm keeps its defaults instead of "undefined".
  return Object.fromEntries(Object.entries(theme).filter(([, c]) => c)) as ITheme;
}

export function readLineHeight(root: Element = document.documentElement): number {
  const n = Number(getComputedStyle(root).getPropertyValue('--leading-terminal').trim());
  return Number.isFinite(n) && n >= 1 && n <= 2 ? n : 1.2;
}

export function readMonoFont(root: Element = document.documentElement): string {
  return getComputedStyle(root).getPropertyValue('--font-mono').trim() || 'monospace';
}
