/**
 * Quick connect field (Ctrl/Cmd+L). Live preview row under the field before Enter; problems
 * are quiet helper text, never a dialog.
 */
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { CornerDownLeft } from 'lucide-react';
import { useT } from '../i18n';
import { DEFAULT_PORTS, DEFAULT_SERIAL, DEFAULT_SSH, isMac } from '../lib/ipc';
import {
  MAX_BAUD,
  MIN_BAUD,
  parseQuickConnect,
  type QuickConnect as Intent,
} from '../lib/quickConnect';
import { formatChord } from '../lib/shortcuts';
import { shortPortName } from '../lib/format';
import { useLibrary } from '../stores/library';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';

type Translate = ReturnType<typeof useT>;

function preview(intent: Intent, t: Translate): { text: string; actionable: boolean } {
  switch (intent.kind) {
    case 'empty':
      return { text: t('quick.hint'), actionable: false };
    case 'session':
      return { text: t('quick.previewSession', { name: intent.session.name }), actionable: true };
    case 'serial':
      return {
        text: t('quick.previewSerial', {
          port: shortPortName(intent.port),
          baud: intent.baud === null ? t('serial.autoBaud') : String(intent.baud),
        }),
        actionable: true,
      };
    case 'network':
      return {
        text: t('quick.previewNetwork', {
          protocol: intent.protocol.toUpperCase(),
          target: `${intent.username ? `${intent.username}@` : ''}${intent.host}:${
            intent.port ?? DEFAULT_PORTS[intent.protocol] ?? ''
          }`,
        }),
        actionable: true,
      };
    case 'invalidPort':
      return { text: t('quick.invalidPort'), actionable: false };
    case 'invalidBaud':
      return { text: t('quick.invalidBaud', { min: MIN_BAUD, max: MAX_BAUD }), actionable: false };
    case 'noMatch':
      return { text: t('quick.noMatch', { text: intent.text }), actionable: false };
  }
}

export function QuickConnect() {
  const t = useT();
  const [value, setValue] = useState('');
  const [focused, setFocused] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const previewId = useId();
  const sessions = useLibrary((s) => s.sessions);
  const focusTick = useUi((s) => s.quickConnectFocus);

  useEffect(() => {
    if (focusTick === 0) return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [focusTick]);

  const intent = useMemo(() => parseQuickConnect(value, sessions), [value, sessions]);
  const p = preview(intent, t);

  const submit = () => {
    const tabs = useTabs.getState();
    if (intent.kind === 'session') {
      const s = sessions.find((x) => x.id === intent.session.id);
      if (s) tabs.openSession(s);
    } else if (intent.kind === 'serial') {
      tabs.openSerial({ ...DEFAULT_SERIAL, port: intent.port, baud: intent.baud });
    } else if (intent.kind === 'network') {
      tabs.openTarget({
        type: 'network',
        protocol: intent.protocol,
        host: intent.host,
        port: intent.port ?? DEFAULT_PORTS[intent.protocol],
        username: intent.username,
        ssh: DEFAULT_SSH,
      });
    } else {
      return;
    }
    setValue('');
  };

  return (
    <div className="relative w-80 max-w-[36vw] min-w-48">
      <input
        ref={inputRef}
        className="field mono h-7 pr-14 text-sm"
        placeholder={t('shell.quickConnect')}
        aria-label={t('quick.label')}
        aria-describedby={previewId}
        spellCheck={false}
        autoCapitalize="off"
        autoComplete="off"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            submit();
          } else if (e.key === 'Escape') {
            e.preventDefault();
            if (value) setValue('');
            else inputRef.current?.blur();
          }
        }}
      />
      {!focused && !value && (
        <span className="kbd pointer-events-none absolute top-1/2 right-2 -translate-y-1/2">
          {formatChord('mod+l', isMac())}
        </span>
      )}
      <div
        id={previewId}
        role="status"
        className={`elevated absolute top-full right-0 left-0 mt-1 flex items-center gap-2 px-3 py-2 text-sm ${
          focused || value ? '' : 'sr-only'
        }`}
        style={{ zIndex: 'var(--z-palette)' }}
      >
        <span className={`min-w-0 flex-1 truncate ${p.actionable ? 'text-primary' : 'text-muted'}`}>
          {p.text}
        </span>
        {p.actionable && <CornerDownLeft size={14} className="shrink-0 text-muted" aria-hidden />}
      </div>
    </div>
  );
}
