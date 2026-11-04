import { api, inTauri } from './ipc';

/** Clipboard text ("" when empty or refused). */
export async function readText(): Promise<string> {
  try {
    return await api.clipboardRead();
  } catch {
    return '';
  }
}

/** Copies text to the clipboard; resolves false when the platform refuses. */
export async function copyText(text: string): Promise<boolean> {
  if (inTauri()) {
    try {
      await api.clipboardWrite(text);
      return true;
    } catch {
      // fall through to the webview clipboard
    }
  }
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Fallback for webviews without the async clipboard permission.
    try {
      const el = document.createElement('textarea');
      el.value = text;
      el.setAttribute('readonly', '');
      el.style.position = 'fixed';
      el.style.opacity = '0';
      document.body.appendChild(el);
      el.select();
      const ok = document.execCommand('copy');
      el.remove();
      return ok;
    } catch {
      return false;
    }
  }
}
