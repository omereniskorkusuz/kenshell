/**
 * Masking before anything goes to the AI assistant (P5-T08): passwords, secrets, keys,
 * SNMP communities / auth / priv keys and PEM private keys are replaced with
 * `<masked>`. The masked text is what the user sees in the preview and what is sent.
 * Works on Cisco IOS / IOS-XE, Ruijie RGOS and similar CLI configs and output.
 */

export const MASK = '<masked>';

const KEYWORDS = new Set([
  'password',
  'secret',
  'key',
  'key-string',
  'community',
  'pre-shared-key',
  'authentication-key',
  'passphrase',
  'psk',
  'wpa-psk',
]);
/** Words after `key` that name something rather than introduce a secret. */
const NOT_SECRET = new Set(['chain', 'config-key', 'exchange', 'generate', 'ring', 'pair']);
const ALGOS = new Set(['md5', 'sha', 'sha256', 'sha384', 'sha512', 'des', '3des', 'aes']);

/** Masks one line; returns it and how many values were masked. */
export function maskLine(line: string): { text: string; count: number } {
  // Tokens with their separators kept, so the layout is unchanged.
  const parts = line.split(/(\s+)/);
  const words: number[] = [];
  parts.forEach((p, i) => {
    if (p && !/^\s+$/.test(p)) words.push(i);
  });
  let count = 0;
  const mask = (w: number) => {
    const i = words[w];
    if (i === undefined) return;
    parts[i] = MASK;
    count++;
  };
  const word = (w: number) => (parts[words[w] ?? -1] ?? '').toLowerCase();
  for (let w = 0; w < words.length; w++) {
    const cur = word(w);
    if (KEYWORDS.has(cur)) {
      let v = w + 1;
      if (cur === 'key' && NOT_SECRET.has(word(v))) continue;
      // Ruijie `enable secret level 15 5 <hash>`, Cisco `secret 5 <hash>`.
      if (word(v) === 'level' && /^\d+$/.test(word(v + 1))) v += 2;
      if (/^\d$/.test(word(v)) && words[v + 1] !== undefined) v += 1;
      if (words[v] !== undefined && word(v) !== MASK.toLowerCase()) {
        mask(v);
        w = v;
      }
    } else if ((cur === 'auth' || cur === 'priv') && ALGOS.has(word(w + 1))) {
      // SNMPv3 `auth sha <key> priv aes 128 <key>`.
      let v = w + 2;
      if (/^\d{3}$/.test(word(v))) v += 1;
      if (words[v] !== undefined) {
        mask(v);
        w = v;
      }
    }
  }
  return { text: parts.join(''), count };
}

/** Masks a whole text; PEM private-key blocks are replaced entirely. */
export function maskSecrets(text: string): { text: string; count: number } {
  let count = 0;
  const pem = text.replace(
    /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g,
    () => {
      count++;
      return MASK;
    },
  );
  const lines = pem.split('\n').map((l) => {
    const m = maskLine(l);
    count += m.count;
    return m.text;
  });
  return { text: lines.join('\n'), count };
}
