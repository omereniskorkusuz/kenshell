/**
 * Autobaud progress → identity bar status text (pure, translator injected).
 *   running:    "Detecting speed… 9600 ✗ 19200…"
 *   found:      "Detecting speed… 9600 ✗ 115200 ✓"
 *   suggestion: "Best guess 38400" (+ a "change" control rendered by the caller)
 *   failed:     "Speed not detected"
 */
import type { TKey, TParams } from '../i18n';
import type { BaudTrial } from './ipc';

export type Translate = (key: TKey, params?: TParams) => string;

export interface AutobaudProgress {
  trials: BaudTrial[];
  current: number | null;
  result: number | null;
  suggestion: number | null;
}

export type StepStatus = 'fail' | 'ok' | 'trying';

export interface AutobaudStep {
  baud: number;
  status: StepStatus;
}

/** Visible steps (most recent last). Older steps collapse into a leading "…". */
export const MAX_VISIBLE_STEPS = 4;

export function autobaudSteps(p: AutobaudProgress): AutobaudStep[] {
  const steps: AutobaudStep[] = [];
  const seen = new Set<number>();
  for (const trial of p.trials) {
    // A baud may be re-tried (second pass); show only its latest outcome.
    if (seen.has(trial.baud)) {
      const i = steps.findIndex((s) => s.baud === trial.baud);
      if (i >= 0) steps.splice(i, 1);
    }
    seen.add(trial.baud);
    steps.push({ baud: trial.baud, status: trial.baud === p.result ? 'ok' : 'fail' });
  }
  if (p.result !== null && !seen.has(p.result)) steps.push({ baud: p.result, status: 'ok' });
  if (p.result === null && p.current !== null && !seen.has(p.current)) {
    steps.push({ baud: p.current, status: 'trying' });
  }
  return steps;
}

const MARK: Record<StepStatus, string> = { fail: ' ✗', ok: ' ✓', trying: '…' };

export function formatSteps(steps: readonly AutobaudStep[], max = MAX_VISIBLE_STEPS): string {
  const visible = steps.length > max ? steps.slice(steps.length - max) : steps;
  const text = visible.map((s) => `${s.baud}${MARK[s.status]}`).join(' ');
  return steps.length > max ? `… ${text}` : text;
}

export type AutobaudPhase = 'running' | 'found' | 'suggestion' | 'failed';

export function autobaudPhase(p: AutobaudProgress): AutobaudPhase {
  if (p.result !== null) return 'found';
  if (p.suggestion !== null) return 'suggestion';
  if (p.current !== null || p.trials.length === 0) return 'running';
  return 'failed';
}

export interface AutobaudStatus {
  phase: AutobaudPhase;
  text: string;
  /** Baud offered with a one-click "change" control (suggestion phase only). */
  suggestion: number | null;
}

export function autobaudStatus(p: AutobaudProgress, t: Translate): AutobaudStatus {
  const phase = autobaudPhase(p);
  switch (phase) {
    case 'suggestion':
      return {
        phase,
        text: t('autobaud.bestGuess', { baud: p.suggestion ?? 0 }),
        suggestion: p.suggestion,
      };
    case 'failed':
      return { phase, text: t('autobaud.failed'), suggestion: null };
    default: {
      const steps = formatSteps(autobaudSteps(p));
      const head = t('autobaud.detecting');
      return { phase, text: steps ? `${head} ${steps}` : head, suggestion: null };
    }
  }
}
