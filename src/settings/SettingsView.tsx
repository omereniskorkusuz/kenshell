/**
 * Settings (lazy chunk). Every change applies instantly and is persisted — no Save/OK.
 */
import { HIGHLIGHT_CATEGORIES } from '../terminal/highlight';
import { useEffect, useId, useState, type ReactNode, type FormEvent } from 'react';
import { Dialog } from '../components/Dialog';
import ColorsSection from './ColorsSection';
import KeysSection from './KeysSection';
import { api, type ImportReport, type ImportSource, type ImportSources } from '../lib/ipc';
import { describeError } from '../lib/errors';
import { toast } from '../stores/toasts';
import { useLibrary } from '../stores/library';
import { useT, type TKey } from '../i18n';
import { LIMITS, useSettings, type AppSettings, type SettingKey } from '../stores/settings';
import { useUi } from '../stores/ui';

type SectionId =
  | 'appearance'
  | 'language'
  | 'terminal'
  | 'connection'
  | 'credentials'
  | 'keys'
  | 'logging'
  | 'console'
  | 'data';
const SECTIONS: { id: SectionId; label: TKey }[] = [
  { id: 'appearance', label: 'settings.appearance' },
  { id: 'language', label: 'settings.language' },
  { id: 'terminal', label: 'settings.terminal' },
  { id: 'connection', label: 'settings.connection' },
  { id: 'credentials', label: 'settings.credentials' },
  { id: 'keys', label: 'settings.keys' },
  { id: 'logging', label: 'settings.logging' },
  { id: 'console', label: 'settings.console' },
  { id: 'data', label: 'settings.data' },
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
          {section === 'credentials' && <CredentialsSection />}
          {section === 'keys' && <KeysSection />}
          {section === 'logging' && <LoggingSection />}
          {section === 'console' && <ConsoleSection />}
          {section === 'data' && <DataSection />}
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
      <ColorsSection />
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
  const [font, setFont] = useSetting('terminal.fontFamily');
  const [lineHeight, setLineHeight] = useSetting('terminal.lineHeight');
  const [cursorStyle, setCursorStyle] = useSetting('terminal.cursorStyle');
  const [cursorBlink, setCursorBlink] = useSetting('terminal.cursorBlink');
  const [highlight, setHighlight] = useSetting('terminal.highlight');
  const [autocomplete, setAutocomplete] = useSetting('terminal.autocomplete');
  const [autoMore, setAutoMore] = useSetting('terminal.autoMore');
  const [highlightOff, setHighlightOff] = useSetting('terminal.highlightOff');
  const [fontText, setFontText] = useState(font);
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
      <Row label={t('settings.fontFamily')} hint={t('settings.fontFamilyHint')}>
        {(id) => (
          <input
            id={id}
            className="field mono w-64"
            value={fontText}
            placeholder="JetBrains Mono"
            spellCheck={false}
            onChange={(e) => setFontText(e.target.value)}
            onBlur={() => setFont(fontText)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') setFont(fontText);
            }}
          />
        )}
      </Row>
      <Row label={t('settings.lineHeight')} hint={t('settings.rangeHint', LIMITS.lineHeight)}>
        {(id) => (
          <select
            id={id}
            className="field w-48"
            value={lineHeight}
            onChange={(e) => setLineHeight(Number(e.target.value))}
          >
            {[1, 1.1, 1.2, 1.25, 1.3, 1.4, 1.5, 1.6].map((v) => (
              <option key={v} value={v}>
                {v.toFixed(2)}
              </option>
            ))}
          </select>
        )}
      </Row>
      <Row label={t('settings.cursorStyle')}>
        {(id) => (
          <select
            id={id}
            className="field w-48"
            value={cursorStyle}
            onChange={(e) => setCursorStyle(e.target.value as typeof cursorStyle)}
          >
            <option value="block">{t('settings.cursorBlock')}</option>
            <option value="bar">{t('settings.cursorBar')}</option>
            <option value="underline">{t('settings.cursorUnderline')}</option>
          </select>
        )}
      </Row>
      <Row label={t('settings.cursorBlink')} hint={t('settings.cursorBlinkHint')}>
        {(id) => <Toggle id={id} checked={cursorBlink} onChange={setCursorBlink} />}
      </Row>
      <Row label={t('settings.highlight')} hint={t('settings.highlightHint')}>
        {(id) => <Toggle id={id} checked={highlight} onChange={setHighlight} />}
      </Row>
      {highlight && (
        <Row label={t('settings.highlightRules')}>
          {() => (
            <div
              role="group"
              aria-label={t('settings.highlightRules')}
              className="flex flex-wrap gap-x-4 gap-y-1"
            >
              {HIGHLIGHT_CATEGORIES.map((c) => (
                <label key={c} className="flex items-center gap-1.5">
                  <input
                    type="checkbox"
                    checked={!highlightOff.includes(c)}
                    onChange={(e) =>
                      setHighlightOff(
                        e.target.checked
                          ? highlightOff.filter((x) => x !== c)
                          : [...highlightOff, c],
                      )
                    }
                  />
                  {t(`settings.highlightCat.${c}`)}
                </label>
              ))}
            </div>
          )}
        </Row>
      )}
      <Row label={t('settings.autocomplete')} hint={t('settings.autocompleteHint')}>
        {(id) => <Toggle id={id} checked={autocomplete} onChange={setAutocomplete} />}
      </Row>
      <Row label={t('settings.autoMore')} hint={t('settings.autoMoreHint')}>
        {(id) => <Toggle id={id} checked={autoMore} onChange={setAutoMore} />}
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
  const [identify, setIdentify] = useSetting('device.identify');
  return (
    <>
      <Row label={t('settings.netAutoReconnect')} hint={t('settings.netAutoReconnectHint')}>
        {(id) => <Toggle id={id} checked={reconnect} onChange={setReconnect} />}
      </Row>
      <Row label={t('settings.identify')} hint={t('settings.identifyHint')}>
        {(id) => <Toggle id={id} checked={identify} onChange={setIdentify} />}
      </Row>
    </>
  );
}

function CredentialsSection() {
  const t = useT();
  const profiles = useLibrary((s) => s.profiles);
  const [name, setName] = useState('');
  const [user, setUser] = useState('');
  const [pw, setPw] = useState('');
  const lib = useLibrary.getState;

  const add = async (e: FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    const ok = await lib().createProfile(name.trim(), user.trim() || null, pw || null);
    setPw('');
    if (ok) {
      setName('');
      setUser('');
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-muted">{t('credentials.profilesHint')}</p>
      {profiles.length === 0 ? (
        <p className="text-sm text-secondary">{t('credentials.none')}</p>
      ) : (
        <ul className="flex flex-col divide-y divide-[var(--border-subtle)] rounded-md border border-subtle">
          {profiles.map((p) => (
            <ProfileRow
              key={p.id}
              id={p.id}
              name={p.name}
              username={p.username}
              hasPassword={p.hasPassword}
            />
          ))}
        </ul>
      )}
      <form className="flex flex-wrap items-end gap-2" onSubmit={(e) => void add(e)}>
        <label className="flex flex-col gap-1 text-xs text-muted">
          {t('credentials.name')}
          <input className="field w-40" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="flex flex-col gap-1 text-xs text-muted">
          {t('editor.username')}
          <input
            className="field mono w-36"
            value={user}
            autoComplete="off"
            spellCheck={false}
            onChange={(e) => setUser(e.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs text-muted">
          {t('credentials.password')}
          <input
            type="password"
            className="field w-36"
            value={pw}
            autoComplete="new-password"
            onChange={(e) => setPw(e.target.value)}
          />
        </label>
        <button type="submit" className="btn btn-primary" disabled={!name.trim()}>
          {t('credentials.add')}
        </button>
      </form>
    </div>
  );
}

function ProfileRow(p: {
  id: number;
  name: string;
  username: string | null;
  hasPassword: boolean;
}) {
  const t = useT();
  const [pw, setPw] = useState('');
  const lib = useLibrary.getState;
  return (
    <li className="flex flex-wrap items-center gap-2 px-3 py-2 text-sm">
      <span className="min-w-28 font-medium">{p.name}</span>
      <span className="mono min-w-20 text-secondary">{p.username ?? '—'}</span>
      <span className="text-xs text-muted">
        {p.hasPassword ? t('credentials.passwordSavedShort') : t('credentials.noPassword')}
      </span>
      <input
        type="password"
        className="field ml-auto w-32"
        placeholder={t('credentials.newPassword')}
        aria-label={t('credentials.newPassword')}
        autoComplete="new-password"
        value={pw}
        onChange={(e) => setPw(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && pw) {
            e.preventDefault();
            void lib()
              .setProfilePassword(p.id, pw)
              .then(() => setPw(''));
          }
        }}
      />
      {p.hasPassword && (
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => void lib().setProfilePassword(p.id, null)}
        >
          {t('credentials.forget')}
        </button>
      )}
      <button type="button" className="btn btn-sm" onClick={() => void lib().deleteProfile(p.id)}>
        {t('credentials.delete')}
      </button>
    </li>
  );
}

function DataSection() {
  const t = useT();
  const [sources, setSources] = useState<ImportSources | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    void api
      .importDetect()
      .then((s) => live && setSources(s))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const report = (r: ImportReport) => {
    const parts = [t('data.created', { count: r.created })];
    if (r.skipped) parts.push(t('data.skipped', { count: r.skipped }));
    if (r.errors.length) parts.push(t('data.failed', { count: r.errors.length }));
    toast({
      kind: r.errors.length && !r.created ? 'error' : 'info',
      message: r.folder ? `${parts.join(' · ')} — ${r.folder}` : parts.join(' · '),
      detail: r.errors.slice(0, 20).join('\n') || null,
    });
  };

  const run = async (source: ImportSource, path: string | null) => {
    setBusy(true);
    try {
      report(await api.importSessions(source, path));
      await useLibrary.getState().load();
    } catch (e) {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    } finally {
      setBusy(false);
    }
  };

  const pickAndRun = async (source: ImportSource, kind: 'file' | 'folder', ext?: string[]) => {
    const path = await api.pickPath(kind, ext).catch(() => null);
    if (path) await run(source, path);
  };

  const exportAs = async (format: 'json' | 'csv') => {
    const date = new Date().toISOString().slice(0, 10);
    const path = await api
      .pickPath('save', [format], `kenshell-sessions-${date}.${format}`)
      .catch(() => null);
    if (!path) return;
    try {
      await api.exportSessions(path, format);
      useUi.getState().flash(t('data.exported', { path }));
    } catch (e) {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <h3 className="label-caps">{t('data.import')}</h3>
      <p className="text-sm text-muted">{t('data.importHint')}</p>
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          className="btn"
          disabled={busy || !sources?.puttyCount}
          onClick={() => void run('putty', null)}
        >
          {t('data.putty', { count: sources?.puttyCount ?? 0 })}
        </button>
        <button
          type="button"
          className="btn"
          disabled={busy}
          onClick={() =>
            sources?.secureCrtDir
              ? void run('securecrt', sources.secureCrtDir)
              : void pickAndRun('securecrt', 'folder')
          }
          title={sources?.secureCrtDir ?? t('data.pickFolder')}
        >
          SecureCRT{sources?.secureCrtDir ? '' : '…'}
        </button>
        <button
          type="button"
          className="btn"
          disabled={busy}
          onClick={() =>
            sources?.mobaxtermIni
              ? void run('mobaxterm', sources.mobaxtermIni)
              : void pickAndRun('mobaxterm', 'file', ['ini'])
          }
          title={sources?.mobaxtermIni ?? t('data.pickFile')}
        >
          MobaXterm{sources?.mobaxtermIni ? '' : '…'}
        </button>
        <button
          type="button"
          className="btn"
          disabled={busy}
          onClick={() => void pickAndRun('csv', 'file', ['csv', 'txt'])}
        >
          CSV…
        </button>
      </div>
      <h3 className="label-caps mt-2">{t('data.export')}</h3>
      <p className="text-sm text-muted">{t('data.exportHint')}</p>
      <div className="flex flex-wrap gap-2">
        <button type="button" className="btn" onClick={() => void exportAs('json')}>
          JSON…
        </button>
        <button type="button" className="btn" onClick={() => void exportAs('csv')}>
          CSV…
        </button>
      </div>
      <VaultBackup />
    </div>
  );
}

const MIN_MASTER = 8;

/** Encrypted backup (P2-T15): the master password never leaves this form except to the backend. */
function VaultBackup() {
  const t = useT();
  const [pw, setPw] = useState('');
  const [repeat, setRepeat] = useState('');
  const [busy, setBusy] = useState(false);
  const pwId = useId();
  const repeatId = useId();
  const hintId = useId();
  const short = pw.length > 0 && pw.length < MIN_MASTER;
  const mismatch = repeat.length > 0 && repeat !== pw;
  const canExport = pw.length >= MIN_MASTER && repeat === pw && !busy;

  const fail = (e: unknown) => {
    const d = describeError(e, t);
    toast({ kind: 'error', message: d.message, detail: d.detail });
  };

  const backup = async () => {
    const date = new Date().toISOString().slice(0, 10);
    const path = await api
      .pickPath('save', ['ksvault'], `kenshell-backup-${date}.ksvault`)
      .catch(() => null);
    if (!path) return;
    setBusy(true);
    try {
      await api.vaultExport(path, pw);
      setPw('');
      setRepeat('');
      useUi.getState().flash(t('data.vaultSaved', { path }));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const restore = async () => {
    const path = await api.pickPath('file', ['ksvault']).catch(() => null);
    if (!path) return;
    setBusy(true);
    try {
      const r = await api.vaultImport(path, pw);
      setPw('');
      setRepeat('');
      await useLibrary.getState().load();
      toast({
        kind: 'info',
        message: t('data.restored', {
          sessions: [
            t('data.created', { count: r.sessions.created }),
            ...(r.sessions.skipped ? [t('data.skipped', { count: r.sessions.skipped })] : []),
          ].join(' · '),
          passwords: r.passwords,
          hosts: r.knownHosts,
        }),
        detail: r.sessions.errors.slice(0, 20).join('\n') || null,
      });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <h3 className="label-caps mt-2">{t('data.vault')}</h3>
      <p id={hintId} className="text-sm text-muted">
        {t('data.vaultHint')}
      </p>
      <div className="grid max-w-md grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2">
        <label htmlFor={pwId} className="text-sm text-secondary">
          {t('data.master')}
        </label>
        <input
          id={pwId}
          type="password"
          className="field"
          autoComplete="new-password"
          value={pw}
          aria-describedby={hintId}
          aria-invalid={short}
          onChange={(e) => setPw(e.target.value)}
        />
        <label htmlFor={repeatId} className="text-sm text-secondary">
          {t('data.masterRepeat')}
        </label>
        <input
          id={repeatId}
          type="password"
          className="field"
          autoComplete="new-password"
          value={repeat}
          aria-invalid={mismatch}
          onChange={(e) => setRepeat(e.target.value)}
        />
        {(short || mismatch) && (
          <p className="col-start-2 text-xs text-danger" role="status">
            {short ? t('data.masterShort', { min: MIN_MASTER }) : t('data.masterMismatch')}
          </p>
        )}
      </div>
      <div className="flex flex-wrap gap-2">
        <button type="button" className="btn" disabled={!canExport} onClick={() => void backup()}>
          {t('data.vaultExport')}
        </button>
        <button
          type="button"
          className="btn"
          disabled={busy || pw.length === 0}
          onClick={() => void restore()}
        >
          {t('data.vaultImport')}
        </button>
      </div>
    </>
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
          <div className="flex gap-2">
            <input
              id={id}
              key={dir}
              className="field mono w-64"
              defaultValue={dir}
              placeholder="~/Documents/KenShell Logs"
              spellCheck={false}
              onBlur={(e) => setDir(e.target.value)}
            />
            <button
              type="button"
              className="btn"
              onClick={() =>
                void api
                  .pickPath('folder')
                  .then((p) => p && setDir(p))
                  .catch(() => {})
              }
            >
              {t('settings.choose')}
            </button>
          </div>
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
