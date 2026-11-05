/**
 * xterm.js host for one tab (lazy chunk). One Terminal per tab, created once and kept alive
 * while the tab is hidden (the pane is hidden, not unmounted). Output arrives through the
 * tab's TerminalSink; input goes to the tabs store.
 */
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useEffect, useRef } from 'react';
import { copyText, readText } from '../lib/clipboard';
import { isMac } from '../lib/ipc';
import { useSettings } from '../stores/settings';
import { useUi } from '../stores/ui';
import { registerTerminal, unregisterTerminal, type SearchOptions } from './registry';
import { cleanCopy, makeDecoder, Timestamper, type Charset } from './transform';
import { useTabs } from '../stores/tabs';
import { getSink } from './sink';
import { readLineHeight, readMonoFont, readTerminalTheme } from './theme';
import './terminal.css';

/** Resolves once the embedded mono font is usable (or after a short timeout). */
let fontsReady: Promise<void> | null = null;
function waitForFonts(): Promise<void> {
  fontsReady ??= Promise.race([
    Promise.all([
      document.fonts.load('400 13px "JetBrains Mono"'),
      document.fonts.load('700 13px "JetBrains Mono"'),
    ]).then(
      () => undefined,
      () => undefined,
    ),
    new Promise<void>((resolve) => setTimeout(resolve, 1500)),
  ]);
  return fontsReady;
}

/** GPU renderer when available; on failure or context loss xterm falls back to the DOM one. */
function tryWebgl(term: Terminal): void {
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => webgl.dispose());
    term.loadAddon(webgl);
  } catch {
    // No WebGL2 (old GPU / disabled): the default DOM renderer stays active.
  }
}

interface Props {
  tabKey: string;
  visible: boolean;
  /** Takes keyboard focus (the active pane; in split view several panes are visible). */
  focused: boolean;
}

/** Lines of scrollback carried over when a tab moves to another window. */
const SNAPSHOT_LINES = 10_000;

function bufferText(term: Terminal): string {
  const buf = term.buffer.active;
  const lines: string[] = [];
  for (let i = Math.max(0, buf.length - SNAPSHOT_LINES); i < buf.length; i++) {
    const line = buf.getLine(i);
    if (!line) continue;
    const text = line.translateToString(true);
    // A wrapped row continues the previous line.
    if (line.isWrapped && lines.length > 0) lines[lines.length - 1] += text;
    else lines.push(text);
  }
  while (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
  return lines.join('\r\n');
}

export default function TerminalView({ tabKey, visible, focused }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<(() => void) | null>(null);
  const focusedRef = useRef(focused);

  useEffect(() => {
    const host = hostRef.current;
    const sink = getSink(tabKey);
    if (!host || !sink) return;
    let disposed = false;
    const cleanups: (() => void)[] = [];

    void waitForFonts().then(() => {
      if (disposed) return;
      const s = useSettings.getState().values;
      const term = new Terminal({
        allowProposedApi: true, // unicode11 addon
        fontFamily: readMonoFont(),
        fontSize: s['terminal.fontSize'],
        lineHeight: readLineHeight(),
        scrollback: s['terminal.scrollback'],
        theme: readTerminalTheme(),
        cursorBlink: true,
        cursorInactiveStyle: 'outline',
        drawBoldTextInBrightColors: false,
        macOptionIsMeta: true,
        rescaleOverlappingGlyphs: true,
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      const search = new SearchAddon();
      term.loadAddon(search);
      term.loadAddon(new Unicode11Addon());
      term.unicode.activeVersion = '11';
      term.open(host);
      tryWebgl(term);
      termRef.current = term;

      const doFit = () => {
        if (!host.offsetWidth || !host.offsetHeight) return; // hidden: keep last size
        try {
          fit.fit();
        } catch {
          return;
        }
        sink.cols = term.cols;
        sink.rows = term.rows;
        useTabs.getState().resize(tabKey, term.cols, term.rows);
      };
      fitRef.current = doFit;
      let fitFrame = 0;
      const ro = new ResizeObserver(() => {
        cancelAnimationFrame(fitFrame);
        fitFrame = requestAnimationFrame(doFit);
      });
      ro.observe(host);
      doFit();

      const input = term.onData((data) => useTabs.getState().write(tabKey, data));

      // ---- output: charset decoding + optional line timestamps
      let decode = makeDecoder(s['terminal.charset'] as Charset);
      let stamper: Timestamper | null = s['terminal.timestamps'] ? new Timestamper() : null;
      sink.attach((data) => {
        const decoded = typeof data === 'string' ? data : decode(data);
        term.write(stamper ? stamper.feed(decoded) : decoded);
      });

      // ---- clipboard
      const mac = isMac();
      const copySelection = () => {
        if (term.hasSelection()) void copyText(cleanCopy(term.getSelection()));
      };
      const paste = () => {
        void readText().then((text) => {
          if (text) useTabs.getState().paste(tabKey, text);
        });
      };
      term.attachCustomKeyEventHandler((e) => {
        if (e.type !== 'keydown') return true;
        const k = e.key.toLowerCase();
        const copyKey = mac
          ? e.metaKey && !e.shiftKey && k === 'c'
          : (e.ctrlKey && e.shiftKey && k === 'c') || (e.ctrlKey && k === 'insert');
        const pasteKey = mac
          ? e.metaKey && !e.shiftKey && k === 'v'
          : (e.ctrlKey && e.shiftKey && k === 'v') || (e.shiftKey && k === 'insert');
        if (copyKey) {
          copySelection();
          return false;
        }
        if (pasteKey) {
          e.preventDefault();
          paste();
          return false;
        }
        if (
          k === 'backspace' &&
          !e.ctrlKey &&
          !e.metaKey &&
          !e.altKey &&
          useSettings.getState().values['terminal.backspace'] === 'ctrlH'
        ) {
          useTabs.getState().write(tabKey, '\x08');
          return false;
        }
        return true;
      });
      let selTimer = 0;
      const selection = term.onSelectionChange(() => {
        if (!useSettings.getState().values['terminal.copyOnSelect']) return;
        window.clearTimeout(selTimer);
        selTimer = window.setTimeout(copySelection, 150);
      });
      const onContextMenu = (e: MouseEvent) => {
        if (!useSettings.getState().values['terminal.rightClickPaste']) return;
        e.preventDefault();
        paste();
      };
      host.addEventListener('contextmenu', onContextMenu);

      // ---- search (Cmd/Ctrl+F bar)
      const decorations = readSearchDecorations();
      const searchOpts = (o: SearchOptions) => ({
        regex: o.regex,
        caseSensitive: o.caseSensitive,
        decorations,
      });
      registerTerminal(tabKey, {
        findNext: (q, o) => search.findNext(q, searchOpts(o)),
        findPrevious: (q, o) => search.findPrevious(q, searchOpts(o)),
        clearSearch: () => {
          search.clearDecorations();
          term.clearSelection();
        },
        focus: () => term.focus(),
        snapshot: () => new Promise((resolve) => term.write('', () => resolve(bufferText(term)))),
      });

      // Theme changes: re-read tokens whenever <html data-theme> flips.
      const mo = new MutationObserver(() => {
        term.options.theme = readTerminalTheme();
      });
      mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });

      const unsubSettings = useSettings.subscribe((st, prev) => {
        const v = st.values;
        const p = prev.values;
        if (v['terminal.fontSize'] !== p['terminal.fontSize']) {
          term.options.fontSize = v['terminal.fontSize'];
          doFit();
        }
        if (v['terminal.scrollback'] !== p['terminal.scrollback']) {
          term.options.scrollback = v['terminal.scrollback'];
        }
        if (v['terminal.charset'] !== p['terminal.charset']) {
          decode = makeDecoder(v['terminal.charset'] as Charset);
        }
        if (v['terminal.timestamps'] !== p['terminal.timestamps']) {
          stamper = v['terminal.timestamps'] ? new Timestamper() : null;
        }
      });

      if (focusedRef.current) term.focus();

      cleanups.push(() => {
        window.clearTimeout(selTimer);
        selection.dispose();
        host.removeEventListener('contextmenu', onContextMenu);
        unregisterTerminal(tabKey);
        if (useUi.getState().findKey === tabKey) useUi.getState().closeFind();
        cancelAnimationFrame(fitFrame);
        ro.disconnect();
        mo.disconnect();
        unsubSettings();
        input.dispose();
        sink.detach();
        fitRef.current = null;
        termRef.current = null;
        term.dispose();
      });
    });

    return () => {
      disposed = true;
      cleanups.forEach((fn) => fn());
    };
  }, [tabKey]);

  useEffect(() => {
    getSink(tabKey)?.setVisible(visible);
    if (visible) fitRef.current?.();
  }, [tabKey, visible]);

  useEffect(() => {
    focusedRef.current = focused;
    if (!focused) return;
    // Keep focus where keyboard navigation is happening (tab strip), otherwise type here.
    const ae = document.activeElement;
    if (!ae || ae === document.body || !ae.closest('[role="tablist"]')) {
      termRef.current?.focus();
    }
  }, [tabKey, focused]);

  return <div ref={hostRef} className="terminal-host min-h-0 flex-1" />;
}

/** Search highlight colors from the design tokens (warning = all matches, accent = current). */
function readSearchDecorations() {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string, fb: string) => css.getPropertyValue(name).trim() || fb;
  return {
    matchBackground: v('--warning', '#e8b341') + '55',
    matchOverviewRuler: v('--warning', '#e8b341'),
    activeMatchBackground: v('--warning', '#e8b341'),
    activeMatchColorOverviewRuler: v('--accent', '#d5dce6'),
  };
}
