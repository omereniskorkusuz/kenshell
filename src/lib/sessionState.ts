/**
 * Live tab state machine driven by backend `SessionEvent`s (pure; tested):
 *   connecting → detecting (autobaud) → connected → disconnected (error / cable removed)
 * plus derived, translated status text for the identity bar, tab and announcements.
 */
import { autobaudPhase, autobaudStatus, type AutobaudProgress, type Translate } from './autobaud';
import { describeError } from './errors';
import { framingLabel, shortPortName } from './format';
import type {
  AppError,
  ConnState,
  HealthKind,
  RuntimeId,
  SerialLink,
  SerialSettings,
  SessionEvent,
} from './ipc';

/** CLI mode. Detected in Phase 3; Phase 1 always shows `unknown` (neutral chip "—"). */
export type CliMode = 'unknown' | 'user' | 'privileged' | 'config' | 'boot' | 'shell';

export interface LiveTab {
  /** Local, stable tab id (independent of the backend runtime id). */
  key: string;
  runtimeId: RuntimeId | null;
  /** Bumped on every (re)open; events from older opens are ignored. */
  generation: number;
  sessionId: number | null;
  name: string;
  envTag: string | null;
  settings: SerialSettings;
  state: ConnState;
  error: AppError | null;
  autobaud: AutobaudProgress | null;
  link: SerialLink | null;
  health: HealthKind | null;
  connectedAt: number | null;
  mode: CliMode;
}

export function applySessionEvent(tab: LiveTab, ev: SessionEvent, now: number): LiveTab {
  switch (ev.type) {
    case 'state':
      switch (ev.state) {
        case 'connecting':
          return {
            ...tab,
            state: 'connecting',
            error: null,
            autobaud: null,
            connectedAt: null,
            // "Cable back, reconnecting…" stays until connected.
            health: tab.health === 'cableReturned' ? 'cableReturned' : null,
          };
        case 'detecting':
          return { ...tab, state: 'detecting', error: null, connectedAt: null };
        case 'connected':
          return {
            ...tab,
            state: 'connected',
            error: null,
            health: null,
            connectedAt:
              tab.state === 'connected' && tab.connectedAt !== null ? tab.connectedAt : now,
          };
        case 'disconnected':
          return {
            ...tab,
            state: 'disconnected',
            error: ev.error,
            connectedAt: null,
            health: tab.health === 'cableRemoved' ? 'cableRemoved' : null,
          };
      }
      return tab;
    case 'autobaud':
      return {
        ...tab,
        autobaud: {
          trials: ev.trials,
          current: ev.current,
          result: ev.result,
          suggestion: ev.suggestion,
        },
      };
    case 'link':
      return { ...tab, link: ev.link };
    case 'health':
      if (ev.kind === 'noData' && tab.state !== 'connected') return tab;
      return { ...tab, health: ev.kind };
  }
}

export type StatusTone = 'idle' | 'busy' | 'ok' | 'warn' | 'error';

export interface TabStatus {
  /** Short status word/sentence (always text; color is never the only carrier). */
  text: string;
  tone: StatusTone;
  /** Technical detail for a tooltip. */
  detail: string | null;
  /** Autobaud best guess offered with a one-click change. */
  suggestion: number | null;
}

export function portLabel(tab: LiveTab): string {
  return tab.link?.displayName || shortPortName(tab.settings.port);
}

export function tabStatus(tab: LiveTab, t: Translate): TabStatus {
  const port = portLabel(tab);
  const none = { detail: null, suggestion: null };
  switch (tab.state) {
    case 'connecting':
      return tab.health === 'cableReturned'
        ? { text: t('health.cableReturned'), tone: 'busy', ...none }
        : { text: t('status.connecting'), tone: 'busy', ...none };
    case 'detecting': {
      if (!tab.autobaud) return { text: t('autobaud.detecting'), tone: 'busy', ...none };
      const ab = autobaudStatus(tab.autobaud, t);
      return {
        text: ab.text,
        tone: ab.phase === 'suggestion' ? 'warn' : ab.phase === 'failed' ? 'error' : 'busy',
        detail: null,
        suggestion: ab.suggestion,
      };
    }
    case 'connected':
      if (tab.health === 'noData') return { text: t('health.noData'), tone: 'warn', ...none };
      if (tab.autobaud && autobaudPhase(tab.autobaud) === 'suggestion') {
        const ab = autobaudStatus(tab.autobaud, t);
        return { text: ab.text, tone: 'warn', detail: null, suggestion: ab.suggestion };
      }
      return { text: t('status.connected'), tone: 'ok', ...none };
    case 'disconnected': {
      if (tab.health === 'cableRemoved') {
        return { text: t('health.cableRemoved', { port }), tone: 'error', ...none };
      }
      if (tab.error) {
        const d = describeError(tab.error, t, { port });
        return { text: d.message, tone: 'error', detail: d.detail, suggestion: null };
      }
      return { text: t('status.disconnected'), tone: 'idle', ...none };
    }
  }
}

/** "COM4 @ 9600 8N1 · FTDI FT232R" (link known) or "COM4 @ Auto 8N1" (requested). */
export function targetLabel(tab: LiveTab, t: Translate, withChip = true): string {
  const s = tab.settings;
  if (tab.link) {
    const l = tab.link;
    const chip = withChip && l.chip ? ` · ${l.chip}` : '';
    return `${l.displayName} @ ${l.baud} ${l.framing}${chip}`;
  }
  const baud = s.baud === null ? t('serial.auto') : String(s.baud);
  return `${portLabel(tab)} @ ${baud} ${framingLabel(s.dataBits, s.parity, s.stopBits)}`;
}
