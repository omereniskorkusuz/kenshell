/**
 * xterm.js host for one tab (lazy chunk). One Terminal per tab, created once and kept alive
 * while the tab is hidden (the pane is hidden, not unmounted). Output arrives through the
 * tab's TerminalSink; input goes to the tabs store.
 */
import { FitAddon } from '@xterm/addon-fit';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useEffect, useRef } from 'react';
import { useSettings } from '../stores/settings';
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
}

export default function TerminalView({ tabKey, visible }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<(() => void) | null>(null);
  const visibleRef = useRef(visible);

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
      sink.attach((data) => term.write(data));

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
      });

      if (visibleRef.current) term.focus();

      cleanups.push(() => {
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
    visibleRef.current = visible;
    getSink(tabKey)?.setVisible(visible);
    if (visible) {
      fitRef.current?.();
      // Keep focus where keyboard navigation is happening (tab strip), otherwise type here.
      const ae = document.activeElement;
      if (!ae || ae === document.body || !ae.closest('[role="tablist"]')) {
        termRef.current?.focus();
      }
    }
  }, [tabKey, visible]);

  return <div ref={hostRef} className="terminal-host min-h-0 flex-1" />;
}
