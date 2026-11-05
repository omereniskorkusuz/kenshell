/**
 * Vendor paging profile (P3-T14): the command that turns the device's pager off for this
 * login (`terminal length 0`, Huawei `screen-length 0 temporary` …), from `src/data/modes.json`.
 */
import modes from '../data/modes.json';

interface ProfileDef {
  id: string;
  paging: { more: string; disable: string };
}

const PROFILES = (modes as unknown as { profiles: ProfileDef[] }).profiles;

/**
 * For a vendor profile id (null = not identified: the Cisco-like default); null when the
 * vendor has no one-line command for it (OmniSwitch, FortiGate).
 */
export function pagingDisableCommand(profileId: string | null): string | null {
  const p = PROFILES.find((x) => x.id === profileId) ?? PROFILES.find((x) => x.id === 'cisco-like');
  return p ? p.paging.disable || null : 'terminal length 0';
}
