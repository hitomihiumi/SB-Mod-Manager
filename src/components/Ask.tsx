import { useEffect, useRef, useState } from "react";
import { create } from "zustand";

import { Button, Dialog } from "./ui";

/**
 * The app's own question boxes, in place of the browser's `prompt` and
 * `confirm`: styled like the rest of the window, keyboard-friendly, and
 * awaited like the originals.
 *
 * ```ts
 * const name = await askText({ title: "Rename mod", initial: mod.name });
 * if (await askConfirm({ title: "Uninstall", message: "…", danger: true })) …
 * ```
 */

interface TextQuestion {
  kind: "text";
  title: string;
  label?: string;
  initial?: string;
  placeholder?: string;
  confirmLabel?: string;
  resolve: (value: string | null) => void;
}

interface ConfirmQuestion {
  kind: "confirm";
  title: string;
  message: string;
  confirmLabel?: string;
  danger?: boolean;
  resolve: (value: boolean) => void;
}

type Question = TextQuestion | ConfirmQuestion;

const useAsk = create<{ question: Question | null }>(() => ({ question: null }));

/** Whether a question is showing — keyboard shortcuts stand aside while it is. */
export function isAsking(): boolean {
  return useAsk.getState().question !== null;
}

export function askText(options: Omit<TextQuestion, "kind" | "resolve">): Promise<string | null> {
  return new Promise((resolve) =>
    useAsk.setState({ question: { kind: "text", ...options, resolve } }),
  );
}

export function askConfirm(options: Omit<ConfirmQuestion, "kind" | "resolve">): Promise<boolean> {
  return new Promise((resolve) =>
    useAsk.setState({ question: { kind: "confirm", ...options, resolve } }),
  );
}

/** Rendered once, near the root. */
export function AskHost() {
  const question = useAsk((s) => s.question);
  const [value, setValue] = useState("");
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (question?.kind === "text") {
      setValue(question.initial ?? "");
      // Selected, so typing replaces it — the way a rename box behaves.
      requestAnimationFrame(() => input.current?.select());
    }
  }, [question]);

  if (!question) return null;

  function finish(answer: string | boolean | null) {
    const current = useAsk.getState().question;
    useAsk.setState({ question: null });
    if (!current) return;
    if (current.kind === "text") current.resolve(typeof answer === "string" ? answer : null);
    else current.resolve(answer === true);
  }

  const cancel = () => finish(question.kind === "text" ? null : false);
  const trimmed = value.trim();

  return (
    <Dialog
      open
      title={question.title}
      width="440px"
      onClose={cancel}
      footer={
        <>
          <Button variant="ghost" onClick={cancel}>
            Cancel
          </Button>
          {question.kind === "text" ? (
            <Button variant="primary" disabled={!trimmed} onClick={() => finish(trimmed)}>
              {question.confirmLabel ?? "OK"}
            </Button>
          ) : (
            <Button
              variant={question.danger ? "danger" : "primary"}
              autoFocus
              onClick={() => finish(true)}
            >
              {question.confirmLabel ?? "OK"}
            </Button>
          )}
        </>
      }
    >
      {question.kind === "text" ? (
        <>
          {question.label && (
            <label className="mb-1.5 block text-[12px] font-medium">{question.label}</label>
          )}
          <input
            ref={input}
            value={value}
            placeholder={question.placeholder}
            onChange={(event) => setValue(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && trimmed) finish(trimmed);
            }}
            className="h-9 w-full rounded-md border border-line bg-base px-2.5 text-[13px] focus:border-accent-soft focus:outline-none"
          />
        </>
      ) : (
        <p className="text-[13px] leading-relaxed text-ink-soft whitespace-pre-line">
          {question.message}
        </p>
      )}
    </Dialog>
  );
}
