/** Session editor form model (pure): record ⇄ draft, validation. */
import {
  DEFAULT_PORTS,
  DEFAULT_SERIAL,
  DEFAULT_SSH,
  type FlowControl,
  type Parity,
  type Protocol,
  type SessionInput,
} from './ipc';
import { isValidBaud, MAX_BAUD, MIN_BAUD } from './quickConnect';
import { envKind, normalizeEnv } from './tree';

export const BAUD_PRESETS = [9600, 19200, 38400, 57600, 115200] as const;
export const DATA_BITS = [5, 6, 7, 8] as const;
export const STOP_BITS = [1, 2] as const;
export const ENV_CHOICES = ['none', 'prod', 'lab', 'test', 'custom'] as const;
export const MAX_NAME = 128;
export const MAX_ENV = 12;

export type BaudChoice = 'auto' | `${(typeof BAUD_PRESETS)[number]}` | 'custom';
export type EnvChoice = (typeof ENV_CHOICES)[number];

export interface Draft {
  name: string;
  folderId: number | null;
  protocol: Protocol;
  port: string;
  baudChoice: BaudChoice;
  customBaud: string;
  dataBits: number;
  parity: Parity;
  stopBits: number;
  flowControl: FlowControl;
  envChoice: EnvChoice;
  envCustom: string;
  /** Network protocols. */
  host: string;
  netPort: string;
  username: string;
  legacyAlgorithms: boolean;
  keyPath: string;
  useAgent: boolean;
  credentialProfileId: number | null;
}

export function draftFrom(input: Partial<SessionInput> | null | undefined): Draft {
  const serial = { ...DEFAULT_SERIAL, port: '', ...(input?.serial ?? {}) };
  const baud = serial.baud;
  const preset = BAUD_PRESETS.find((b) => b === baud);
  const kind = envKind(input?.envTag);
  return {
    name: input?.name ?? '',
    folderId: input?.folderId ?? null,
    protocol: input?.protocol ?? 'serial',
    port: serial.port,
    baudChoice: baud === null ? 'auto' : preset ? (String(preset) as BaudChoice) : 'custom',
    customBaud: baud !== null && !preset ? String(baud) : '',
    dataBits: serial.dataBits,
    parity: serial.parity,
    stopBits: serial.stopBits,
    flowControl: serial.flowControl,
    envChoice: kind === null ? 'none' : kind,
    envCustom: kind === 'custom' ? (normalizeEnv(input?.envTag) ?? '') : '',
    host: input?.host ?? '',
    netPort: input?.port != null ? String(input.port) : '',
    username: input?.username ?? '',
    legacyAlgorithms: input?.ssh?.legacyAlgorithms ?? false,
    keyPath: input?.ssh?.keyPath ?? '',
    useAgent: input?.ssh?.useAgent ?? false,
    credentialProfileId: input?.credentialProfileId ?? null,
  };
}

export type DraftErrors = Partial<
  Record<'name' | 'port' | 'baud' | 'env' | 'host' | 'netPort', 'required' | 'range' | 'tooLong'>
>;

export function draftBaud(d: Draft): number | null {
  if (d.baudChoice === 'auto') return null;
  if (d.baudChoice === 'custom') return Number(d.customBaud.trim());
  return Number(d.baudChoice);
}

export function validateDraft(d: Draft): DraftErrors {
  const errors: DraftErrors = {};
  const name = d.name.trim();
  if (!name) errors.name = 'required';
  else if (name.length > MAX_NAME) errors.name = 'tooLong';
  if (d.protocol === 'serial' && !d.port.trim()) errors.port = 'required';
  if (d.protocol !== 'serial') {
    const host = d.host.trim();
    if (!host || /\s/.test(host)) errors.host = 'required';
    else if (host.length > 255) errors.host = 'tooLong';
    const raw = d.netPort.trim();
    if (raw === '') {
      if (DEFAULT_PORTS[d.protocol] === null) errors.netPort = 'required';
    } else if (!/^\d+$/.test(raw) || Number(raw) < 1 || Number(raw) > 65535) {
      errors.netPort = 'range';
    }
  }
  if (d.baudChoice === 'custom') {
    const raw = d.customBaud.trim();
    if (!/^\d+$/.test(raw) || !isValidBaud(Number(raw))) errors.baud = 'range';
  }
  if (d.envChoice === 'custom') {
    const env = d.envCustom.trim();
    if (!env) errors.env = 'required';
    else if (env.length > MAX_ENV) errors.env = 'tooLong';
  }
  return errors;
}

export const hasErrors = (e: DraftErrors): boolean => Object.keys(e).length > 0;

/** Builds the backend input; `base` keeps fields the form does not edit (tags, favorite …). */
export function draftToInput(d: Draft, base?: Partial<SessionInput>): SessionInput {
  const env =
    d.envChoice === 'none' ? null : d.envChoice === 'custom' ? d.envCustom.trim() : d.envChoice;
  return {
    folderId: d.folderId,
    name: d.name.trim(),
    protocol: d.protocol,
    host: d.protocol === 'serial' ? null : d.host.trim(),
    port: d.protocol === 'serial' || d.netPort.trim() === '' ? null : Number(d.netPort.trim()),
    username: d.protocol === 'serial' || !d.username.trim() ? null : d.username.trim(),
    serial:
      d.protocol === 'serial'
        ? {
            port: d.port.trim(),
            baud: draftBaud(d),
            dataBits: d.dataBits,
            parity: d.parity,
            stopBits: d.stopBits,
            flowControl: d.flowControl,
            dtr: base?.serial?.dtr ?? null,
            rts: base?.serial?.rts ?? null,
          }
        : null,
    envTag: env,
    tags: base?.tags ?? [],
    favorite: base?.favorite ?? false,
    credentialProfileId: d.protocol === 'ssh' ? d.credentialProfileId : null,
    ssh:
      d.protocol === 'ssh'
        ? {
            ...DEFAULT_SSH,
            ...(base?.ssh ?? {}),
            legacyAlgorithms: d.legacyAlgorithms,
            keyPath: d.keyPath.trim() || null,
            useAgent: d.useAgent,
          }
        : null,
  };
}

export { MAX_BAUD, MIN_BAUD };
