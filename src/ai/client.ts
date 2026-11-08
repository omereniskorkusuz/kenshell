/**
 * AI assistant client (P5-T08, ADR-020). This module — and the Anthropic SDK with it — is
 * only imported when the user turns the assistant on and sends something: with the
 * assistant off, none of this code is loaded.
 *
 * Runs in the webview with the user's own key (read from the keychain per request,
 * never stored in JS state). The text sent is exactly what the user saw in the
 * preview, already masked. The assistant only answers; it never sends anything to a
 * device.
 */
import Anthropic from '@anthropic-ai/sdk';

export const AI_MODELS = ['model-id', 'model-id', 'model-id'] as const;
export type AiModel = (typeof AI_MODELS)[number];

const SYSTEM = `You help network engineers working in a terminal app with Cisco IOS / IOS-XE, \
Ruijie RGOS and other network devices. You explain device output, interpret errors and \
suggest CLI commands. Be concise and practical. Put commands in fenced code blocks, one \
command per line. Prefer read-only show/display commands; when a suggestion changes \
configuration, say so clearly in one short line before it. You cannot run commands \
yourself and you see only the text below. Values shown as <masked> were hidden on purpose; \
do not ask for them. Answer in the language of the user's question.`;

export interface AskRequest {
  apiKey: string;
  model: AiModel;
  /** Masked terminal text (may be empty). */
  context: string;
  question: string;
  onText: (delta: string) => void;
  signal: AbortSignal;
}

export type AskError =
  | { kind: 'auth' }
  | { kind: 'rateLimit' }
  | { kind: 'refused' }
  | { kind: 'network' }
  | { kind: 'aborted' }
  | { kind: 'api'; status: number | null; message: string };

export async function ask(req: AskRequest): Promise<{ ok: true } | { ok: false; error: AskError }> {
  // A desktop app with the user's own key: there is no server of ours to proxy through.
  const client = new Anthropic({ apiKey: req.apiKey, dangerouslyAllowBrowser: true });
  const content = req.context
    ? `Terminal text:\n<terminal>\n${req.context}\n</terminal>\n\n${req.question}`
    : req.question;
  try {
    const stream = client.beta.messages.stream(
      {
        model: req.model,
        max_tokens: 16000,
        system: SYSTEM,
        output_config: { effort: 'medium' },
        // Refusals re-run on Anthropic's recommended fallback model (Opus / Sonnet only).
        ...(req.model === 'model-id'
          ? {}
          : { betas: ['server-side-fallback-2026-07-01'], fallbacks: 'default' as const }),
        messages: [{ role: 'user', content }],
      },
      { signal: req.signal },
    );
    for await (const event of stream) {
      if (event.type === 'content_block_delta' && event.delta.type === 'text_delta') {
        req.onText(event.delta.text);
      }
    }
    const final = await stream.finalMessage();
    if (final.stop_reason === 'refusal') return { ok: false, error: { kind: 'refused' } };
    return { ok: true };
  } catch (e) {
    if (e instanceof Anthropic.APIUserAbortError) return { ok: false, error: { kind: 'aborted' } };
    if (e instanceof Anthropic.AuthenticationError || e instanceof Anthropic.PermissionDeniedError)
      return { ok: false, error: { kind: 'auth' } };
    if (e instanceof Anthropic.RateLimitError) return { ok: false, error: { kind: 'rateLimit' } };
    if (e instanceof Anthropic.APIConnectionError) return { ok: false, error: { kind: 'network' } };
    if (e instanceof Anthropic.APIError)
      return { ok: false, error: { kind: 'api', status: e.status ?? null, message: e.message } };
    return { ok: false, error: { kind: 'api', status: null, message: String(e) } };
  }
}
