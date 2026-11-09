import { useState } from "react";
import { AlertTriangle, Check, Copy, Settings2 } from "lucide-react";
import {
  bindPortalShortcuts,
  commandErrorMessage,
  configurePortalShortcuts,
  type HotkeyStatus,
} from "../backend";
import { currentPlatform } from "../lib/platform";

/** The actions a desktop keyboard shortcut can run, and their commands. */
const COMMANDS = [
  { action: "toggle", label: "Start or stop dictation" },
  { action: "paste-last", label: "Paste the last dictation" },
  { action: "cancel", label: "Cancel the recording" },
  { action: "show", label: "Show Scribe" },
] as const;

/** The full command line for one action. */
function controlCommandLine(command: string, action: string): string {
  return `${command} ${action}`;
}

function CopyCommand({ line }: { line: string }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(line);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };
  return (
    <div className="wayland-command">
      <code>{line}</code>
      <button
        aria-label={copied ? "Copied" : `Copy ${line}`}
        className="ghost-button wayland-command-copy"
        onClick={() => void copy()}
        type="button"
      >
        {copied ? (
          <Check aria-hidden="true" size={13} />
        ) : (
          <Copy aria-hidden="true" size={13} />
        )}
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

/** How to bind Scribe's command in the desktop's keyboard settings. */
function CommandShortcuts({
  command,
  open,
}: {
  command: string;
  open: boolean;
}) {
  return (
    <details className="wayland-commands" open={open}>
      <summary>Use a shortcut from your desktop's keyboard settings</summary>
      <p className="hotkey-note">
        Add a custom shortcut that runs one of these commands. Scribe has to be
        running; the command hands the action to it.
      </p>
      <div className="wayland-command-list">
        {COMMANDS.map(({ action, label }) => (
          <div className="wayland-command-row" key={action}>
            <span>{label}</span>
            <CopyCommand line={controlCommandLine(command, action)} />
          </div>
        ))}
      </div>
      <ul className="wayland-steps">
        <li>
          <strong>GNOME:</strong> Settings, Keyboard, View and Customize
          Shortcuts, Custom Shortcuts, then Add Shortcut. Paste the command and
          press the keys, for example Ctrl+Alt+`.
        </li>
        <li>
          <strong>KDE Plasma:</strong> System Settings, Keyboard, Shortcuts, Add
          New, then Command or Script. Paste the command and choose the keys.
        </li>
        <li>
          Hold to talk needs the key release too, which only some desktops can
          bind (sway and Hyprland <code>bindr</code>): use{" "}
          <code>hold-start</code> on press and <code>hold-stop</code> on
          release.
        </li>
      </ul>
    </details>
  );
}

/** Wayland only: who delivers the hotkeys (the desktop's GlobalShortcuts
 * portal, or X11 grabs that only fire in X11 apps) and how to bind Scribe's
 * command where the portal can't help. */
export function WaylandShortcutHelp({
  onStatus,
  status,
}: {
  onStatus: (status: HotkeyStatus) => void;
  status: HotkeyStatus;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const command = status.controlCommand;

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (failure) {
      setError(commandErrorMessage(failure));
    } finally {
      setBusy(false);
    }
  };

  // Wayland guidance exists only on Linux. The backend never reports a
  // Wayland backend elsewhere; this guards the page against any status that
  // does.
  if (currentPlatform() !== "linux") {
    return null;
  }

  if (status.backend === "x11OnWayland") {
    return (
      <div className="wayland-shortcuts">
        <p className="wayland-shortcuts-warning" role="status">
          <AlertTriangle aria-hidden="true" size={14} />
          Your desktop has no global shortcuts portal, so these hotkeys only
          work while an X11 app is in front. Bind Scribe's command to keys in
          your desktop's settings to use them everywhere.
        </p>
        {command ? <CommandShortcuts command={command} open /> : null}
      </div>
    );
  }

  if (status.backend !== "portal") {
    return null;
  }

  const state = status.portalState;
  return (
    <div className="wayland-shortcuts">
      {state === "starting" ? (
        <p className="hotkey-note" role="status">
          Your desktop is asking which keys to use. Answer its dialog to turn
          these hotkeys on.
        </p>
      ) : null}
      {state === "declined" ? (
        <div className="wayland-shortcuts-action">
          <p className="wayland-shortcuts-warning" role="status">
            <AlertTriangle aria-hidden="true" size={14} />
            The desktop's shortcut dialog was cancelled, so these hotkeys are
            off.
          </p>
          <button
            className="secondary-button"
            disabled={busy}
            onClick={() =>
              void run(async () => onStatus(await bindPortalShortcuts()))
            }
            type="button"
          >
            {busy ? "Waiting for your desktop..." : "Set up shortcuts"}
          </button>
        </div>
      ) : null}
      {state === "active" ? (
        <div className="wayland-shortcuts-action">
          <p className="hotkey-note">
            Your desktop delivers these shortcuts and keeps them across
            restarts.{" "}
            {status.portalConfigurable
              ? "Change the keys in its settings."
              : "Change the keys in your desktop's keyboard settings, under Scribe."}
          </p>
          {status.portalConfigurable ? (
            <button
              className="secondary-button"
              disabled={busy}
              onClick={() => void run(configurePortalShortcuts)}
              type="button"
            >
              <Settings2 aria-hidden="true" size={13} />
              Change keys
            </button>
          ) : null}
        </div>
      ) : null}
      {error ? (
        <p className="hotkeys-row-error" role="alert">
          <AlertTriangle aria-hidden="true" size={13} />
          {error}
        </p>
      ) : null}
      {command ? (
        <CommandShortcuts command={command} open={state === "declined"} />
      ) : null}
    </div>
  );
}
