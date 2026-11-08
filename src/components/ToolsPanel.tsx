/** Side panel → Tools (Phase 5): file transfer, embedded file servers, network tools. */
import { AI_FEATURE } from '../lib/features';
import { useT, type TKey } from '../i18n';
import { useSettings } from '../stores/settings';
import { useUi, type ToolId } from '../stores/ui';
import { AssistantPanel } from './AssistantPanel';
import { FileServers } from './FileServers';
import { FileTransfer } from './FileTransfer';
import { NetTools } from './NetTools';

const TOOLS: [ToolId, TKey][] = [
  ['transfer', 'transfer.title'],
  ['servers', 'servers.title'],
  ['nettools', 'nettools.title'],
  ['ai', 'ai.title'],
];

export function ToolsPanel() {
  const t = useT();
  const aiOn = useSettings((s) => s.values['ai.enabled']) && AI_FEATURE;
  const picked = useUi((s) => s.tool);
  // The assistant is listed only while it is turned on.
  const tool = picked === 'ai' && !aiOn ? 'transfer' : picked;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-b border-subtle px-2 pt-2">
        <select
          className="field h-7 w-full text-sm"
          aria-label={t('panel.tools')}
          value={tool}
          onChange={(e) => useUi.getState().openTool(e.target.value as ToolId)}
        >
          {TOOLS.filter(([id]) => id !== 'ai' || aiOn).map(([id, label]) => (
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
          {tool === 'servers' ? <FileServers /> : tool === 'ai' ? <AssistantPanel /> : <NetTools />}
        </div>
      )}
    </div>
  );
}
