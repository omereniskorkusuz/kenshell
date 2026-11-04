/** Saved session editor (a form, not a confirmation). Serial only in this version. */
import { useId, useMemo, useState, type FormEvent, type ReactNode } from 'react';
import { useT, type TKey } from '../i18n';
import { isMac, type Protocol } from '../lib/ipc';
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
  MAX_NAME,
  MIN_BAUD,
  STOP_BITS,
  validateDraft,
  type BaudChoice,
  type Draft,
  type DraftErrors,
  type EnvChoice,
} from '../lib/sessionDraft';
import { folderPath } from '../lib/tree';
import { chipLabel, useConsoles } from '../stores/consoles';
import { useLibrary } from '../stores/library';
import { useTabs } from '../stores/tabs';
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
  const errors = useMemo(() => validateDraft(draft), [draft]);
  const shown: DraftErrors = touched ? errors : {};
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

        <Field label={t('editor.protocol')}>
          {(id) => (
            <select
              id={id}
              className="field"
              value={draft.protocol}
              onChange={(e) => set('protocol', e.target.value as Protocol)}
            >
              {PROTOCOLS.map((p) => (
                <option key={p.id} value={p.id} disabled={p.id !== 'serial'}>
                  {p.id === 'serial'
                    ? t(p.label)
                    : t('editor.notSupported', { protocol: t(p.label) })}
                </option>
              ))}
            </select>
          )}
        </Field>

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
