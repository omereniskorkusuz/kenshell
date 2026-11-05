/**
 * Live xterm handles per tab, for UI that sits outside the terminal component
 * (search bar). Kept xterm-free at the type level so the main chunk stays small.
 */
export interface SearchOptions {
  regex: boolean;
  caseSensitive: boolean;
}

export interface TerminalHandle {
  findNext(query: string, opts: SearchOptions): boolean;
  findPrevious(query: string, opts: SearchOptions): boolean;
  clearSearch(): void;
  focus(): void;
  /** Scrollback + screen as plain text, after xterm has parsed everything written. */
  snapshot(): Promise<string>;
}

const handles = new Map<string, TerminalHandle>();

export const registerTerminal = (key: string, h: TerminalHandle) => handles.set(key, h);
export const unregisterTerminal = (key: string) => handles.delete(key);
export const getTerminal = (key: string): TerminalHandle | undefined => handles.get(key);
