import { useCallback, useEffect, useRef, useState } from "react";
import type { Entry } from "../lib/conversation";
import { afterResize, afterScroll, initialFollow, isOwnScroll, jumpToLatest, type FollowState } from "../lib/scroll";
import type { Jarvis } from "../lib/useJarvis";
import { RunView } from "./RunView";

function EntryView({ entry, jarvis, compact }: { entry: Entry; jarvis: Jarvis; compact?: boolean }) {
  switch (entry.type) {
    case "user":
      return <div className="msg msg--user">{entry.text}</div>;
    case "pending":
      return null;
    case "assistant":
      return (
        <div className={`msg msg--assistant msg--${entry.status}`}>
          {entry.text || (entry.status === "streaming" ? <span className="typing" aria-label="Esperando respuesta" /> : null)}
          {entry.status === "cancelled" && <p className="note">Respuesta interrumpida.</p>}
          {entry.status === "error" && entry.error && (
            <div className="callout callout--error" role="alert">
              <p>{entry.error.message}</p>
              <p className="hint">{entry.error.recovery}</p>
              <button className="btn" onClick={() => void jarvis.retry(entry.sourceText)}>
                Reintentar
              </button>
            </div>
          )}
          {entry.status === "streaming" && (
            <button className="btn btn--ghost btn--small" onClick={() => void jarvis.cancel()}>
              Detener
            </button>
          )}
        </div>
      );
    case "help":
      return <pre className="msg msg--system help">{entry.text}</pre>;
    case "parse_error":
      return (
        <div className="callout callout--warn" role="alert">
          <p>{entry.error.message}</p>
          <p className="hint">{entry.error.hint}</p>
        </div>
      );
    case "not_available":
      return (
        <div className="callout callout--info">
          <p>
            <span className="tag">Fase {entry.phase}</span> {entry.reason}
          </p>
          <p className="hint">{entry.alternative}</p>
        </div>
      );
    case "no_model":
      return (
        <div className="callout callout--error" role="alert">
          <p>{entry.message}</p>
          <p className="hint">{entry.recovery}</p>
          <button className="btn" onClick={() => void jarvis.retry(entry.sourceText)}>
            Reintentar
          </button>
        </div>
      );
    case "run":
      return (
        <RunView
          run={entry.run}
          compact={compact}
          onCancel={() => void jarvis.cancel()}
          onRetry={() => void jarvis.retry(entry.sourceText)}
        />
      );
  }
}

export function ChatLog({ jarvis, compact, emptyHint }: { jarvis: Jarvis; compact?: boolean; emptyHint?: React.ReactNode }) {
  const scroller = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const follow = useRef<FollowState>(initialFollow);
  const pinnedTop = useRef<number | null>(null);
  const [unseen, setUnseen] = useState(false);

  const apply = useCallback((next: { state: FollowState; pin: boolean }) => {
    follow.current = next.state;
    setUnseen(next.state.unseen);
    const el = scroller.current;
    if (next.pin && el) {
      el.scrollTop = el.scrollHeight;
      pinnedTop.current = el.scrollTop;
    }
  }, []);

  // Size changes cover streaming text, reports, opened <details> and window
  // resizes. Pin to the bottom only if the user was already following.
  useEffect(() => {
    const el = scroller.current;
    const inner = content.current;
    if (!el || !inner) return;
    let lastHeight = inner.offsetHeight;
    const ro = new ResizeObserver(() => {
      const h = inner.offsetHeight;
      apply(afterResize(follow.current, h > lastHeight));
      lastHeight = h;
    });
    ro.observe(inner);
    ro.observe(el);
    return () => ro.disconnect();
  }, [apply]);

  // A message sent by the user always brings the view to the end.
  const lastUserId = [...jarvis.entries].reverse().find((e) => e.type === "user")?.id;
  useEffect(() => {
    if (lastUserId) apply(jumpToLatest());
  }, [lastUserId, apply]);

  const onScroll = () => {
    const el = scroller.current;
    if (!el || isOwnScroll(el.scrollTop, pinnedTop.current)) return;
    pinnedTop.current = null;
    follow.current = afterScroll(follow.current, el);
    setUnseen(follow.current.unseen);
  };

  return (
    <div className="chatlog-wrap">
      <div
        ref={scroller}
        className="chatlog"
        role="log"
        aria-live="polite"
        aria-relevant="additions"
        aria-label="Conversación"
        tabIndex={0}
        onScroll={onScroll}
      >
        <div ref={content} className="chatlog__content">
          {jarvis.entries.length === 0 && emptyHint}
          {jarvis.entries.map((e) => (
            <EntryView key={e.id} entry={e} jarvis={jarvis} compact={compact} />
          ))}
        </div>
      </div>
      {unseen && (
        <button className="btn btn--primary jump-latest" onClick={() => apply(jumpToLatest())}>
          ↓ Mensajes nuevos
        </button>
      )}
    </div>
  );
}
