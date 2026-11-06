/**
 * Recording player (P5-T06): plays an asciicast v2 file (KenShell's own recordings or
 * any other) in a read-only terminal at its recorded size. Play / pause, speed, seek
 * (backwards replays from the start, which is instant), long pauses shortened to 2 s,
 * and the same TXT / HTML export as a live tab.
 */
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { Pause, Play, RotateCcw } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Dialog } from '../components/Dialog';
import { useT } from '../i18n';
import { formatClock, indexAt, parseCast, parseSize, type Cast } from '../lib/cast';
import { describeError } from '../lib/errors';
import { api } from '../lib/ipc';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';
import { bufferToHtml, bufferToText } from './exportOutput';
import { readLineHeight, readMonoFont, readTerminalTheme } from './theme';

const SPEEDS = [0.5, 1, 2, 4, 8] as const;
const SEEK_STEP = 5;

interface Engine {
  term: Terminal;
  cast: Cast;
  /** Position on the timeline (seconds). */
  pos: number;
  /** Next event to play. */
  idx: number;
}

/** Writes events from `e.idx` up to time `to` in one batch. */
function playTo(e: Engine, to: number) {
  const end = indexAt(e.cast.events, to);
  let out = '';
  for (let i = e.idx; i < end; i++) {
    const ev = e.cast.events[i];
    if (!ev) continue;
    if (ev.kind === 'o') out += ev.data;
    else {
      const size = parseSize(ev.data);
      if (out) e.term.write(out);
      out = '';
      if (size) e.term.resize(size.cols, size.rows);
    }
  }
  if (out) e.term.write(out);
  e.idx = end;
  e.pos = to;
}

function seekTo(e: Engine, to: number) {
  const t = Math.max(0, Math.min(to, e.cast.duration));
  if (t < e.pos) {
    e.term.reset();
    e.term.resize(e.cast.header.width, e.cast.header.height);
    e.idx = 0;
    e.pos = 0;
  }
  playTo(e, t);
}

export default function Replay({ path }: { path: string }) {
  const t = useT();
  const host = useRef<HTMLDivElement>(null);
  const engine = useRef<Engine | null>(null);
  const [cast, setCast] = useState<Cast | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [clock, setClock] = useState(0);
  const close = () => useUi.getState().closeReplay();

  useEffect(() => {
    let live = true;
    api
      .recordingRead(path)
      .then((text) => {
        if (!live) return;
        const c = parseCast(text);
        if (c) setCast(c);
        else setFailed(t('replay.invalid'));
      })
      .catch((e: unknown) => live && setFailed(describeError(e, t).message));
    return () => {
      live = false;
    };
  }, [path, t]);

  // The terminal, once the recording is loaded.
  useEffect(() => {
    if (!cast || !host.current) return;
    const term = new Terminal({
      allowProposedApi: true,
      cols: cast.header.width,
      rows: cast.header.height,
      scrollback: 10_000,
      disableStdin: true,
      fontFamily: readMonoFont(),
      fontSize: 12,
      lineHeight: readLineHeight(),
      theme: readTerminalTheme(),
      cursorInactiveStyle: 'none',
    });
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = '11';
    term.open(host.current);
    engine.current = { term, cast, pos: 0, idx: 0 };
    return () => {
      engine.current = null;
      term.dispose();
    };
  }, [cast]);

  // Playback clock.
  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    let last = performance.now();
    let shown = 0;
    const tick = (now: number) => {
      const e = engine.current;
      if (!e) return;
      const next = Math.min(e.pos + ((now - last) / 1000) * speed, e.cast.duration);
      last = now;
      playTo(e, next);
      if (now - shown > 100 || next >= e.cast.duration) {
        shown = now;
        setClock(next);
      }
      if (next >= e.cast.duration) setPlaying(false);
      else frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing, speed]);

  const seek = useCallback((to: number) => {
    const e = engine.current;
    if (!e) return;
    seekTo(e, to);
    setClock(e.pos);
  }, []);

  const toggle = useCallback(() => {
    const e = engine.current;
    if (!e) return;
    // At the end, play starts over.
    if (!playing && e.pos >= e.cast.duration) seek(0);
    setPlaying((p) => !p);
  }, [playing, seek]);

  const save = async (format: 'txt' | 'html') => {
    const e = engine.current;
    if (!e) return;
    const name =
      path
        .split(/[\\/]/)
        .pop()
        ?.replace(/\.cast$/, '') ?? 'recording';
    const dest = await api.pickPath('save', [format], `${name}.${format}`).catch(() => null);
    if (!dest) return;
    const b = e.term.buffer.active;
    const text =
      format === 'txt'
        ? bufferToText(b)
        : bufferToHtml(b, readTerminalTheme(), e.cast.header.title ?? name, readMonoFont());
    try {
      await api.textExport(dest, text);
      useUi.getState().flash(t('status.exported', { path: dest }));
    } catch (err) {
      const d = describeError(err, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    }
  };

  const onKey = (ev: React.KeyboardEvent) => {
    if (ev.target instanceof HTMLSelectElement) return;
    if (ev.key === ' ') {
      ev.preventDefault();
      toggle();
    } else if (ev.key === 'ArrowLeft' || ev.key === 'ArrowRight') {
      ev.preventDefault();
      const e = engine.current;
      if (e) seek(e.pos + (ev.key === 'ArrowLeft' ? -SEEK_STEP : SEEK_STEP));
    }
  };

  const duration = cast?.duration ?? 0;
  const title = cast?.header.title
    ? `${t('replay.title')} · ${cast.header.title}`
    : t('replay.title');

  return (
    <Dialog title={title} onClose={close} width="min(1100px, 94vw)">
      <div className="flex flex-col gap-2 p-3" onKeyDown={onKey}>
        {failed ? (
          <p className="tone-text text-sm" data-tone="error">
            {failed}
          </p>
        ) : (
          <div className="max-h-[65vh] overflow-auto rounded-sm bg-terminal p-2">
            <div ref={host} />
          </div>
        )}
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <button
            type="button"
            className="btn btn-primary h-7"
            disabled={!cast}
            data-autofocus
            aria-label={playing ? t('replay.pause') : t('replay.play')}
            onClick={toggle}
          >
            {playing ? <Pause size={13} aria-hidden /> : <Play size={13} aria-hidden />}
            {playing ? t('replay.pause') : t('replay.play')}
          </button>
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            disabled={!cast}
            aria-label={t('replay.restart')}
            title={t('replay.restart')}
            onClick={() => seek(0)}
          >
            <RotateCcw size={13} />
          </button>
          <input
            type="range"
            className="min-w-40 flex-1"
            min={0}
            max={duration}
            step={0.1}
            value={clock}
            disabled={!cast}
            aria-label={t('replay.seek')}
            onChange={(e) => seek(Number(e.target.value))}
          />
          <span className="mono text-muted">
            {formatClock(clock)} / {formatClock(duration)}
          </span>
          <label className="flex items-center gap-1 text-secondary">
            {t('replay.speed')}
            <select
              className="field h-6 text-xs"
              value={speed}
              onChange={(e) => setSpeed(Number(e.target.value))}
            >
              {SPEEDS.map((s) => (
                <option key={s} value={s}>
                  {s}×
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            className="btn btn-ghost h-7"
            disabled={!cast}
            onClick={() => void save('txt')}
          >
            {t('replay.exportText')}
          </button>
          <button
            type="button"
            className="btn btn-ghost h-7"
            disabled={!cast}
            onClick={() => void save('html')}
          >
            {t('replay.exportHtml')}
          </button>
        </div>
        <p className="text-xs text-muted">{t('replay.hint')}</p>
      </div>
    </Dialog>
  );
}
