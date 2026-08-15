import { Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** The window is frameless, so the chrome is ours to draw. */
export function TitleBar() {
  const win = getCurrentWindow();
  return (
    <header data-tauri-drag-region className="drag-region flex h-9 shrink-0 items-center justify-between border-b border-line bg-surface pl-3">
      <div data-tauri-drag-region className="flex items-center gap-2">
        <div className="size-3.5 rounded-[3px] bg-gradient-to-b from-accent to-accent-soft" />
        <span className="text-[12px] font-semibold tracking-tight text-ink-soft">
          SB Mod Manager
        </span>
      </div>
      <div className="no-drag flex h-full">
        <WindowButton onClick={() => win.minimize()} label="Minimise">
          <Minus size={14} />
        </WindowButton>
        <WindowButton onClick={() => win.toggleMaximize()} label="Maximise">
          <Square size={11} />
        </WindowButton>
        <WindowButton onClick={() => win.close()} label="Close" danger>
          <X size={14} />
        </WindowButton>
      </div>
    </header>
  );
}

function WindowButton({
  children,
  onClick,
  label,
  danger,
}: {
  children: React.ReactNode;
  onClick: () => void;
  label: string;
  danger?: boolean;
}) {
  return (
    <button
      aria-label={label}
      onClick={onClick}
      className={
        "grid h-full w-11 place-items-center text-ink-muted transition-colors " +
        (danger ? "hover:bg-danger hover:text-white" : "hover:bg-hover hover:text-ink")
      }
    >
      {children}
    </button>
  );
}
