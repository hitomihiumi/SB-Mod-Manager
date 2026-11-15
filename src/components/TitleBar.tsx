import { Minus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import iconUrl from "../../src-tauri/icons/32x32.png";

/** The window is frameless, so the chrome is ours to draw. */
export function TitleBar() {
  return (
    <header data-tauri-drag-region className="drag-region glass-bar flex h-9 shrink-0 items-center justify-between pl-5">
      <span data-tauri-drag-region className="flex items-center text-[12px] text-ink">
        <img src={iconUrl} alt="" width={20} height={20} draggable={false} className="mr-2 size-5 rounded-[4px]" />
        Stelle
      </span>
      <div className="no-drag flex h-full">
        <WindowButton onClick={() => void getCurrentWindow().minimize()} label="Minimise">
          <Minus size={14} />
        </WindowButton>
        <WindowButton onClick={() => void getCurrentWindow().toggleMaximize()} label="Maximise">
          <Square size={11} />
        </WindowButton>
        <WindowButton onClick={() => void getCurrentWindow().close()} label="Close" danger>
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
        "grid h-full w-11 place-items-center text-ink-muted transition-colors duration-150 ease-quiet " +
        (danger ? "hover:bg-danger/15 hover:text-danger" : "hover:bg-white/8 hover:text-ink")
      }
    >
      {children}
    </button>
  );
}
