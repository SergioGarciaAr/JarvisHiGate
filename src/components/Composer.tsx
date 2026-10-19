import { useEffect, useRef, useState } from "react";
import type { Jarvis } from "../lib/useJarvis";

interface Props {
  jarvis: Jarvis;
  autoFocus?: boolean;
  onEscape?: () => void;
}

export function Composer({ jarvis, autoFocus, onEscape }: Props) {
  const [text, setText] = useState("");
  const history = useRef<string[]>([]);
  const cursor = useRef(-1);
  const input = useRef<HTMLTextAreaElement>(null);
  const busy = !!jarvis.pending;

  useEffect(() => {
    if (autoFocus) input.current?.focus();
  }, [autoFocus]);

  const send = () => {
    const value = text.trim();
    if (!value || busy) return;
    history.current.unshift(value);
    cursor.current = -1;
    setText("");
    void jarvis.submit(value);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    } else if (e.key === "Escape") {
      if (busy) void jarvis.cancel();
      else onEscape?.();
    } else if (e.key === "ArrowUp" && !text.includes("\n") && history.current.length > 0) {
      e.preventDefault();
      cursor.current = Math.min(cursor.current + 1, history.current.length - 1);
      setText(history.current[cursor.current]);
    } else if (e.key === "ArrowDown" && cursor.current >= 0) {
      e.preventDefault();
      cursor.current -= 1;
      setText(cursor.current >= 0 ? history.current[cursor.current] : "");
    }
  };

  return (
    <form
      className="composer"
      onSubmit={(e) => {
        e.preventDefault();
        send();
      }}
    >
      <label htmlFor="composer-input" className="sr-only">
        Mensaje o comando para Jarvis
      </label>
      <span className="composer__prompt" aria-hidden="true">
        ›
      </span>
      <textarea
        id="composer-input"
        ref={input}
        rows={1}
        value={text}
        placeholder="Escribe un mensaje, «jarvis ayuda» o «-- objetivo»"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        spellCheck={false}
        autoComplete="off"
      />
      {busy ? (
        <button type="button" className="btn btn--danger" onClick={() => void jarvis.cancel()}>
          Detener
        </button>
      ) : (
        <button type="submit" className="btn btn--primary" disabled={!text.trim()}>
          Enviar
        </button>
      )}
    </form>
  );
}
