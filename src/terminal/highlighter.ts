/**
 * Applies output colouring (P3-T10) to the rows on screen only, as xterm decorations: the
 * stream, the scrollback history and the log stay untouched. Re-runs (debounced) after
 * output, scrolling and resizing; a row whose text did not change keeps its decorations.
 */
import type { IDecoration, IMarker, ITheme, Terminal } from '@xterm/xterm';
import { compileRules, highlightLine, type CompiledRules, type HighlightStyle } from './highlight';

interface Row {
  text: string;
  marker: IMarker;
  decorations: IDecoration[];
}

const DELAY_MS = 40;
const HEX = /^#[0-9a-f]{6}$/i;

type Colors = Partial<Record<HighlightStyle, { fg?: string | undefined; bg?: string | undefined }>>;

/** Colours from the terminal theme (design tokens), so they follow dark/light. */
export function highlightColors(theme: ITheme): Colors {
  const hex = (c: string | undefined) => (c && HEX.test(c) ? c : undefined);
  return {
    up: { fg: hex(theme.green) },
    down: { fg: hex(theme.red) },
    warning: { fg: hex(theme.yellow) },
    error: { fg: hex(theme.background), bg: hex(theme.red) },
    ip: { fg: hex(theme.cyan) },
    mac: { fg: hex(theme.magenta) },
    iface: { fg: hex(theme.brightBlue) },
  };
}

export interface HighlighterConfig {
  enabled: boolean;
  family: string | null;
  off: readonly string[];
  colors: Colors;
}

export class Highlighter {
  private rows = new Map<number, Row>();
  private timer = 0;
  private disposed = false;
  private rules: CompiledRules | null = null;
  private config: HighlighterConfig;
  private readonly subs: { dispose(): void }[];

  constructor(
    private readonly term: Terminal,
    config: HighlighterConfig,
  ) {
    this.config = config;
    this.subs = [
      term.onWriteParsed(() => this.schedule()),
      term.onScroll(() => this.schedule()),
      term.onResize(() => this.reset()),
      term.buffer.onBufferChange(() => this.reset()),
    ];
    this.schedule();
  }

  configure(config: HighlighterConfig): void {
    if (this.disposed) return;
    this.config = config;
    this.rules = null;
    this.reset();
  }

  dispose(): void {
    this.disposed = true;
    window.clearTimeout(this.timer);
    for (const s of this.subs) s.dispose();
    this.clear();
  }

  private reset(): void {
    this.clear();
    this.schedule();
  }

  private clear(): void {
    for (const r of this.rows.values()) this.drop(r);
    this.rows.clear();
  }

  private drop(r: Row): void {
    for (const d of r.decorations) d.dispose();
    r.marker.dispose();
  }

  private schedule(): void {
    if (this.timer) return;
    this.timer = window.setTimeout(() => {
      this.timer = 0;
      this.refresh();
    }, DELAY_MS);
  }

  private refresh(): void {
    const { term, config } = this;
    const buf = term.buffer.active;
    if (!config.enabled || buf.type === 'alternate') {
      this.clear();
      return;
    }
    this.rules ??= compileRules(config.family, config.off);
    const cursorAbs = buf.baseY + buf.cursorY;
    const next = new Map<number, Row>();
    for (let y = buf.viewportY; y < buf.viewportY + term.rows; y++) {
      const text = buf.getLine(y)?.translateToString(true) ?? '';
      const old = this.rows.get(y);
      if (old && !old.marker.isDisposed && old.marker.line === y && old.text === text) {
        next.set(y, old);
        this.rows.delete(y);
        continue;
      }
      const spans = highlightLine(text, this.rules);
      if (!spans.length) continue;
      const marker = term.registerMarker(y - cursorAbs);
      if (!marker) continue;
      const decorations: IDecoration[] = [];
      for (const s of spans) {
        const c = config.colors[s.style];
        if (!c?.fg && !c?.bg) continue;
        const d = term.registerDecoration({
          marker,
          x: s.start,
          width: s.end - s.start,
          layer: 'bottom',
          ...(c.fg ? { foregroundColor: c.fg } : {}),
          ...(c.bg ? { backgroundColor: c.bg } : {}),
        });
        if (d) decorations.push(d);
      }
      next.set(y, { text, marker, decorations });
    }
    // Rows that scrolled away or changed.
    for (const r of this.rows.values()) this.drop(r);
    this.rows = next;
  }
}
