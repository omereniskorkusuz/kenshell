/** Side panel → Tools (Phase 5): file transfer, embedded file servers, network tools. */
import { useT, type TKey } from '../i18n';
import { useUi, type ToolId } from '../stores/ui';
import { FileServers } from './FileServers';
import { FileTransfer } from './FileTransfer';

const TOOLS: [ToolId, TKey][] = [
  ['transfer', 'transfer.title'],
  ['servers', 'servers.title'],
];

export function ToolsPanel() {
  const t = useT();
  const tool = useUi((s) => s.tool);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-b border-subtle px-2 pt-2">
        <select
          className="field h-7 w-full text-sm"
          aria-label={t('panel.tools')}
          value={tool}
          onChange={(e) => useUi.getState().openTool(e.target.value as ToolId)}
        >
          {TOOLS.map(([id, label]) => (
            <option key={id} value={id}>
              {t(label)}
            </option>
          ))}
        </select>
      </div>
      {tool === 'transfer' ? (
        <FileTransfer />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <FileServers />
        </div>
      )}
    </div>
  );
}
