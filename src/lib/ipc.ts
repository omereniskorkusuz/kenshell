/** Typed wrappers around Tauri commands. Keep every `invoke` call in this module. */
import { invoke } from '@tauri-apps/api/core';

export interface AppInfo {
  version: string;
  os: string;
  arch: string;
}

const inTauri = (): boolean => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function appReady(): Promise<void> {
  if (inTauri()) await invoke('app_ready');
}

export async function appInfo(): Promise<AppInfo | null> {
  return inTauri() ? invoke<AppInfo>('app_info') : null;
}

/** Platform-aware modifier label used in shortcut hints. */
export function modKey(): string {
  return /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';
}
