/**
 * Side panel → Tools → AI assistant (P5-T08, only listed when turned on). Take the
 * terminal selection or the last command's output, read the masked preview, ask,
 * Send. The answer streams in; suggested commands can be copied, never sent to a device.
 */
import { Copy, Send, Square, X } from 'lucide-react';
import { useT, type TKey } from '../i18n';
import { api } from '../lib/ipc';
import { Markdown } from '../lib/markdown';
import { useAi } from '../stores/ai';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { getTerminal } from '../terminal/registry';

const PRESETS = ['explain', 'error', 'suggest'] as const;

/** Fills the assistant from the active terminal; false when there is nothing to take. */
export function takeFromTerminal(what: 'selection' | 'output', question?: string): boolean {
  const key = useTabs.getState().activeKey;
  const term = key ? getTerminal(key) : undefined;
  if (!term) return false;
  const last = what === 'output' ? term.lastOutput() : null;
  const text =
    what === 'selection' ? term.selection() : last ? `${last.command}\n${last.text}` : '';
  if (!text.trim()) return false;
  useAi.getState().setContext(text, question);
  useUi.getState().openTool('ai');
  return true;
}

export function AssistantPanel() {
  const t = useT();
  const { context, masked, trimmed, question, answer, running, error } = useAi();
  const ai = useAi.getState();

  const take = (what: 'selection' | 'output') => {
    if (!takeFromTerminal(what)) useUi.getState().flash(t('ai.nothing'));
  };

  return (
    <div className="flex flex-col gap-2 p-2 text-xs">
      <div className="flex gap-1">
        <button
          type="button"
          className="btn btn-ghost h-7 flex-1"
          onClick={() => take('selection')}
        >
          {t('ai.takeSelection')}
        </button>
        <button type="button" className="btn btn-ghost h-7 flex-1" onClick={() => take('output')}>
          {t('ai.takeOutput')}
        </button>
      </div>
      {context && (
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-1 text-muted">
            <span className="flex-1">
              {t('ai.preview')}
              {masked > 0 && ` · ${t('ai.masked', { count: masked })}`}
              {trimmed && ` · ${t('ai.trimmed')}`}
            </span>
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              aria-label={t('ai.clearContext')}
              onClick={() => ai.setContext('')}
            >
              <X size={12} />
            </button>
          </div>
          <pre className="mono max-h-40 overflow-auto rounded-sm border border-subtle p-1.5 text-[11px] leading-snug whitespace-pre-wrap">
            {context}
          </pre>
        </div>
      )}
      <div className="flex flex-wrap gap-1">
        {PRESETS.map((p) => (
          <button
            key={p}
            type="button"
            className="btn btn-ghost h-6 px-2 text-xs"
            onClick={() => ai.setQuestion(t(`ai.presets.${p}` as TKey))}
          >
            {t(`ai.presetLabels.${p}` as TKey)}
          </button>
        ))}
      </div>
      <textarea
        className="field min-h-16 text-xs"
        value={question}
        aria-label={t('ai.question')}
        placeholder={t('ai.question')}
        onChange={(e) => ai.setQuestion(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void ai.send();
          }
        }}
      />
      <div className="flex items-center gap-2">
        {running ? (
          <button type="button" className="btn btn-ghost h-7" onClick={() => ai.stop()}>
            <Square size={12} aria-hidden /> {t('ai.stop')}
          </button>
        ) : (
          <button
            type="button"
            className="btn btn-primary h-7"
            disabled={!question.trim()}
            onClick={() => void ai.send()}
          >
            <Send size={12} aria-hidden /> {t('ai.send')}
          </button>
        )}
        <span className="text-muted">{t('ai.privacy')}</span>
      </div>
      {error && (
        <p className="tone-text" data-tone="error" role="alert">
          {t(`ai.errors.${error.kind}` as TKey)}
        </p>
      )}
      {answer && (
        <div className="flex flex-col gap-1 rounded-sm border border-subtle p-2" aria-live="polite">
          <div className="text-sm leading-relaxed">
            <Markdown text={answer} />
          </div>
          {!running && (
            <button
              type="button"
              className="btn btn-ghost h-6 self-end px-2"
              onClick={() => void api.clipboardWrite(answer).catch(() => {})}
            >
              <Copy size={12} aria-hidden /> {t('ai.copy')}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
