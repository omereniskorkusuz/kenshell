/**
 * xterm.js host for one tab (lazy chunk). The pane stays mounted while its tab is hidden, but
 * the Terminal itself only lives while the tab is shown (plus a short grace period for quick
 * tab switching): in WebKit one terminal with 10 000 lines of scrollback costs ~100 MB, while
 * the sink's text history of the same output is ~1 MB. A recreated terminal replays that
 * history, then continues with the output buffered meanwhile. Input goes to the tabs store.
 */
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal, type IMarker } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useEffect, useRef, useState } from 'react';
import { copyText, readText } from '../lib/clipboard';
import { isMac } from '../lib/ipc';
import { useSettings } from '../stores/settings';
import { useUi } from '../stores/ui';
import { registerTerminal, unregisterTerminal, type SearchOptions } from './registry';
import { cleanCopy, makeDecoder, Timestamper, type Charset, type Decoder } from './transform';
import { ownerKey, useNotes } from '../stores/notes';
import { useTabs } from '../stores/tabs';
import { CHARS_PER_LINE } from './history';
import { getSink } from './sink';
import { LineTracker, storable, suggest } from './autocomplete';
import { useAnalyzer } from '../stores/analyzer';
import { historyKey, useHistory } from '../stores/history';
import { LinkMenu, type LinkMenuState } from './LinkMenu';
import { findLinks } from './links';
import { Highlighter, highlightColors, type HighlighterConfig } from './highlighter';
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

/** How long a hidden tab keeps its terminal (switching back within it costs nothing). */
const HIDDEN_DISPOSE_MS = 5000;

/** Charset decoder and timestamp state, kept on the sink across terminal instances. */
interface TextState {
  charset: Charset;
  decode: Decoder;
  stamper: Timestamper | null;
}

/** User font first, then the bundled/system mono stack from the design tokens. */
const fontFamily = (custom: string) => (custom ? `${custom}, ${readMonoFont()}` : readMonoFont());

const historyLimit = (scrollback: number) => (scrollback + 200) * CHARS_PER_LINE;

export default function TerminalView({ tabKey, visible, focused }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<(() => void) | null>(null);
  const focusedRef = useRef(focused);
  const [linkMenu, setLinkMenu] = useState<LinkMenuState | null>(null);
  // The xterm instance exists while shown, and for a short while after being hidden.
  const [disposed, setDisposed] = useState(!visible);
  if (visible && disposed) setDisposed(false); // shown again: recreate (render-time update)
  const alive = !disposed;

  useEffect(() => {
    if (visible) return;
    const timer = setTimeout(() => setDisposed(true), HIDDEN_DISPOSE_MS);
    return () => clearTimeout(timer);
  }, [visible]);

  useEffect(() => {
    const host = hostRef.current;
    const sink = getSink(tabKey);
    if (!host || !sink || !alive) return;
    let disposed = false;
    const cleanups: (() => void)[] = [];

    void waitForFonts().then(() => {
      if (disposed) return;
      const s = useSettings.getState().values;
      const term = new Terminal({
        allowProposedApi: true, // unicode11 addon
        fontFamily: fontFamily(s['terminal.fontFamily']),
        fontSize: s['terminal.fontSize'],
        lineHeight: s['terminal.lineHeight'] || readLineHeight(),
        scrollback: s['terminal.scrollback'],
        theme: readTerminalTheme(),
        cursorStyle: s['terminal.cursorStyle'],
        cursorBlink: s['terminal.cursorBlink'],
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

      // ---- command history + dim autocomplete (P3-T12)
      const tracker = new LineTracker();
      const ghost = document.createElement('span');
      ghost.className = 'term-ghost';
      ghost.setAttribute('aria-hidden', 'true');
      ghost.hidden = true;
      host.appendChild(ghost);
      let ghostText: string | null = null;
      const tabNow = () => useTabs.getState().tabs.find((x) => x.key === tabKey);
      /** The screen shows `typed` right before the cursor, nothing after it. */
      const echoed = (typed: string) => {
        const b = term.buffer.active;
        const line = b.getLine(b.baseY + b.cursorY)?.translateToString(false) ?? '';
        return line.slice(0, b.cursorX).endsWith(typed) && line.slice(b.cursorX).trim() === '';
      };
      const hideGhost = () => {
        ghostText = null;
        ghost.hidden = true;
      };
      const updateGhost = () => {
        const typed = tracker.line;
        const t = tabNow();
        const b = term.buffer.active;
        if (
          !typed ||
          !t ||
          t.state !== 'connected' ||
          t.mode === 'unknown' ||
          b.type !== 'normal' ||
          b.viewportY !== b.baseY ||
          !useSettings.getState().values['terminal.autocomplete'] ||
          !echoed(typed)
        ) {
          hideGhost();
          return;
        }
        const history =
          useHistory.getState().byOwner[historyKey(tabKey, t.sessionId, t.device.deviceId)] ?? [];
        const rest = suggest(typed, history, t.device.vendor?.family ?? null);
        const screen = host.querySelector('.xterm-screen');
        if (!rest || !screen) {
          hideGhost();
          return;
        }
        const r = screen.getBoundingClientRect();
        const hr = host.getBoundingClientRect();
        const w = r.width / term.cols;
        const h = r.height / term.rows;
        ghostText = rest;
        ghost.textContent = rest;
        Object.assign(ghost.style, {
          left: `${r.left - hr.left + b.cursorX * w}px`,
          top: `${r.top - hr.top + b.cursorY * h}px`,
          height: `${h}px`,
          lineHeight: `${h}px`,
          maxWidth: `${Math.max(0, term.cols - b.cursorX) * w}px`,
          fontFamily: term.options.fontFamily ?? '',
          fontSize: `${term.options.fontSize ?? 13}px`,
        });
        ghost.hidden = false;
      };
      // A finished line is saved once the device has echoed it (passwords are not echoed):
      // checked when the cursor has left that row, within a few seconds.
      const pending: { line: string; marker: IMarker; at: number }[] = [];
      const rowText = (y: number) => {
        const b = term.buffer.active;
        let text = b.getLine(y)?.translateToString(true) ?? '';
        for (let n = y + 1; b.getLine(n)?.isWrapped; n++) {
          text += b.getLine(n)?.translateToString(true) ?? '';
        }
        return text;
      };
      /** `row` is a prompt followed by `want` (`sw1#show run`, `<HW>display …`). */
      const afterPrompt = (row: string, want: string) => {
        const r = row.trimEnd();
        if (!r.endsWith(want)) return false;
        return /[#>\]$%)]\s*$/.test(r.slice(0, r.length - want.length));
      };
      const checkPending = () => {
        const b = term.buffer.active;
        const cursorRow = b.baseY + b.cursorY;
        const now = Date.now();
        // Oldest first, so the history ends up newest first.
        for (const p of [...pending]) {
          if (p.marker.isDisposed || now - p.at > 5000) {
            p.marker.dispose();
            pending.splice(pending.indexOf(p), 1);
            continue;
          }
          // Typed ahead: the echo can land on a later row than the cursor was on at Enter.
          const want = p.line.trimEnd();
          let found = -1;
          for (let y = p.marker.line; y < cursorRow && y < p.marker.line + 200; y++) {
            if (!b.getLine(y)?.isWrapped && afterPrompt(rowText(y), want)) {
              found = y;
              break;
            }
          }
          if (found < 0) continue;
          const echo = rowText(found).trimEnd();
          // P4-T01: tell the analyzer once the prompt is back after this command.
          awaitDone = {
            command: p.line.trim(),
            prompt: echo.slice(0, echo.length - want.length).trimEnd(),
            row: found,
          };
          const t = tabNow();
          if (t && t.mode !== 'unknown') {
            const key = historyKey(tabKey, t.sessionId, t.device.deviceId);
            useHistory.getState().add(key, t.sessionId, t.device.deviceId, p.line);
          }
          p.marker.dispose();
          pending.splice(pending.indexOf(p), 1);
        }
      };
      let awaitDone: { command: string; prompt: string; row: number } | null = null;
      const checkDone = () => {
        if (!awaitDone) return;
        const b = term.buffer.active;
        const row = b.baseY + b.cursorY;
        if (row <= awaitDone.row) return;
        const line = b.getLine(row)?.translateToString(false) ?? '';
        if (line.slice(0, b.cursorX).trimEnd() !== awaitDone.prompt) return;
        if (line.slice(b.cursorX).trim() !== '') return;
        const { command } = awaitDone;
        awaitDone = null;
        void useAnalyzer.getState().commandDone(tabKey, command);
      };
      const record = (line: string) => {
        if (!storable(line) || pending.length > 20) return;
        const marker = term.registerMarker(0);
        if (marker) pending.push({ line, marker, at: Date.now() });
      };
      const ghostWatch = [
        term.onWriteParsed(() => {
          if (pending.length) checkPending();
          checkDone();
          updateGhost();
        }),
        term.onResize(hideGhost),
      ];

      const input = term.onData((typed) => {
        let data = typed;
        // Tab / → accept the suggestion (otherwise they go to the device as usual).
        if (ghostText && (data === '\t' || data === '\x1b[C' || data === '\x1bOC')) {
          data = ghostText;
        }
        for (const line of tracker.feed(data)) record(line);
        hideGhost();
        useTabs.getState().input(tabKey, data);
      });

      // ---- output: charset decoding + optional line timestamps (state outlives the terminal)
      const charset = s['terminal.charset'] as Charset;
      const prev = sink.textState as TextState | null;
      const text: TextState = {
        charset,
        decode: prev?.charset === charset ? prev.decode : makeDecoder(charset),
        stamper: s['terminal.timestamps'] ? (prev?.stamper ?? new Timestamper()) : null,
      };
      sink.textState = text;
      // Replay what this tab showed before (a hidden tab's terminal was disposed).
      sink.history.setLimit(historyLimit(s['terminal.scrollback']));
      for (const chunk of sink.history.replay()) term.write(chunk);
      sink.attach((data) => {
        const decoded = typeof data === 'string' ? data : text.decode(data);
        const shown = text.stamper ? text.stamper.feed(decoded) : decoded;
        sink.history.push(shown);
        term.write(shown);
      });

      // ---- clipboard
      const mac = isMac();
      const copySelection = () => {
        if (term.hasSelection()) void copyText(cleanCopy(term.getSelection()));
      };
      const paste = () => {
        void readText().then((text) => {
          if (!text) return;
          tracker.lose();
          hideGhost();
          useTabs.getState().pasteInput(tabKey, text);
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
          tracker.feed('\x08');
          useTabs.getState().input(tabKey, '\x08');
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

      // ---- interface note tooltips (P3-T09): hover an interface that has a note
      const tip = document.createElement('div');
      tip.className = 'term-tooltip';
      tip.setAttribute('role', 'tooltip');
      tip.hidden = true;
      document.body.appendChild(tip);
      const notesOf = () => {
        const tab = useTabs.getState().tabs.find((x) => x.key === tabKey);
        if (!tab) return undefined;
        return useNotes.getState().byOwner[ownerKey(tab.sessionId, tab.device.deviceId)];
      };
      // ---- output colouring (P3-T10), visible rows only
      const familyOf = () =>
        useTabs.getState().tabs.find((x) => x.key === tabKey)?.device.vendor?.family ?? null;
      const highlightConfig = (): HighlighterConfig => {
        const v = useSettings.getState().values;
        return {
          enabled: v['terminal.highlight'],
          family: familyOf(),
          off: v['terminal.highlightOff'],
          colors: highlightColors(term.options.theme ?? {}),
        };
      };
      const highlighter = new Highlighter(term, highlightConfig());
      let family = familyOf();
      const unsubFamily = useTabs.subscribe(() => {
        const f = familyOf();
        if (f !== family) {
          family = f;
          highlighter.configure(highlightConfig());
        }
      });

      // Clickable IPs / interfaces (P3-T11); an interface with a note shows it on hover.
      const links = term.registerLinkProvider({
        provideLinks(y, done) {
          const line = term.buffer.active.getLine(y - 1)?.translateToString(true) ?? '';
          const notes = notesOf();
          const found = findLinks(line).map((l) => {
            const note = l.kind === 'iface' ? notes?.[l.iface.key] : undefined;
            return {
              range: { start: { x: l.start + 1, y }, end: { x: l.end, y } },
              text: l.text,
              decorations: { pointerCursor: true, underline: true },
              activate: (e: MouseEvent) => {
                tip.hidden = true;
                setLinkMenu({ x: e.clientX, y: e.clientY + 4, link: l });
              },
              hover: (e: MouseEvent) => {
                if (note === undefined) return;
                tip.textContent = note.slice(0, 500);
                tip.style.left = `${Math.min(e.clientX + 12, window.innerWidth - 320)}px`;
                tip.style.top = `${e.clientY + 16}px`;
                tip.hidden = false;
              },
              leave: () => {
                tip.hidden = true;
              },
            };
          });
          done(found.length ? found : undefined);
        },
      });

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
        selection: () => term.getSelection(),
        lastOutput: () => {
          // The rows between the last `prompt command` line and the current prompt.
          const b = term.buffer.active;
          const cur = b.baseY + b.cursorY;
          const prompt = (b.getLine(cur)?.translateToString(false) ?? '')
            .slice(0, b.cursorX)
            .trimEnd();
          if (!/[#>\]$%)]$/.test(prompt)) return null;
          for (let y = cur - 1; y >= 0 && y > cur - 20000; y--) {
            if (b.getLine(y)?.isWrapped) continue;
            const row = rowText(y).trimEnd();
            if (!row.startsWith(prompt) || row.length === prompt.length) continue;
            const command = row.slice(prompt.length).trim();
            const lines: string[] = [];
            for (let n = y + 1; n < cur; n++) {
              const l = b.getLine(n);
              if (!l) break;
              const text = l.translateToString(true);
              if (l.isWrapped && lines.length) lines[lines.length - 1] += text;
              else lines.push(text);
            }
            return { command, text: lines.join('\n') };
          }
          return null;
        },
      });

      // Theme changes: re-read tokens whenever <html data-theme> flips.
      const mo = new MutationObserver(() => {
        term.options.theme = readTerminalTheme();
        highlighter.configure(highlightConfig());
      });
      mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });

      const unsubSettings = useSettings.subscribe((st, prev) => {
        const v = st.values;
        const p = prev.values;
        if (v['terminal.fontSize'] !== p['terminal.fontSize']) {
          term.options.fontSize = v['terminal.fontSize'];
          doFit();
        }
        if (v['terminal.fontFamily'] !== p['terminal.fontFamily']) {
          term.options.fontFamily = fontFamily(v['terminal.fontFamily']);
          doFit();
        }
        if (v['terminal.lineHeight'] !== p['terminal.lineHeight']) {
          term.options.lineHeight = v['terminal.lineHeight'];
          doFit();
        }
        if (v['terminal.cursorStyle'] !== p['terminal.cursorStyle']) {
          term.options.cursorStyle = v['terminal.cursorStyle'];
        }
        if (v['terminal.cursorBlink'] !== p['terminal.cursorBlink']) {
          term.options.cursorBlink = v['terminal.cursorBlink'];
        }
        if (v['terminal.scrollback'] !== p['terminal.scrollback']) {
          term.options.scrollback = v['terminal.scrollback'];
          sink.history.setLimit(historyLimit(v['terminal.scrollback']));
        }
        if (v['terminal.charset'] !== p['terminal.charset']) {
          text.charset = v['terminal.charset'] as Charset;
          text.decode = makeDecoder(text.charset);
        }
        if (
          v['terminal.highlight'] !== p['terminal.highlight'] ||
          v['terminal.highlightOff'] !== p['terminal.highlightOff']
        ) {
          highlighter.configure(highlightConfig());
        }
        if (v['terminal.timestamps'] !== p['terminal.timestamps']) {
          text.stamper = v['terminal.timestamps'] ? new Timestamper() : null;
        }
      });

      if (focusedRef.current) term.focus();

      cleanups.push(() => {
        ghostWatch.forEach((d) => d.dispose());
        pending.forEach((p) => p.marker.dispose());
        ghost.remove();
        highlighter.dispose();
        unsubFamily();
        links.dispose();
        tip.remove();
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
  }, [tabKey, alive]);

  // Interface notes of this tab's device / session (for the tooltips).
  const noteSession = useTabs((s) => s.tabs.find((x) => x.key === tabKey)?.sessionId ?? null);
  const noteDevice = useTabs((s) => s.tabs.find((x) => x.key === tabKey)?.device.deviceId ?? null);
  useEffect(() => {
    void useNotes.getState().loadOwner(noteSession, noteDevice);
    void useHistory.getState().load(noteSession, noteDevice);
  }, [noteSession, noteDevice]);

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

  return (
    <>
      <div ref={hostRef} className="terminal-host min-h-0 flex-1" />
      {linkMenu && <LinkMenu tabKey={tabKey} menu={linkMenu} onClose={() => setLinkMenu(null)} />}
    </>
  );
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
