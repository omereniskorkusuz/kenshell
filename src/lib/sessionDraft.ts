/** Session editor form model (pure): record ⇄ draft, validation. */
import {
  DEFAULT_SERIAL,
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
  };
}

export type DraftErrors = Partial<
  Record<'name' | 'port' | 'baud' | 'env', 'required' | 'range' | 'tooLong'>
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
    host: base?.host ?? null,
    port: base?.port ?? null,
    username: base?.username ?? null,
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
  };
}

export { MAX_BAUD, MIN_BAUD };
