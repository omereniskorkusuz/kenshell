/**
 * Neutral display names for vendor families. The app never shows brand names: a family
 * appears as "Type A", "Type B"… (i18n `vendorFamily.*`); `generic` is the standard CLI
 * assumed until the device is identified.
 */
import type { TKey, TParams } from '../i18n';

export function familyLabel(
  t: (key: TKey, params?: TParams) => string,
  family: string | null | undefined,
): string {
  return family ? t(`vendorFamily.${family}` as TKey) : '';
}
