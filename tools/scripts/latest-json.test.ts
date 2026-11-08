import { describe, expect, it } from 'vitest';
// @ts-expect-error plain .mjs build script without types
import { manifest } from './latest-json.mjs';

describe('latest.json', () => {
  const files = [
    'a/macos/KenShell.app.tar.gz',
    'a/macos/KenShell.app.tar.gz.sig',
    'b/nsis/KenShell_0.2.0_x64-setup.exe',
    'b/nsis/KenShell_0.2.0_x64-setup.exe.sig',
    'b/msi/KenShell_0.2.0_x64_en-US.msi',
  ];
  const sig = (p: string) => `SIG(${p})\n`;

  it('maps signed bundles to updater platforms', () => {
    const m = manifest('0.2.0', 'https://dl.example/v0.2.0/', files, sig, 'notes', 'T');
    expect(m.version).toBe('0.2.0');
    expect(m.pub_date).toBe('T');
    expect(Object.keys(m.platforms).sort()).toEqual([
      'darwin-aarch64',
      'darwin-x86_64',
      'windows-x86_64',
    ]);
    expect(m.platforms['darwin-x86_64']).toEqual({
      signature: 'SIG(a/macos/KenShell.app.tar.gz.sig)',
      url: 'https://dl.example/v0.2.0/KenShell.app.tar.gz',
    });
    expect(m.platforms['windows-x86_64'].url).toBe(
      'https://dl.example/v0.2.0/KenShell_0.2.0_x64-setup.exe',
    );
  });

  it('refuses unsigned output', () => {
    expect(() => manifest('1', 'https://x', ['KenShell.app.tar.gz'], sig)).toThrow();
  });
});
