import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  getReplacements,
  REPLACEMENTS_CHANGED_EVENT,
  type ReplacementsInfo,
} from "../backend";

/** replacements.txt as Scribe last read it: loaded on mount, refreshed when
 * the file changes (the backend watches it) and when the window regains
 * focus. `set` takes the contents a write returned. */
export function useReplacements(): {
  info: ReplacementsInfo | null;
  set: (info: ReplacementsInfo) => void;
  refresh: () => void;
} {
  const [info, setInfo] = useState<ReplacementsInfo | null>(null);
  const refresh = useCallback(() => {
    getReplacements()
      .then(setInfo)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    refresh();
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void listen<ReplacementsInfo>(REPLACEMENTS_CHANGED_EVENT, (event) =>
      setInfo(event.payload),
    ).then((stop) => {
      if (disposed) {
        stop();
      } else {
        unlisten = stop;
      }
    });
    window.addEventListener("focus", refresh);
    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener("focus", refresh);
    };
  }, [refresh]);

  return { info, set: setInfo, refresh };
}
