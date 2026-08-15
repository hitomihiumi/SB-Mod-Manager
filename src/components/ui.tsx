/**
 * The handful of primitives the app needs.
 *
 * Hand-rolled rather than pulled from a component library: the surface used
 * here is small, and the whole point of the project is a light footprint.
 */
import clsx from "clsx";
import { Check, Minus, X } from "lucide-react";
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
        "no-drag inline-flex items-center justify-center gap-1.5 rounded-md font-medium",
        "transition-colors disabled:pointer-events-none disabled:opacity-40",
        size === "sm" ? "h-7 px-2.5 text-[12px]" : "h-8 px-3 text-[13px]",
        variant === "primary" &&
          "bg-accent text-accent-ink hover:bg-accent/85 active:bg-accent/70",
        variant === "ghost" && "text-ink-soft hover:bg-hover hover:text-ink",
        variant === "outline" &&
          "border border-line-strong text-ink-soft hover:border-ink-muted hover:text-ink",
        variant === "danger" && "text-danger hover:bg-danger/10",
        className,
      )}
      {...props}
    />
  );
}

/** A checkbox that can also show a partial state for a group. */
export function Checkbox({
  checked,
  indeterminate = false,
  onChange,
  label,
  className,
}: {
  checked: boolean;
  indeterminate?: boolean;
  onChange: (next: boolean) => void;
  label?: string;
  className?: string;
}) {
  const active = checked || indeterminate;
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
      className={clsx(
        "grid size-4 shrink-0 place-items-center rounded border transition-colors",
        active
          ? "border-accent bg-accent text-accent-ink"
          : "border-line-strong hover:border-ink-muted",
        className,
      )}
    >
      {indeterminate ? (
        <Minus size={11} strokeWidth={3} />
      ) : checked ? (
        <Check size={11} strokeWidth={3} />
      ) : null}
    </button>
  );
}

export function Badge({
  children,
  tone = "neutral",
  title,
}: {
  children: ReactNode;
  tone?: "neutral" | "accent" | "warn" | "danger";
  title?: string;
}) {
  return (
    <span
      title={title}
      className={clsx(
        "inline-flex h-[18px] items-center rounded px-1.5 text-[10px] font-semibold tracking-wide uppercase",
        tone === "neutral" && "bg-hover text-ink-muted",
        tone === "accent" && "bg-accent-soft/15 text-accent-soft",
        tone === "warn" && "bg-warn/15 text-warn",
        tone === "danger" && "bg-danger/15 text-danger",
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
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/55 p-6">
      <div
        className="flex max-h-full w-full flex-col overflow-hidden rounded-xl border border-line-strong bg-surface shadow-2xl"
        style={{ maxWidth: width }}
      >
        <header className="flex h-11 shrink-0 items-center justify-between border-b border-line px-4">
          <h2 className="text-[13px] font-semibold">{title}</h2>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="Close">
            <X size={14} />
          </Button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3.5">{children}</div>
        {footer && (
          <footer className="flex shrink-0 items-center justify-end gap-2 border-t border-line px-4 py-3">
            {footer}
          </footer>
        )}
      </div>
    </div>
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
    <div className="grid h-full place-items-center px-8 text-center">
      <div className="max-w-sm">
        <div className="mx-auto mb-3 grid size-11 place-items-center rounded-full bg-raised text-ink-muted">
          {icon}
        </div>
        <p className="text-[14px] font-semibold">{title}</p>
        <p className="mt-1.5 text-[12px] leading-relaxed text-ink-muted">{hint}</p>
        {action && <div className="mt-4 flex justify-center">{action}</div>}
      </div>
    </div>
  );
}
