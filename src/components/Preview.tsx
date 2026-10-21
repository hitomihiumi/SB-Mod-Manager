import clsx from "clsx";
import { ImageOff } from "lucide-react";
import { useEffect, useState } from "react";

import { ipc } from "../lib/ipc";

/** Pictures already read this session, so moving between options is instant. */
const cache = new Map<string, Promise<string>>();

function load(folder: string, path: string): Promise<string> {
  const key = `${folder}\u0000${path}`;
  let pending = cache.get(key);
  if (!pending) {
    pending = ipc.previewImage(folder, path);
    // A failure is not cached, so a picture that appears later is shown.
    pending.catch(() => cache.delete(key));
    cache.set(key, pending);
  }
  return pending;
}

/**
 * A picture from a staged archive or an installed mod. The window has no
 * access to the file system, so the manager reads it and hands it over.
 */
export function Preview({
  folder,
  path,
  alt,
  className,
}: {
  folder: string;
  path: string | null;
  alt: string;
  className?: string;
}) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let live = true;
    setSrc(null);
    setFailed(false);
    if (path) {
      load(folder, path)
        .then((url) => live && setSrc(url))
        .catch(() => live && setFailed(true));
    }
    return () => {
      live = false;
    };
  }, [folder, path]);

  if (!path || failed) {
    return (
      <div
        className={clsx(
          "grid place-items-center rounded-md border border-line bg-base text-ink-muted",
          className,
        )}
      >
        <ImageOff size={18} />
      </div>
    );
  }
  if (!src) {
    return <div className={clsx("animate-pulse rounded-md bg-hover", className)} />;
  }
  return (
    <img
      src={src}
      alt={alt}
      className={clsx("rounded-md border border-line bg-base object-contain", className)}
    />
  );
}

/** A strip of an archive's pictures, the selected one large above it. */
export function Gallery({ folder, paths }: { folder: string; paths: string[] }) {
  const [current, setCurrent] = useState(0);
  if (paths.length === 0) return null;
  const shown = paths[Math.min(current, paths.length - 1)] ?? null;
  return (
    <div className="mb-4 space-y-2">
      <Preview folder={folder} path={shown} alt={shown ?? ""} className="h-48 w-full" />
      {paths.length > 1 && (
        <div className="flex gap-1.5 overflow-x-auto pb-1">
          {paths.map((path, index) => (
            <button
              key={path}
              onClick={() => setCurrent(index)}
              title={path}
              className={clsx(
                "shrink-0 rounded-md border-2",
                index === current ? "border-accent" : "border-transparent",
              )}
            >
              <Preview folder={folder} path={path} alt={path} className="size-14" />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
