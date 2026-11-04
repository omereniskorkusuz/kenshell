/**
 * Settings (lazy chunk). Every change applies instantly and is persisted — no Save/OK.
 */
import { useId, useState, type ReactNode } from 'react';
import { Dialog } from '../components/Dialog';
import { useT, type TKey } from '../i18n';
import { LIMITS, useSettings, type AppSettings, type SettingKey } from '../stores/settings';
import { useUi } from '../stores/ui';

type SectionId = 'appearance' | 'language' | 'terminal' | 'connection' | 'logging' | 'console';
const SECTIONS: { id: SectionId; label: TKey }[] = [
  { id: 'appearance', label: 'settings.appearance' },
  { id: 'language', label: 'settings.language' },
  { id: 'terminal', label: 'settings.terminal' },
  { id: 'connection', label: 'settings.connection' },
  { id: 'logging', label: 'settings.logging' },
  { id: 'console', label: 'settings.console' },
];

export default function SettingsView() {
  const t = useT();
  const [section, setSection] = useState<SectionId>('appearance');

  return (
    <Dialog
      title={t('shell.settings')}
      onClose={() => useUi.getState().setSettingsOpen(false)}
      width="640px"
    >
      <div className="flex min-h-[340px]">
        <nav
          aria-label={t('shell.settings')}
          className="flex w-40 shrink-0 flex-col gap-0.5 border-r border-subtle p-2"
        >
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              type="button"
              aria-current={section === s.id ? 'page' : undefined}
              className={`h-7 rounded-sm px-2 text-left text-sm ${
                section === s.id ? 'bg-selected text-primary' : 'text-secondary hover:bg-hover'
              }`}
              onClick={() => setSection(s.id)}
            >
              {t(s.label)}
            </button>
          ))}
        </nav>
        <div className="flex min-w-0 flex-1 flex-col gap-5 p-4">
          {section === 'appearance' && <AppearanceSection />}
          {section === 'language' && <LanguageSection />}
          {section === 'terminal' && <TerminalSection />}
          {section === 'connection' && <ConnectionSection />}
          {section === 'logging' && <LoggingSection />}
          {section === 'console' && <ConsoleSection />}
          <p className="mt-auto text-xs text-muted">{t('settings.instantHint')}</p>
        </div>
      </div>
    </Dialog>
  );
}

function useSetting<K extends SettingKey>(key: K): [AppSettings[K], (v: AppSettings[K]) => void] {
  const value = useSettings((s) => s.values[key]);
  return [value, (v) => useSettings.getState().set(key, v)];
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: (id: string) => ReactNode;
}) {
  const id = useId();
  return (
    <div className="flex items-center gap-4">
      <div className="min-w-0 flex-1">
        <label htmlFor={id} className="text-sm text-primary">
          {label}
        </label>
        {hint && <p className="text-xs text-muted">{hint}</p>}
      </div>
      <div className="shrink-0">{children(id)}</div>
    </div>
  );
}

function Segmented<T extends string | number>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex gap-0.5 rounded-md bg-app p-0.5">
      {options.map((o) => (
        <button
          key={String(o.value)}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          className={`h-6 rounded-sm px-3 text-xs ${
            o.value === value ? 'bg-selected text-primary' : 'text-secondary hover:text-primary'
          }`}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function Toggle({
  id,
  checked,
  onChange,
}: {
  id: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <input
      id={id}
      type="checkbox"
      role="switch"
      className="h-4 w-4 accent-[var(--accent)]"
      checked={checked}
      onChange={(e) => onChange(e.target.checked)}
    />
  );
}

function AppearanceSection() {
  const t = useT();
  const [theme, setTheme] = useSetting('appearance.theme');
  const [stripe, setStripe] = useSetting('appearance.modeStripeWidth');
  return (
    <>
      <Row label={t('settings.theme')}>
        {() => (
          <Segmented
            label={t('settings.theme')}
            value={theme}
            onChange={setTheme}
            options={[
              { value: 'dark', label: t('settings.themeDark') },
              { value: 'light', label: t('settings.themeLight') },
              { value: 'system', label: t('settings.themeSystem') },
            ]}
          />
        )}
      </Row>
      <Row label={t('settings.stripeWidth')} hint={t('settings.stripeHint')}>
        {() => (
          <Segmented
            label={t('settings.stripeWidth')}
            value={stripe}
            onChange={setStripe}
            options={[
              { value: 4, label: '4 px' },
              { value: 8, label: '8 px' },
            ]}
          />
        )}
      </Row>
    </>
  );
}

function LanguageSection() {
  const t = useT();
  const [lang, setLang] = useSetting('general.language');
  return (
    <Row label={t('settings.language')} hint={t('settings.languageHint')}>
      {() => (
        <Segmented
          label={t('settings.language')}
          value={lang}
          onChange={setLang}
          options={[
            { value: 'system', label: t('settings.languageSystem') },
            { value: 'tr', label: 'Türkçe' },
            { value: 'en', label: 'English' },
          ]}
        />
      )}
    </Row>
  );
}

function NumberField({
  id,
  value,
  min,
  max,
  step,
  onCommit,
}: {
  id: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onCommit: (v: number) => void;
}) {
  const [text, setText] = useState(String(value));
  const [prev, setPrev] = useState(value);
  if (prev !== value) {
    // External change (e.g. clamped): show the stored value.
    setPrev(value);
    setText(String(value));
  }
  const commit = () => {
    const n = Number(text);
    if (text.trim() === '' || !Number.isFinite(n)) setText(String(value));
    else onCommit(Math.min(max, Math.max(min, Math.round(n))));
  };
  return (
    <input
      id={id}
      type="number"
      className="field mono w-28 text-right"
      min={min}
      max={max}
      step={step}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === 'Enter') commit();
      }}
    />
  );
}

function TerminalSection() {
  const t = useT();
  const [fontSize, setFontSize] = useSetting('terminal.fontSize');
  const [scrollback, setScrollback] = useSetting('terminal.scrollback');
  const [copySel, setCopySel] = useSetting('terminal.copyOnSelect');
  const [rcPaste, setRcPaste] = useSetting('terminal.rightClickPaste');
  const [backspace, setBackspace] = useSetting('terminal.backspace');
  const [charset, setCharset] = useSetting('terminal.charset');
  const [stamps, setStamps] = useSetting('terminal.timestamps');
  const [pasteDelay, setPasteDelay] = useSetting('terminal.pasteDelayMs');
  return (
    <>
      <Row label={t('settings.fontSize')} hint={t('settings.rangeHint', LIMITS.fontSize)}>
        {(id) => (
          <NumberField
            id={id}
            value={fontSize}
            step={1}
            {...LIMITS.fontSize}
            onCommit={setFontSize}
          />
        )}
      </Row>
      <Row label={t('settings.scrollback')} hint={t('settings.rangeHint', LIMITS.scrollback)}>
        {(id) => (
          <NumberField
            id={id}
            value={scrollback}
            step={1000}
            {...LIMITS.scrollback}
            onCommit={setScrollback}
          />
        )}
      </Row>
      <Row label={t('settings.copyOnSelect')}>
        {(id) => <Toggle id={id} checked={copySel} onChange={setCopySel} />}
      </Row>
      <Row label={t('settings.rightClickPaste')}>
        {(id) => <Toggle id={id} checked={rcPaste} onChange={setRcPaste} />}
      </Row>
      <Row label={t('settings.pasteDelay')} hint={t('settings.pasteDelayHint')}>
        {(id) => (
          <NumberField
            id={id}
            value={pasteDelay}
            step={10}
            min={0}
            max={1000}
            onCommit={setPasteDelay}
          />
        )}
      </Row>
      <Row label={t('settings.backspace')}>
        {(id) => (
          <select
            id={id}
            className="field w-48"
            value={backspace}
            onChange={(e) => setBackspace(e.target.value as 'del' | 'ctrlH')}
          >
            <option value="del">DEL (0x7F)</option>
            <option value="ctrlH">Ctrl-H (0x08)</option>
          </select>
        )}
      </Row>
      <Row label={t('settings.charset')} hint={t('settings.charsetHint')}>
        {(id) => (
          <select
            id={id}
            className="field w-48"
            value={charset}
            onChange={(e) => setCharset(e.target.value as typeof charset)}
          >
            <option value="utf-8">UTF-8</option>
            <option value="iso-8859-9">ISO-8859-9 (Latin-5)</option>
            <option value="windows-1254">Windows-1254</option>
            <option value="cp437">CP437</option>
          </select>
        )}
      </Row>
      <Row label={t('settings.lineTimestamps')} hint={t('settings.lineTimestampsHint')}>
        {(id) => <Toggle id={id} checked={stamps} onChange={setStamps} />}
      </Row>
    </>
  );
}

function ConnectionSection() {
  const t = useT();
  const [reconnect, setReconnect] = useSetting('connection.autoReconnect');
  return (
    <Row label={t('settings.netAutoReconnect')} hint={t('settings.netAutoReconnectHint')}>
      {(id) => <Toggle id={id} checked={reconnect} onChange={setReconnect} />}
    </Row>
  );
}

function LoggingSection() {
  const t = useT();
  const [enabled, setEnabled] = useSetting('log.enabled');
  const [dir, setDir] = useSetting('log.dir');
  const [template, setTemplate] = useSetting('log.template');
  const [mode, setMode] = useSetting('log.mode');
  const [stamps, setStamps] = useSetting('log.timestamps');
  return (
    <>
      <Row label={t('settings.logEnabled')} hint={t('settings.logEnabledHint')}>
        {(id) => <Toggle id={id} checked={enabled} onChange={setEnabled} />}
      </Row>
      <Row label={t('settings.logDir')} hint={t('settings.logDirHint')}>
        {(id) => (
          <input
            id={id}
            className="field mono w-72"
            defaultValue={dir}
            placeholder="~/Documents/KenShell Logs"
            spellCheck={false}
            onBlur={(e) => setDir(e.target.value)}
          />
        )}
      </Row>
      <Row label={t('settings.logTemplate')} hint={t('settings.logTemplateHint')}>
        {(id) => (
          <input
            id={id}
            className="field mono w-72"
            defaultValue={template}
            spellCheck={false}
            onBlur={(e) => setTemplate(e.target.value)}
          />
        )}
      </Row>
      <Row label={t('settings.logMode')}>
        {(id) => (
          <select
            id={id}
            className="field w-48"
            value={mode}
            onChange={(e) => setMode(e.target.value as 'printable' | 'all')}
          >
            <option value="printable">{t('settings.logModePrintable')}</option>
            <option value="all">{t('settings.logModeAll')}</option>
          </select>
        )}
      </Row>
      <Row label={t('settings.logTimestamps')} hint={t('settings.logTimestampsHint')}>
        {(id) => <Toggle id={id} checked={stamps} onChange={setStamps} />}
      </Row>
    </>
  );
}

function ConsoleSection() {
  const t = useT();
  const [toast, setToast] = useSetting('console.hotplugToast');
  const [reconnect, setReconnect] = useSetting('console.autoReconnect');
  return (
    <>
      <Row label={t('settings.hotplugToast')} hint={t('settings.hotplugToastHint')}>
        {(id) => <Toggle id={id} checked={toast} onChange={setToast} />}
      </Row>
      <Row label={t('settings.autoReconnect')} hint={t('settings.autoReconnectHint')}>
        {(id) => <Toggle id={id} checked={reconnect} onChange={setReconnect} />}
      </Row>
    </>
  );
}
