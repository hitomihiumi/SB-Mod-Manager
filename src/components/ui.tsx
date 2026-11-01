/**
 * The handful of primitives the app needs.
 *
 * Hand-rolled rather than pulled from a component library: the surface used
 * here is small, and the whole point of the project is a light footprint.
 *
 * The look is deliberately quiet: no fills in the accent colour, no borders
 * where a change of tone says the same thing, and state shown by brightness.
 * Everything that moves does so on the same curve (`ease-quiet`).
 */
import clsx from "clsx";
import { X } from "lucide-react";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { useEffect } from "react";

export function Button({
  variant = "ghost",
  size = "md",
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "ghost" | "outline" | "danger";
  size?: "sm" | "md";
}) {
  return (
    <button
      className={clsx(
        "no-drag inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-sm font-normal",
        "transition-[color,background-color,text-decoration-color,opacity] duration-150 ease-quiet disabled:pointer-events-none disabled:opacity-40",
        size === "sm" ? "h-7 px-2.5 text-[12px]" : "h-8 px-3 text-[13px]",
        // The main action is a word with an underline, not a coloured block.
        variant === "primary" &&
          "text-ink underline decoration-ink-muted decoration-1 underline-offset-[5px] hover:decoration-ink active:opacity-70",
        variant === "ghost" && "text-ink-soft hover:bg-raised hover:text-ink",
        variant === "outline" && "bg-raised text-ink-soft hover:bg-hover hover:text-ink",
        variant === "danger" && "text-danger hover:bg-danger/10",
        className,
      )}
      {...props}
    />
  );
}

/**
 * A dot rather than a box: filled is on, a ring is off, half filled is a
 * partly enabled group, and a ring around the dot means the change is waiting
 * for Apply. It keeps the checkbox role and keyboard behaviour.
 */
export function Checkbox({
  checked,
  indeterminate = false,
  pending = false,
  onChange,
  label,
  className,
}: {
  checked: boolean;
  indeterminate?: boolean;
  /** The state shown is not on disk yet. */
  pending?: boolean;
  onChange: (next: boolean) => void;
  label?: string;
  className?: string;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={indeterminate ? "mixed" : checked}
      aria-label={label}
      onClick={(event) => {
        event.stopPropagation();
        onChange(!checked);
      }}
      className={clsx("group/dot grid size-[22px] shrink-0 place-items-center", className)}
    >
      <span
        className={clsx(
          "size-[9px] rounded-full border outline-1 transition-[background-color,border-color,outline-color,outline-offset,transform] duration-200 ease-quiet group-active/dot:scale-75",
          pending ? "outline-accent outline-offset-[3px]" : "outline-transparent outline-offset-0",
          checked
            ? "border-ink bg-ink"
            : indeterminate
              ? "border-ink bg-[linear-gradient(90deg,var(--color-ink)_50%,transparent_50%)]"
              : "border-ink-muted group-hover/dot:border-ink-soft",
        )}
      />
    </button>
  );
}

/** A word of state. No box: colour says how much attention it wants. */
export function Badge({
  children,
  tone = "neutral",
  title,
}: {
  children: ReactNode;
  tone?: "neutral" | "accent" | "warn" | "danger" | "pending";
  title?: string;
}) {
  return (
    <span
      title={title}
      className={clsx(
        "inline-flex items-center text-[12px] whitespace-nowrap",
        tone === "neutral" && "text-ink-muted",
        tone === "accent" && "text-ink-soft",
        tone === "warn" && "text-warn",
        tone === "danger" && "text-danger",
        tone === "pending" && "text-accent",
      )}
    >
      {children}
    </span>
  );
}

export function Dialog({
  open,
  title,
  onClose,
  children,
  footer,
  width = "560px",
}: {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: string;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 grid animate-fade place-items-center bg-base/75 p-6">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="flex max-h-full w-full animate-rise flex-col overflow-hidden bg-hover"
        style={{ maxWidth: width }}
      >
        <header className="flex h-12 shrink-0 items-center justify-between pr-3 pl-5">
          <h2 className="text-[15px] font-light">{title}</h2>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="Close">
            <X size={14} />
          </Button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-3.5">{children}</div>
        {footer && (
          <footer className="flex shrink-0 items-center justify-end gap-3 px-5 py-4">
            {footer}
          </footer>
        )}
      </div>
    </div>
  );
}

/** One block of the settings screen. */
export function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint: string;
  children: ReactNode;
}) {
  return (
    <section className="bg-surface p-5">
      <h3 className="text-[14px] font-normal">{title}</h3>
      {hint && <p className="mt-1 mb-3.5 text-[12px] text-ink-muted">{hint}</p>}
      {children}
    </section>
  );
}

export function Empty({
  icon,
  title,
  hint,
  action,
}: {
  icon: ReactNode;
  title: string;
  hint: string;
  action?: ReactNode;
}) {
  return (
    <div className="grid h-full animate-rise place-items-center px-8 text-center">
      <div className="max-w-sm">
        <div className="mx-auto mb-3 grid size-11 place-items-center rounded-full bg-surface text-ink-muted">
          {icon}
        </div>
        <p className="text-[15px] font-light">{title}</p>
        <p className="mt-1.5 text-[12px] leading-relaxed text-ink-muted">{hint}</p>
        {action && <div className="mt-4 flex justify-center">{action}</div>}
      </div>
    </div>
  );
}
