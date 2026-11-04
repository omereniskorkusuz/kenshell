/**
 * AppError → human, translated message. Raw OS text (`detail`) is NEVER shown as the message;
 * it is only offered as a tooltip/title for support.
 */
import type { TKey } from '../i18n';
import type { Translate } from './autobaud';
import { isAppError, type ErrorCode } from './ipc';

const KEYS: Record<ErrorCode, TKey> = {
  portBusy: 'errors.portBusy',
  portNotFound: 'errors.portNotFound',
  permissionDenied: 'errors.permissionDenied',
  deviceGone: 'errors.deviceGone',
  hostNotFound: 'errors.hostNotFound',
  connectionRefused: 'errors.connectionRefused',
  timeout: 'errors.timeout',
  authFailed: 'errors.authFailed',
  hostKeyChanged: 'errors.hostKeyChanged',
  noCommonAlgorithm: 'errors.noCommonAlgorithm',
  connectionClosed: 'errors.connectionClosed',
  notFound: 'errors.notFound',
  invalid: 'errors.invalid',
  unsupported: 'errors.unsupported',
  io: 'errors.io',
  db: 'errors.db',
  internal: 'errors.internal',
};

export function errorCode(e: unknown): ErrorCode {
  if (isAppError(e) && typeof e.code === 'string' && e.code in KEYS) return e.code;
  return 'internal';
}

export function errorKey(e: unknown): TKey {
  return KEYS[errorCode(e)];
}

/** Technical detail for a tooltip, or null. Never contains secrets (backend contract). */
export function errorDetail(e: unknown): string | null {
  if (isAppError(e)) return typeof e.detail === 'string' && e.detail ? e.detail : null;
  if (e instanceof Error) return e.message || null;
  if (typeof e === 'string') return e || null;
  return null;
}

export interface ErrorContext {
  /** Port/target the operation was about (e.g. "COM4"). */
  port?: string | null;
}

export interface DescribedError {
  code: ErrorCode;
  message: string;
  detail: string | null;
}

export function describeError(e: unknown, t: Translate, ctx: ErrorContext = {}): DescribedError {
  const code = errorCode(e);
  const port = ctx.port && ctx.port.trim() ? ctx.port : t('errors.thePort');
  return { code, message: t(KEYS[code], { port }), detail: errorDetail(e) };
}
