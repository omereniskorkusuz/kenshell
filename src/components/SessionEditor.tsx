/** Saved session editor (a form, not a confirmation). Serial only in this version. */
import { useEffect, useId, useMemo, useState, type FormEvent, type ReactNode } from 'react';
import { useT, type TKey } from '../i18n';
import { describeError } from '../lib/errors';
import { api, DEFAULT_PORTS, isMac, type NetProtocol, type Protocol } from '../lib/ipc';
import { formatChord } from '../lib/shortcuts';
import {
  BAUD_PRESETS,
  DATA_BITS,
  draftFrom,
  draftToInput,
  ENV_CHOICES,
  hasErrors,
  MAX_BAUD,
  MAX_ENV,
  MAX_KEEPALIVE,
  MAX_NAME,
  MIN_BAUD,
  STOP_BITS,
  forwardErrors,
  validateDraft,
  type BaudChoice,
  type Draft,
  type DraftErrors,
  type EnvChoice,
  type LineChoice,
} from '../lib/sessionDraft';
import { MAX_FORWARDS } from '../lib/sshTunnel';
import { folderPath } from '../lib/tree';
import { chipLabel, useConsoles } from '../stores/consoles';
import { useLibrary } from '../stores/library';
import { useTabs } from '../stores/tabs';
import { toast } from '../stores/toasts';
import { useUi, type EditorState } from '../stores/ui';
import { Dialog } from './Dialog';

const PROTOCOLS: { id: Protocol; label: TKey }[] = [
  { id: 'serial', label: 'protocol.serial' },
  { id: 'ssh', label: 'protocol.ssh' },
  { id: 'telnet', label: 'protocol.telnet' },
  { id: 'rlogin', label: 'protocol.rlogin' },
  { id: 'raw', label: 'protocol.raw' },
];

export function SessionEditor({ state }: { state: EditorState }) {
  const t = useT();
  const sessions = useLibrary((s) => s.sessions);
  const folders = useLibrary((s) => s.folders);
  const ports = useConsoles((s) => s.snapshot.ports);
  const record =
    state.sessionId !== null ? sessions.find((s) => s.id === state.sessionId) : undefined;
  const base = record ?? state.initial;
  const [draft, setDraft] = useState<Draft>(() => draftFrom(base));
  const [touched, setTouched] = useState(false);
  const [saving, setSaving] = useState(false);
  const profiles = useLibrary((s) => s.profiles);
  // Session password: written to the OS keychain on save, never read back into the UI.
  const [password, setPassword] = useState('');
  const [hasSaved, setHasSaved] = useState(false);
  const [forget, setForget] = useState(false);
  useEffect(() => {
    if (!record) return;
    let live = true;
    void api
      .sessionPasswordExists(record.id)
      .then((v) => live && setHasSaved(v))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [record]);
  const errors = useMemo(() => validateDraft(draft), [draft]);
  const shown: DraftErrors = touched ? errors : {};
  const [advancedOpen, setAdvancedOpen] = useState(
    () =>
      draft.jump !== '' ||
      draft.proxyKind !== 'none' ||
      draft.forwards !== '' ||
      draft.keepalive !== '',
  );
  const tunnelError = Boolean(shown.jump || shown.proxy || shown.forwards || shown.keepalive);
  // Serial line control is rarely changed: open only when a session already sets it.
  const [lineOpen, setLineOpen] = useState(
    () => draft.dtr !== 'default' || draft.rts !== 'default',
  );
  const portsListId = useId();
  const mac = isMac();

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft((d) => ({ ...d, [key]: value }));
  const close = () => useUi.getState().closeEditor();

  const folderOptions = useMemo(
    () =>
      folders
        .map((f) => ({ id: f.id, label: folderPath(folders, f.id) }))
        .sort((a, b) => a.label.localeCompare(b.label)),
    [folders],
  );

  const save = async (connect: boolean) => {
    setTouched(true);
    if (hasErrors(errors) || saving) return;
    setSaving(true);
    const input = draftToInput(draft, base);
    const lib = useLibrary.getState();
    const saved = record
      ? await lib.updateSession(record.id, input)
      : await lib.createSession(input);
    if (saved && input.protocol === 'ssh' && input.credentialProfileId === null) {
      try {
        if (password) await api.sessionPasswordSet(saved.id, password);
        else if (forget) await api.sessionPasswordSet(saved.id, null);
      } catch (e) {
        const d = describeError(e, t);
        toast({ kind: 'error', message: t('credentials.saveFailed'), detail: d.detail });
      }
    }
    setPassword('');
    setSaving(false);
    if (!saved) return; // error toast already shown; keep the form open
    close();
    if (connect) useTabs.getState().openSession(saved);
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void save(true);
  };

  const errorText = (field: keyof DraftErrors): string | null => {
    const code = shown[field];
    if (!code) return null;
    if (field === 'baud') return t('editor.baudRange', { min: MIN_BAUD, max: MAX_BAUD });
    if (field === 'netPort' && code === 'range') return t('editor.portRange');
    if (field === 'jump') return t('editor.jumpFormat');
    if (field === 'proxy') return t('editor.proxyFormat');
    if (field === 'keepalive') return t('editor.keepaliveRange', { max: MAX_KEEPALIVE });
    if (field === 'forwards') {
      const lines = forwardErrors(draft) ?? [];
      return lines.length
        ? t('editor.forwardsFormat', { lines: lines.join(', ') })
        : t('editor.forwardsTooMany', { max: MAX_FORWARDS });
    }
    if (code === 'tooLong')
      return t('editor.tooLong', { max: field === 'env' ? MAX_ENV : MAX_NAME });
    return t('editor.required');
  };

  const serial = draft.protocol === 'serial';
  const formId = useId();

  return (
    <Dialog
      title={record ? t('editor.editTitle', { name: record.name }) : t('editor.newTitle')}
      onClose={close}
      footer={
        <>
          <span className="mr-auto text-xs text-muted">
            {t('editor.saveConnectHint', { shortcut: formatChord('mod+enter', mac) })}
          </span>
          <button type="button" className="btn" onClick={close}>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn" disabled={saving} onClick={() => void save(false)}>
            {t('editor.save')}
          </button>
          <button type="submit" form={formId} className="btn btn-primary" disabled={saving}>
            {t('editor.saveConnect')}
          </button>
        </>
      }
    >
      <form
        id={formId}
        noValidate
        className="grid grid-cols-[120px_1fr] items-center gap-x-3 gap-y-3 p-4"
        onSubmit={onSubmit}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void save(true);
          }
        }}
      >
        <Field label={t('editor.name')} error={errorText('name')}>
          {(id, describedBy) => (
            <input
              id={id}
              data-autofocus
              className="field mono"
              value={draft.name}
              maxLength={MAX_NAME + 10}
              aria-invalid={!!shown.name}
              aria-describedby={describedBy}
              placeholder={t('editor.namePlaceholder')}
              onChange={(e) => set('name', e.target.value)}
            />
          )}
        </Field>

        <Field label={t('editor.folder')}>
          {(id) => (
            <select
              id={id}
              className="field"
              value={draft.folderId ?? ''}
              onChange={(e) =>
                set('folderId', e.target.value === '' ? null : Number(e.target.value))
              }
            >
              <option value="">{t('library.rootFolder')}</option>
              {folderOptions.map((f) => (
                <option key={f.id} value={f.id}>
                  {f.label}
                </option>
              ))}
            </select>
          )}
        </Field>

        <Field label={t('editor.autoCommands')}>
          {(id) => (
            <div className="flex flex-col gap-1">
              <textarea
                id={id}
                className="field mono min-h-16"
                rows={3}
                value={draft.autoCommands}
                placeholder={'terminal length 0\nshow clock'}
                spellCheck={false}
                onChange={(e) => set('autoCommands', e.target.value)}
              />
              <span className="text-xs text-muted">{t('editor.autoCommandsHint')}</span>
            </div>
          )}
        </Field>

        <Field label={t('editor.tags')}>
          {(id) => (
            <input
              id={id}
              className="field"
              value={draft.tags}
              placeholder={t('editor.tagsHint')}
              spellCheck={false}
              onChange={(e) => set('tags', e.target.value)}
            />
          )}
        </Field>

        <Field label={t('editor.protocol')}>
          {(id) => (
            <select
              id={id}
              className="field"
              value={draft.protocol}
              onChange={(e) => set('protocol', e.target.value as Protocol)}
            >
              {PROTOCOLS.map((p) => (
                <option key={p.id} value={p.id}>
                  {t(p.label)}
                </option>
              ))}
            </select>
          )}
        </Field>

        {!serial && (
          <>
            <Field label={t('editor.host')} error={errorText('host')}>
              {(id, describedBy) => (
                <input
                  id={id}
                  className="field mono"
                  value={draft.host}
                  aria-invalid={!!shown.host}
                  aria-describedby={describedBy}
                  placeholder="10.0.0.1"
                  spellCheck={false}
                  autoCapitalize="off"
                  onChange={(e) => set('host', e.target.value)}
                />
              )}
            </Field>
            <Field label={t('editor.netPort')} error={errorText('netPort')}>
              {(id, describedBy) => (
                <input
                  id={id}
                  className="field mono w-32"
                  inputMode="numeric"
                  value={draft.netPort}
                  aria-invalid={!!shown.netPort}
                  aria-describedby={describedBy}
                  placeholder={String(DEFAULT_PORTS[draft.protocol as NetProtocol] ?? '')}
                  onChange={(e) => set('netPort', e.target.value.replace(/[^\d]/g, ''))}
                />
              )}
            </Field>
            {draft.protocol !== 'raw' && (
              <Field label={t('editor.username')}>
                {(id) => (
                  <input
                    id={id}
                    className="field mono"
                    value={draft.username}
                    placeholder={t('editor.usernameHint')}
                    spellCheck={false}
                    autoCapitalize="off"
                    autoComplete="off"
                    onChange={(e) => set('username', e.target.value)}
                  />
                )}
              </Field>
            )}
            {draft.protocol === 'ssh' && (
              <Field label={t('credentials.login')}>
                {(id) => (
                  <div className="flex flex-col gap-2">
                    <select
                      id={id}
                      className="field"
                      value={draft.credentialProfileId ?? ''}
                      onChange={(e) =>
                        set('credentialProfileId', e.target.value ? Number(e.target.value) : null)
                      }
                    >
                      <option value="">{t('credentials.ownLogin')}</option>
                      {profiles.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.username ? `${p.name} · ${p.username}` : p.name}
                        </option>
                      ))}
                    </select>
                    {draft.credentialProfileId === null && (
                      <div className="flex items-center gap-2">
                        <input
                          type="password"
                          className="field"
                          autoComplete="new-password"
                          aria-label={t('credentials.password')}
                          placeholder={
                            hasSaved && !forget
                              ? t('credentials.passwordSaved')
                              : t('credentials.passwordOptional')
                          }
                          value={password}
                          onChange={(e) => {
                            setPassword(e.target.value);
                            setForget(false);
                          }}
                        />
                        {hasSaved && !forget && (
                          <button
                            type="button"
                            className="btn btn-sm shrink-0"
                            onClick={() => {
                              setForget(true);
                              setPassword('');
                            }}
                          >
                            {t('credentials.forget')}
                          </button>
                        )}
                      </div>
                    )}
                    <span className="text-xs text-muted">{t('credentials.keychainHint')}</span>
                  </div>
                )}
              </Field>
            )}
            {draft.protocol === 'ssh' && (
              <details
                className="col-span-full"
                open={advancedOpen || tunnelError}
                onToggle={(e) => setAdvancedOpen(e.currentTarget.open)}
              >
                <summary className="cursor-pointer text-sm text-secondary">
                  {t('editor.advanced')}
                </summary>
                <div className="mt-2 flex flex-col gap-3">
                  <label className="flex items-start gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={draft.legacyAlgorithms}
                      onChange={(e) => set('legacyAlgorithms', e.target.checked)}
                    />
                    <span>
                      {t('editor.legacyAlgorithms')}
                      <span className="block text-xs text-muted">
                        {t('editor.legacyAlgorithmsHint')}
                      </span>
                    </span>
                  </label>
                  <label className="flex items-start gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={draft.useAgent}
                      onChange={(e) => set('useAgent', e.target.checked)}
                    />
                    <span>
                      {t('editor.useAgent')}
                      <span className="block text-xs text-muted">{t('editor.useAgentHint')}</span>
                    </span>
                  </label>
                  <Field label={t('editor.keyPath')}>
                    {(id) => (
                      <input
                        id={id}
                        className="field mono"
                        value={draft.keyPath}
                        placeholder="~/.ssh/id_ed25519"
                        spellCheck={false}
                        autoCapitalize="off"
                        onChange={(e) => set('keyPath', e.target.value)}
                      />
                    )}
                  </Field>
                  <Field label={t('editor.keepalive')} error={errorText('keepalive')}>
                    {(id, describedBy) => (
                      <div className="flex flex-col gap-1">
                        <input
                          id={id}
                          className="field mono w-28"
                          inputMode="numeric"
                          value={draft.keepalive}
                          placeholder="30"
                          aria-describedby={describedBy}
                          onChange={(e) => set('keepalive', e.target.value)}
                        />
                        <span className="text-xs text-muted">{t('editor.keepaliveHint')}</span>
                      </div>
                    )}
                  </Field>
                  <Field label={t('editor.jump')} error={errorText('jump')}>
                    {(id, describedBy) => (
                      <div className="flex flex-col gap-1">
                        <input
                          id={id}
                          className="field mono"
                          value={draft.jump}
                          placeholder="admin@bastion.example.net:22"
                          spellCheck={false}
                          autoCapitalize="off"
                          aria-describedby={describedBy}
                          aria-invalid={Boolean(shown.jump)}
                          onChange={(e) => set('jump', e.target.value)}
                        />
                        <span className="text-xs text-muted">{t('editor.jumpHint')}</span>
                      </div>
                    )}
                  </Field>
                  <Field label={t('editor.proxy')} error={errorText('proxy')}>
                    {(id, describedBy) => (
                      <div className="flex gap-2">
                        <select
                          id={draft.proxyKind === 'none' ? id : undefined}
                          className="field w-32 shrink-0"
                          aria-label={t('editor.proxyKind')}
                          value={draft.proxyKind}
                          onChange={(e) => set('proxyKind', e.target.value as Draft['proxyKind'])}
                        >
                          <option value="none">{t('editor.proxyNone')}</option>
                          <option value="socks5">SOCKS5</option>
                          <option value="http">HTTP CONNECT</option>
                        </select>
                        {draft.proxyKind !== 'none' && (
                          <input
                            id={id}
                            className="field mono min-w-0 flex-1"
                            aria-label={t('editor.proxy')}
                            value={draft.proxy}
                            placeholder="127.0.0.1:1080"
                            spellCheck={false}
                            autoCapitalize="off"
                            aria-describedby={describedBy}
                            aria-invalid={Boolean(shown.proxy)}
                            onChange={(e) => set('proxy', e.target.value)}
                          />
                        )}
                      </div>
                    )}
                  </Field>
                  <Field label={t('editor.forwards')} error={errorText('forwards')}>
                    {(id, describedBy) => (
                      <div className="flex flex-col gap-1">
                        <textarea
                          id={id}
                          className="field mono min-h-16"
                          rows={3}
                          value={draft.forwards}
                          placeholder={'L 8080:10.0.0.10:80\nD 1080\nR 9000:127.0.0.1:22'}
                          spellCheck={false}
                          aria-describedby={describedBy}
                          aria-invalid={Boolean(shown.forwards)}
                          onChange={(e) => set('forwards', e.target.value)}
                        />
                        <span className="text-xs text-muted">{t('editor.forwardsHint')}</span>
                      </div>
                    )}
                  </Field>
                </div>
              </details>
            )}
          </>
        )}

        {serial && (
          <>
            <Field label={t('editor.port')} error={errorText('port')}>
              {(id, describedBy) => (
                <>
                  <input
                    id={id}
                    className="field mono"
                    list={portsListId}
                    value={draft.port}
                    aria-invalid={!!shown.port}
                    aria-describedby={describedBy}
                    placeholder={mac ? '/dev/cu.usbserial-…' : 'COM4'}
                    spellCheck={false}
                    autoCapitalize="off"
                    onChange={(e) => set('port', e.target.value)}
                  />
                  <datalist id={portsListId}>
                    {ports.map((p) => (
                      <option key={p.key} value={p.path}>
                        {`${p.displayName} · ${chipLabel(p)}`}
                      </option>
                    ))}
                  </datalist>
                </>
              )}
            </Field>

            <Field label={t('editor.baud')} error={errorText('baud')}>
              {(id, describedBy) => (
                <div className="flex gap-2">
                  <select
                    id={id}
                    className="field"
                    value={draft.baudChoice}
                    onChange={(e) => set('baudChoice', e.target.value as BaudChoice)}
                  >
                    <option value="auto">{t('serial.autoBaud')}</option>
                    {BAUD_PRESETS.map((b) => (
                      <option key={b} value={String(b)}>
                        {b}
                      </option>
                    ))}
                    <option value="custom">{t('editor.customBaud')}</option>
                  </select>
                  {draft.baudChoice === 'custom' && (
                    <input
                      className="field mono w-32 shrink-0"
                      inputMode="numeric"
                      aria-label={t('editor.customBaud')}
                      aria-invalid={!!shown.baud}
                      aria-describedby={describedBy}
                      placeholder={`${MIN_BAUD}–${MAX_BAUD}`}
                      value={draft.customBaud}
                      onChange={(e) => set('customBaud', e.target.value.replace(/[^\d]/g, ''))}
                    />
                  )}
                </div>
              )}
            </Field>

            <Field label={t('editor.framing')}>
              {(id) => (
                <div className="grid grid-cols-3 gap-2">
                  <select
                    id={id}
                    className="field"
                    aria-label={t('editor.dataBits')}
                    value={draft.dataBits}
                    onChange={(e) => set('dataBits', Number(e.target.value))}
                  >
                    {DATA_BITS.map((b) => (
                      <option key={b} value={b}>
                        {t('editor.dataBitsValue', { n: b })}
                      </option>
                    ))}
                  </select>
                  <select
                    className="field"
                    aria-label={t('editor.parity')}
                    value={draft.parity}
                    onChange={(e) => set('parity', e.target.value as Draft['parity'])}
                  >
                    <option value="none">{t('editor.parityNone')}</option>
                    <option value="odd">{t('editor.parityOdd')}</option>
                    <option value="even">{t('editor.parityEven')}</option>
                  </select>
                  <select
                    className="field"
                    aria-label={t('editor.stopBits')}
                    value={draft.stopBits}
                    onChange={(e) => set('stopBits', Number(e.target.value))}
                  >
                    {STOP_BITS.map((b) => (
                      <option key={b} value={b}>
                        {t('editor.stopBitsValue', { n: b })}
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </Field>

            <Field label={t('editor.flow')}>
              {(id) => (
                <select
                  id={id}
                  className="field"
                  value={draft.flowControl}
                  onChange={(e) => set('flowControl', e.target.value as Draft['flowControl'])}
                >
                  <option value="none">{t('editor.flowNone')}</option>
                  <option value="rtscts">{t('editor.flowRtsCts')}</option>
                  <option value="xonxoff">{t('editor.flowXonXoff')}</option>
                </select>
              )}
            </Field>

            <details
              className="col-span-full"
              open={lineOpen}
              onToggle={(e) => setLineOpen(e.currentTarget.open)}
            >
              <summary className="cursor-pointer text-sm text-secondary">
                {t('editor.advanced')}
              </summary>
              <div className="mt-2 grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2">
                {(['dtr', 'rts'] as const).map((line) => (
                  <Field key={line} label={line.toUpperCase()}>
                    {(id) => (
                      <select
                        id={id}
                        className="field w-40"
                        value={draft[line]}
                        onChange={(e) => set(line, e.target.value as LineChoice)}
                      >
                        <option value="default">{t('editor.lineDefault')}</option>
                        <option value="on">{t('editor.lineOn')}</option>
                        <option value="off">{t('editor.lineOff')}</option>
                      </select>
                    )}
                  </Field>
                ))}
                <p className="col-span-full text-xs text-muted">{t('editor.lineHint')}</p>
              </div>
            </details>
          </>
        )}

        <Field label={t('editor.env')} error={errorText('env')}>
          {(id, describedBy) => (
            <div className="flex gap-2">
              <select
                id={id}
                className="field"
                value={draft.envChoice}
                onChange={(e) => set('envChoice', e.target.value as EnvChoice)}
              >
                {ENV_CHOICES.map((c) => (
                  <option key={c} value={c}>
                    {t(`editor.env_${c}` as TKey)}
                  </option>
                ))}
              </select>
              {draft.envChoice === 'custom' && (
                <input
                  className="field mono w-32 shrink-0 uppercase"
                  aria-label={t('editor.envCustom')}
                  aria-invalid={!!shown.env}
                  aria-describedby={describedBy}
                  maxLength={MAX_ENV + 4}
                  value={draft.envCustom}
                  onChange={(e) => set('envCustom', e.target.value)}
                />
              )}
            </div>
          )}
        </Field>
      </form>
    </Dialog>
  );
}

function Field({
  label,
  error,
  children,
}: {
  label: string;
  error?: string | null;
  children: (id: string, describedBy: string | undefined) => ReactNode;
}) {
  const id = useId();
  const errId = useId();
  return (
    <>
      <label htmlFor={id} className="text-sm text-secondary">
        {label}
      </label>
      <div className="min-w-0">
        {children(id, error ? errId : undefined)}
        {error && (
          <p id={errId} className="mt-1 text-xs text-danger">
            {error}
          </p>
        )}
      </div>
    </>
  );
}
