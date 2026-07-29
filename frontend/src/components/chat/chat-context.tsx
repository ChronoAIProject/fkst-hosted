import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { ReactNode } from 'react';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { useToast } from '@/components/ui/toast';
import { parseActionProposal } from './action-types';
import type { ActionProposal } from './action-types';
import { parseDataCard } from './data-card-types';
import type { DataCard } from './data-card-types';
import type { ProposalExecutionInput } from './proposal-exec';
import { errorCopy, nextId } from './chat-helpers';
import { readStored, writeStored } from './transcript-storage';
// Re-exported so this module stays the façade its consumers already import from.
export { RESTORED_UNKNOWN } from './transcript-storage';
import { useProposals } from './use-proposals';
import {
  appendRoundStart,
  appendToolCall,
  applyRoundEnd,
  applyToolResult,
  isViewLevel,
} from './steps';
import type { ChatStep, ChatViewLevel } from './steps';
import { mockEchoTransport } from './transport';
import type { ChatTransport, ChatTurnMessage, SessionRef } from './transport';
import { prefersReducedMotion, TypewriterQueue } from './typewriter';

export type { ChatStep, ChatToolStep, ChatViewLevel } from './steps';

/** How far a proposal's execution has got.
 *
 *  `failed` covers both a rejected mutation and the unknowable case: a transcript
 *  restored while a request was in flight, where the outcome cannot be recovered. */
export type ProposalExecState = 'idle' | 'executing' | 'succeeded' | 'failed';

/** A confirm-gated proposal as the UI tracks it. */
export interface ChatProposal {
  /** Client-assigned: the wire union carries no id. */
  id: string;
  proposal: ActionProposal;
  state: ProposalExecState;
  /** Server message on failure, or the restored-mid-flight note. */
  error?: string;
  /** The created issue's URL on success. */
  issueUrl?: string;
  /** The created issue's number on success — the dashboard deep link needs it. */
  issueNumber?: number;
}

/** One entry in the visible transcript.
 *
 *  `system-note` is a LOCAL message (an error, an outcome) that never goes on the
 *  wire — keeping it in the same list means the user reads one chronological
 *  thread instead of a transcript plus a separate notification area. */
export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system-note';
  content: string;
  /** True while the assistant message is still growing. */
  pending?: boolean;
  /** The user sent the next question before this answer finished. Kept on the
   *  record — with whatever text and steps it had — rather than deleted, so the
   *  transcript stays a truthful account of what happened. */
  interrupted?: boolean;
  /** The orchestration loop attributed to this assistant message, in arrival order. */
  steps?: ChatStep[];
  /** The view level captured when THIS turn started. Rendering reads this rather
   *  than the live setting, so toggling never rewrites a turn already on screen. */
  viewLevel?: ChatViewLevel;
  /** Sessions the turn identified, for deep-linking cards. */
  sessionRefs?: SessionRef[];
  /** Confirm-gated action proposals drafted during this turn. */
  proposals?: ChatProposal[];
  /** Structured renderings of the turn's tool results, in arrival order. */
  dataCards?: DataCard[];
  /** A `system-note` that should read as a warning rather than information. */
  tone?: 'info' | 'warn';
}

interface ChatContextValue {
  open: boolean;
  messages: ChatMessage[];
  streaming: boolean;
  /** The level the NEXT turn will be captured at. */
  viewLevel: ChatViewLevel;
  setViewLevel: (level: ChatViewLevel) => void;
  openPanel: () => void;
  closePanel: () => void;
  toggle: () => void;
  sendMessage: (text: string) => void;
  stopStreaming: () => void;
  clearTranscript: () => void;
  /** Execute a reviewed proposal under the user's own token. Nothing ever runs
   *  without a deliberate call to this. */
  executeProposal: (id: string, input?: ProposalExecutionInput) => Promise<void>;
  /** Drop a proposal the user does not want. */
  dismissProposal: (id: string) => void;
  /** Record that a proposal's mutation ALREADY ran elsewhere and succeeded.
   *
   *  Exists for exactly one caller: the stop path, where `ConfirmDialog` owns the
   *  mutation by contract. Calling `executeProposal` after the dialog succeeded
   *  would close the trigger a second time. */
  markProposalSucceeded: (id: string) => void;
}

const ChatContext = createContext<ChatContextValue | null>(null);

/** Persisted CLEAN/VERBOSE preference. */
const VIEW_LEVEL_KEY = 'fkst-chat-view-level';

/**
 * Owns the Orchestrator's client state: panel visibility, the transcript, and the
 * one in-flight turn.
 *
 * The transport is injected rather than imported so the shell can supply the real
 * SSE client while tests and this milestone's review supply the mock — the
 * provider itself never knows which it has.
 */
export function ChatProvider({
  children,
  transport = mockEchoTransport,
}: {
  children: ReactNode;
  transport?: ChatTransport;
}) {
  const { isAuthenticated, apiFetch } = useAuth();
  const s = useContent().chat;
  const { show: showToast } = useToast();
  const [open, setOpen] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>(() => readStored());
  const [streaming, setStreaming] = useState(false);
  // localStorage, not sessionStorage: unlike the transcript (a per-tab working
  // conversation) this is a durable preference about how much machinery you want
  // to see, and it should survive a new tab.
  const [viewLevel, setViewLevelState] = useState<ChatViewLevel>(() => {
    try {
      const stored = window.localStorage.getItem(VIEW_LEVEL_KEY);
      return isViewLevel(stored) ? stored : 'clean';
    } catch {
      // A blocked or full storage must never stop the panel opening.
      return 'clean';
    }
  });
  const abortRef = useRef<AbortController | null>(null);
  // The turn's reveal buffer. A provider emits whatever it happened to flush — often a
  // whole paragraph at once — so what the transport delivers is NOT what the transcript
  // shows: deltas go in here and come out at a readable rate. See `./typewriter`.
  const typewriterRef = useRef<TypewriterQueue | null>(null);

  // Persist on every change so a refresh (or an accidental tab switch) keeps the
  // conversation the user is reading.
  useEffect(() => {
    writeStored(messages);
  }, [messages]);

  // Sign-out (or an expiry) must not leave one user's conversation on screen for
  // the next person on a shared machine. Only a true→false transition clears, so
  // a visitor who was never signed in keeps whatever they were reading.
  const wasAuthenticated = useRef(isAuthenticated);
  useEffect(() => {
    if (wasAuthenticated.current && !isAuthenticated) {
      abortRef.current?.abort();
      typewriterRef.current?.cancel();
      setMessages([]);
      setStreaming(false);
    }
    wasAuthenticated.current = isAuthenticated;
  }, [isAuthenticated]);

  // A turn outlives the panel being closed only if we let it; aborting on unmount
  // stops a dead component's callbacks from firing — and cancelling the reveal stops
  // its interval from ticking into an unmounted tree.
  useEffect(
    () => () => {
      abortRef.current?.abort();
      typewriterRef.current?.cancel();
    },
    []
  );

  /** Switch the level for FUTURE turns and mark the switch in the transcript.
   *
   *  Deliberately does not touch existing messages: each carries the level it was
   *  produced under, so the change reads as a point in the conversation rather
   *  than a silent rewrite of what the user already saw. The note is what makes
   *  that boundary visible. */
  const setViewLevel = useCallback(
    (level: ChatViewLevel) => {
      setViewLevelState((current) => {
        if (current === level) return current;
        try {
          window.localStorage.setItem(VIEW_LEVEL_KEY, level);
        } catch {
          // Preference lost on reload is acceptable; failing the toggle is not.
        }
        setMessages((messages) => [
          ...messages,
          {
            id: nextId('n'),
            role: 'system-note',
            content: level === 'verbose' ? s.viewLevelNoteVerbose : s.viewLevelNoteClean,
            tone: 'info',
          },
        ]);
        return level;
      });
    },
    [s]
  );

  const stopStreaming = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    // Flush rather than cancel: the user asked to stop the ANSWER, not to discard the
    // words already streamed and paid for. They appear at once, which is the honest
    // rendering of "this is everything that arrived".
    typewriterRef.current?.flush();
    typewriterRef.current = null;
    setStreaming(false);
    setMessages((current) =>
      current.map((message) => (message.pending ? { ...message, pending: false } : message))
    );
  }, []);

  /** Which turn is current. Bumped per send so a superseded turn's late callbacks
   *  can tell they no longer own the panel's state. */
  const turnSeqRef = useRef(0);

  /** Apply an update to the assistant message with `id`. */
  const patch = useCallback((id: string, update: (message: ChatMessage) => ChatMessage) => {
    setMessages((current) =>
      current.map((message) => (message.id === id ? update(message) : message))
    );
  }, []);

  const sendMessage = useCallback(
    (text: string) => {
      const content = text.trim();
      if (!content) return;

      // Sending WHILE a turn streams interrupts it. Previously this returned early,
      // so changing your mind mid-answer — the common case when the orchestrator is
      // off down the wrong path — meant pressing Stop, then typing, then sending.
      if (streaming) {
        abortRef.current?.abort();
        abortRef.current = null;
        // Flush rather than cancel: the user is redirecting, not disowning what was
        // already said, so the partial answer stays on the record and is MARKED as
        // interrupted rather than silently looking like a complete reply.
        typewriterRef.current?.flush();
        typewriterRef.current = null;
        setMessages((current) =>
          current.map((message) =>
            message.pending ? { ...message, pending: false, interrupted: true } : message
          )
        );
      }

      // Generation token. The aborted turn's in-flight callbacks can still fire, and
      // without this its terminal handler would clear `streaming` and null the
      // abort ref belonging to the turn that REPLACED it.
      turnSeqRef.current += 1;
      const turn = turnSeqRef.current;
      const isCurrentTurn = () => turnSeqRef.current === turn;

      const userMessage: ChatMessage = { id: nextId('u'), role: 'user', content };
      const assistantId = nextId('a');
      const assistantMessage: ChatMessage = {
        id: assistantId,
        role: 'assistant',
        content: '',
        pending: true,
        steps: [],
        // Captured HERE, at turn start, so a toggle later in the session cannot
        // change how this turn renders.
        viewLevel,
      };

      // The wire history is built from what is on screen PLUS this message, and
      // carries only user/assistant content — never a local notice, and never the
      // empty placeholder we are about to add.
      const history: ChatTurnMessage[] = [...messages, userMessage]
        .filter(
          (message): message is ChatMessage & { role: 'user' | 'assistant' } =>
            (message.role === 'user' || message.role === 'assistant') &&
            message.content.trim().length > 0
        )
        .map((message) => ({ role: message.role, content: message.content }));

      setMessages((current) => [...current, userMessage, assistantMessage]);
      setStreaming(true);

      const controller = new AbortController();
      abortRef.current = controller;

      // One queue per turn, writing into THIS turn's assistant message. Reduced motion
      // is read here rather than captured once, so an OS-setting change takes effect on
      // the next question instead of needing a reload.
      const typewriter = new TypewriterQueue(
        (slice) => patch(assistantId, (message) => ({ ...message, content: message.content + slice })),
        { instant: prefersReducedMotion() }
      );
      typewriterRef.current = typewriter;

      transport.send(
        history,
        {
          onDelta: (delta) => typewriter.push(delta),
          // The four step handlers fold the orchestration loop into one ORDERED
          // list; the reducers live in `./steps` so their ordering rules are
          // testable without mounting anything.
          onRoundStart: (ev) =>
            patch(assistantId, (message) => ({
              ...message,
              steps: appendRoundStart(message.steps ?? [], ev),
            })),
          onRoundEnd: (ev) =>
            patch(assistantId, (message) => ({
              ...message,
              steps: applyRoundEnd(message.steps ?? [], ev),
            })),
          onToolCall: (ev) =>
            patch(assistantId, (message) => ({
              ...message,
              steps: appendToolCall(message.steps ?? [], ev),
            })),
          onToolResult: (ev) =>
            patch(assistantId, (message) => ({
              ...message,
              steps: applyToolResult(message.steps ?? [], ev),
            })),
          onDataCard: (raw) => {
            const card = parseDataCard(raw);
            // An unreadable card is DROPPED, not reported: the prose answer still
            // stands on its own, and a note about a rendering detail would be noise.
            if (card == null) return;
            patch(assistantId, (message) => ({
              ...message,
              dataCards: [...(message.dataCards ?? []), card],
            }));
          },
          onActionProposal: (raw) => {
            const proposal = parseActionProposal(raw);
            if (proposal == null) {
              // An unreadable or unrecognized draft becomes a note, NOT an error
              // toast and never a card: the turn is fine, only this draft is not,
              // and a card the SPA cannot execute is worse than saying so.
              setMessages((current) => [
                ...current,
                {
                  id: nextId('n'),
                  role: 'system-note',
                  content: s.unreadableProposal,
                  tone: 'info',
                },
              ]);
              return;
            }
            patch(assistantId, (message) => ({
              ...message,
              proposals: [
                ...(message.proposals ?? []),
                { id: nextId('p'), proposal, state: 'idle' as const },
              ],
            }));
          },
          onDone: ({ sessionRefs }) => {
            // The wire is done; the reader is not. Dropping the caret and re-enabling the
            // composer while text is still appearing would contradict what is on screen,
            // so the turn "ends" when the reveal drains.
            typewriter.finish(() => {
              // Session refs still belong on THIS message even if it was superseded —
              // only the shared panel state below is off-limits to an old turn.
              patch(assistantId, (message) => ({ ...message, pending: false, sessionRefs }));
              if (!isCurrentTurn()) return;
              setStreaming(false);
              abortRef.current = null;
              typewriterRef.current = null;
            });
          },
          onError: ({ code, message, retryAfterSeconds }) => {
            // An interrupted turn's failure is not the user's problem: they already
            // moved on, and a warning note about the answer they abandoned would be
            // noise attached to the wrong question.
            if (!isCurrentTurn()) return;
            // Show whatever had already streamed before the failure — a partial answer is
            // more useful than a blank bubble above the error note.
            typewriter.flush();
            typewriterRef.current = null;
            // Copy comes from the stable CODE, not the server's prose: the prose is
            // for the log, and a user-facing string must be translatable. The raw
            // message is kept as a fallback for a code we do not recognize yet.
            const text = errorCopy(s, code, message, retryAfterSeconds);
            patch(assistantId, (current) => ({ ...current, pending: false }));
            setMessages((current) => [
              ...current,
              { id: nextId('n'), role: 'system-note', content: text, tone: 'warn' },
            ]);
            // The note explains it in place; the toast makes sure it is noticed even
            // if the panel is scrolled away from the bottom.
            showToast({ kind: 'error', message: text });
            setStreaming(false);
            abortRef.current = null;
          },
        },
        controller.signal
      );
    },
    // `viewLevel` is read at turn start to stamp the message, so it belongs here:
    // a stale capture would give a new turn the PREVIOUS level.
    [messages, patch, showToast, s, streaming, transport, viewLevel]
  );

  // Proposal execution lives in its own hook: a separate concern from streaming a
  // turn, and keeping it here pushed this file past the 500-line limit.
  const { executeProposal, markProposalSucceeded, dismissProposal } = useProposals({
    messages,
    setMessages,
    apiFetch,
    nextId,
  });

  const clearTranscript = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    // Cancel, not flush: the transcript is being discarded, so revealing into a message
    // that is about to disappear would only race the clear.
    typewriterRef.current?.cancel();
    typewriterRef.current = null;
    setStreaming(false);
    setMessages([]);
  }, []);

  const value = useMemo(
    () => ({
      open,
      messages,
      streaming,
      viewLevel,
      setViewLevel,
      openPanel: () => setOpen(true),
      closePanel: () => setOpen(false),
      toggle: () => setOpen((current) => !current),
      sendMessage,
      stopStreaming,
      clearTranscript,
      executeProposal,
      dismissProposal,
      markProposalSucceeded,
    }),
    [
      open,
      messages,
      streaming,
      viewLevel,
      setViewLevel,
      sendMessage,
      stopStreaming,
      clearTranscript,
      executeProposal,
      dismissProposal,
      markProposalSucceeded,
    ]
  );

  return <ChatContext.Provider value={value}>{children}</ChatContext.Provider>;
}

/** Read the chat state. Throws outside a provider, because a silent no-op
 *  context would make a mis-mounted panel look merely broken. */
export function useChat(): ChatContextValue {
  const value = useContext(ChatContext);
  if (value == null) throw new Error('useChat must be used inside a ChatProvider');
  return value;
}
