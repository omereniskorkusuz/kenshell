/** Side panel → Tools (Phase 5): file transfer, TFTP server, network tools. */
import { useState } from 'react';
import { useT, type TKey } from '../i18n';
import { FileTransfer } from './FileTransfer';
import { TftpServer } from './TftpServer';

const TOOLS = [
  ['transfer', 'transfer.title'],
  ['tftp', 'tftp.title'],
] as const;

export function ToolsPanel() {
  const t = useT();
  const [tool, setTool] = useState<(typeof TOOLS)[number][0]>('transfer');
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-b border-subtle px-2 pt-2">
        <select
          className="field h-7 w-full text-sm"
          aria-label={t('panel.tools')}
          value={tool}
          onChange={(e) => setTool(e.target.value as typeof tool)}
        >
          {TOOLS.map(([id, label]) => (
            <option key={id} value={id}>
              {t(label as TKey)}
            </option>
          ))}
        </select>
      </div>
      {tool === 'transfer' ? (
        <FileTransfer />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <TftpServer />
        </div>
      )}
    </div>
  );
}
