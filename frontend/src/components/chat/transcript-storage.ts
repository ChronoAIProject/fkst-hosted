import type { ChatMessage } from './chat-context';

/**
 * Transcript persistence.
 *
 * Split out of the chat context so that file stays under the 500-line limit, and
 * because durability is its own concern: everything here is about surviving a
 * reload safely, which is why every path degrades to "empty transcript" rather
 * than throwing.
 *
 * `sessionStorage`, not `localStorage`: a chat transcript is a working
 * conversation, not a saved document, and per-tab scope means two tabs do not
 * fight over one thread.
 */
const STORAGE_KEY = 'fkst-chat-transcript';

/** Transcript cap. Bounded because a long conversation would otherwise grow the
 *  stored payload without limit; the oldest messages are the least useful. */
const MAX_STORED_MESSAGES = 100;

/** Read the stored transcript, tolerating anything. A corrupt or foreign value
 *  must degrade to an empty transcript, never break the panel. */
export function readStored(): ChatMessage[] {
  try {
    const raw = window.sessionStorage?.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(
        (entry): entry is ChatMessage =>
          typeof entry === 'object' &&
          entry != null &&
          typeof (entry as ChatMessage).id === 'string' &&
          typeof (entry as ChatMessage).content === 'string'
      )
      .map(restoreProposals);
  } catch {
    return [];
  }
}

/** Rehydrate a restored message's proposals.
 *
 *  A proposal stored as `executing` is unknowable after a reload: the request may
 *  have succeeded, failed, or never left. It becomes `failed` with a note telling
 *  the user to check the dashboard — because the one thing that must NEVER happen
 *  is re-executing it silently on restore.
 *
 *  `RESTORED_UNKNOWN` is a sentinel, not copy: the component localizes it. */
export const RESTORED_UNKNOWN = 'restored-unknown';

function restoreProposals(message: ChatMessage): ChatMessage {
  if (message.proposals == null || message.proposals.length === 0) return message;
  return {
    ...message,
    proposals: message.proposals.map((entry) =>
      entry.state === 'executing'
        ? { ...entry, state: 'failed' as const, error: RESTORED_UNKNOWN }
        : entry
    ),
  };
}

export function writeStored(messages: ChatMessage[]) {
  try {
    // An EMPTY transcript removes the key rather than storing `[]`. That makes this
    // effect the single writer: a clear (or a sign-out) does not need to also call
    // removeItem, which the very next persist would undo anyway.
    if (messages.length === 0) {
      window.sessionStorage?.removeItem(STORAGE_KEY);
      return;
    }
    // A restored `pending` message would show a caret that never resolves, so the
    // flag is dropped on the way out.
    const storable = messages.slice(-MAX_STORED_MESSAGES).map((message) => {
      const stored: ChatMessage = { ...message };
      delete stored.pending;
      return stored;
    });
    window.sessionStorage?.setItem(STORAGE_KEY, JSON.stringify(storable));
  } catch {
    // Storage can be full or blocked; the transcript still works in memory.
  }
}

