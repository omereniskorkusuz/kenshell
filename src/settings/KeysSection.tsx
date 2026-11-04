/**
 * SSH key generator / converter (P2-T16, PuTTYgen equivalent). Passphrases live only in
 * this form's state and are cleared after every action.
 */
import { useId, useState, type ReactNode } from 'react';
import { useT } from '../i18n';
import { copyText } from '../lib/clipboard';
import { describeError } from '../lib/errors';
import { api, type KeyFormat, type KeyInfo, type KeyKind } from '../lib/ipc';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';

const KINDS: { value: KeyKind; label: string }[] = [
  { value: 'ed25519', label: 'Ed25519' },
  { value: 'rsa3072', label: 'RSA 3072' },
  { value: 'rsa4096', label: 'RSA 4096' },
];
const FORMATS: { value: KeyFormat; label: string }[] = [
  { value: 'openssh', label: 'OpenSSH' },
  { value: 'ppk', label: 'PuTTY (.ppk)' },
];

function Choice<T extends string>({
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
    <div
      role="radiogroup"
      aria-label={label}
      className="flex w-fit gap-0.5 rounded-md bg-app p-0.5"
    >
      {options.map((o) => (
        <button
          key={o.value}
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

function Line({ label, children }: { label: string; children: (id: string) => ReactNode }) {
  const id = useId();
  return (
    <>
      <label htmlFor={id} className="text-sm text-secondary">
        {label}
      </label>
      <div className="min-w-0">{children(id)}</div>
    </>
  );
}

function fileName(kind: KeyKind, format: KeyFormat): string {
  const base = kind === 'ed25519' ? 'id_ed25519' : 'id_rsa';
  return format === 'ppk' ? `${base}.ppk` : base;
}

/** Shows a key's public part with a copy button. */
function KeyCard({ info }: { info: KeyInfo }) {
  const t = useT();
  const copy = async () => {
    const ok = await copyText(info.publicLine);
    useUi.getState().flash(ok ? t('keys.copied') : t('keys.copyFailed'));
  };
  return (
    <div className="flex flex-col gap-1 rounded-md border border-subtle p-2">
      <div className="flex items-center gap-2 text-xs text-secondary">
        <span className="mono">
          {info.algorithm}
          {info.bits ? ` · ${info.bits}` : ''}
        </span>
        {info.encrypted && <span>· {t('keys.encrypted')}</span>}
        <span className="mono ml-auto truncate" title={info.fingerprint}>
          {info.fingerprint}
        </span>
      </div>
      <textarea
        readOnly
        aria-label={t('keys.publicKey')}
        className="field mono min-h-14 text-xs"
        rows={2}
        value={info.publicLine}
        onFocus={(e) => e.currentTarget.select()}
      />
      <button type="button" className="btn btn-sm w-fit" onClick={() => void copy()}>
        {t('keys.copyPublic')}
      </button>
    </div>
  );
}

export default function KeysSection() {
  const t = useT();
  const [kind, setKind] = useState<KeyKind>('ed25519');
  const [format, setFormat] = useState<KeyFormat>('openssh');
  const [comment, setComment] = useState('');
  const [pass, setPass] = useState('');
  const [repeat, setRepeat] = useState('');
  const [created, setCreated] = useState<KeyInfo | null>(null);
  const [busy, setBusy] = useState(false);

  const [opened, setOpened] = useState<{ path: string; info: KeyInfo } | null>(null);
  const [openPass, setOpenPass] = useState('');
  const [target, setTarget] = useState<KeyFormat>('ppk');
  const [newPass, setNewPass] = useState('');

  const mismatch = repeat !== pass;
  const fail = (e: unknown) => {
    const d = describeError(e, t);
    toast({ kind: 'error', message: d.message, detail: d.detail });
  };

  const generate = async () => {
    const path = await api.pickPath('save', [], fileName(kind, format)).catch(() => null);
    if (!path) return;
    setBusy(true);
    try {
      setCreated(await api.keyGenerate(kind, comment, format, pass, path));
      setPass('');
      setRepeat('');
      useUi.getState().flash(t('keys.saved', { path }));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const open = async () => {
    const path = await api.pickPath('file').catch(() => null);
    if (!path) return;
    try {
      const info = await api.keyInspect(path);
      setOpened({ path, info });
      setOpenPass('');
      setTarget(path.toLowerCase().endsWith('.ppk') ? 'openssh' : 'ppk');
    } catch (e) {
      fail(e);
    }
  };

  const convert = async () => {
    if (!opened) return;
    const stem = opened.path.replace(/^.*[\\/]/, '').replace(/\.ppk$/i, '');
    const name = target === 'ppk' ? `${stem}.ppk` : `${stem}_openssh`;
    const dest = await api.pickPath('save', [], name).catch(() => null);
    if (!dest) return;
    setBusy(true);
    try {
      await api.keyConvert(opened.path, openPass, target, newPass, dest);
      setOpenPass('');
      setNewPass('');
      useUi.getState().flash(t('keys.saved', { path: dest }));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <h3 className="label-caps">{t('keys.generate')}</h3>
      <div className="grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2">
        <span className="text-sm text-secondary">{t('keys.type')}</span>
        <Choice label={t('keys.type')} value={kind} options={KINDS} onChange={setKind} />
        <span className="text-sm text-secondary">{t('keys.format')}</span>
        <Choice label={t('keys.format')} value={format} options={FORMATS} onChange={setFormat} />
        <Line label={t('keys.comment')}>
          {(id) => (
            <input
              id={id}
              className="field"
              value={comment}
              placeholder="admin@laptop"
              spellCheck={false}
              onChange={(e) => setComment(e.target.value)}
            />
          )}
        </Line>
        <Line label={t('keys.passphrase')}>
          {(id) => (
            <input
              id={id}
              type="password"
              className="field"
              autoComplete="new-password"
              value={pass}
              placeholder={t('keys.optional')}
              onChange={(e) => setPass(e.target.value)}
            />
          )}
        </Line>
        <Line label={t('keys.repeat')}>
          {(id) => (
            <input
              id={id}
              type="password"
              className="field"
              autoComplete="new-password"
              value={repeat}
              aria-invalid={mismatch}
              onChange={(e) => setRepeat(e.target.value)}
            />
          )}
        </Line>
        {mismatch && repeat !== '' && (
          <p className="col-start-2 text-xs text-danger" role="status">
            {t('keys.mismatch')}
          </p>
        )}
      </div>
      <p className="text-xs text-muted">{t('keys.generateHint')}</p>
      <button
        type="button"
        className="btn w-fit"
        disabled={busy || mismatch}
        onClick={() => void generate()}
      >
        {busy ? t('keys.working') : t('keys.generateSave')}
      </button>
      {created && <KeyCard info={created} />}

      <h3 className="label-caps mt-2">{t('keys.convert')}</h3>
      <p className="text-xs text-muted">{t('keys.convertHint')}</p>
      <button type="button" className="btn w-fit" disabled={busy} onClick={() => void open()}>
        {t('keys.open')}
      </button>
      {opened && (
        <>
          <p className="mono truncate text-xs text-secondary" title={opened.path}>
            {opened.path}
          </p>
          <KeyCard info={opened.info} />
          <div className="grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2">
            {opened.info.encrypted && (
              <Line label={t('keys.currentPassphrase')}>
                {(id) => (
                  <input
                    id={id}
                    type="password"
                    className="field"
                    autoComplete="current-password"
                    value={openPass}
                    onChange={(e) => setOpenPass(e.target.value)}
                  />
                )}
              </Line>
            )}
            <span className="text-sm text-secondary">{t('keys.saveAs')}</span>
            <Choice
              label={t('keys.saveAs')}
              value={target}
              options={FORMATS}
              onChange={setTarget}
            />
            <Line label={t('keys.newPassphrase')}>
              {(id) => (
                <input
                  id={id}
                  type="password"
                  className="field"
                  autoComplete="new-password"
                  value={newPass}
                  placeholder={t('keys.optional')}
                  onChange={(e) => setNewPass(e.target.value)}
                />
              )}
            </Line>
          </div>
          <button
            type="button"
            className="btn w-fit"
            disabled={busy || (opened.info.encrypted && !openPass)}
            onClick={() => void convert()}
          >
            {t('keys.convertSave')}
          </button>
        </>
      )}
    </div>
  );
}
