import { useCallback, useEffect, useState } from "react";
import {
  getPastePermission,
  requestPastePermission,
  type PastePermission,
} from "../backend";

/** The paste permission, read again whenever Scribe's window regains focus
 * (the user grants it in System Settings and comes back). `request` asks for
 * it: on Wayland that waits for the desktop's dialog, so `requesting` covers
 * the wait. */
export function usePastePermission() {
  const [permission, setPermission] = useState<PastePermission | null>(null);
  const [requesting, setRequesting] = useState(false);

  const refresh = useCallback(() => {
    getPastePermission()
      .then(setPermission)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    let active = true;
    const load = () => {
      getPastePermission()
        .then((next) => {
          if (active) setPermission(next);
        })
        .catch(() => undefined);
    };
    load();
    window.addEventListener("focus", load);
    return () => {
      active = false;
      window.removeEventListener("focus", load);
    };
  }, []);

  const request = () => {
    setRequesting(true);
    requestPastePermission()
      .then(setPermission)
      // A declined or failed request leaves the row up; read the new state.
      .catch(refresh)
      .finally(() => setRequesting(false));
  };

  return { permission, request, requesting };
}

/** The Dashboard's name for the way dictations reach other apps, where it is
 * worth naming (Linux); null elsewhere. */
export function insertMethodLabel(
  permission: PastePermission | null,
): string | null {
  switch (permission?.method) {
    case "portal":
      return "Portal paste";
    case "x11":
      return "X11 paste";
    case "copyOnly":
      return "Copy only";
    default:
      return null;
  }
}
