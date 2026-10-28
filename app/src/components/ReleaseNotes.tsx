import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Sparkles, X } from "lucide-react";
import {
  dismissWhatsNew,
  openReleasePage,
  whatsNewPending,
} from "../backend";
import {
  bundledNotes,
  parseInline,
  parseNotes,
  shortVersion,
} from "../lib/releaseNotes";
import "./release-notes.css";

const RELEASES_URL = "https://github.com/n8watkins/scribe/releases";

function InlineNotes({ text }: { text: string }) {
  return (
    <>
      {parseInline(text).map((piece, index) => {
        switch (piece.kind) {
          case "strong":
            return <strong key={index}>{piece.text}</strong>;
          case "code":
            return <code key={index}>{piece.text}</code>;
          case "link":
            return (
              <button
                className="release-notes-link"
                key={index}
                onClick={() => void openReleasePage(piece.url).catch(() => {})}
                title={piece.url}
                type="button"
              >
                {piece.text}
              </button>
            );
          default:
            return <span key={index}>{piece.text}</span>;
        }
      })}
    </>
  );
}

/** Renders release notes (a CHANGELOG section or a GitHub release body) as
 * plain React elements; nothing in the notes is treated as HTML. */
export function ReleaseNotesBody({
  markdown,
  className,
}: {
  markdown: string;
  className?: string;
}) {
  return (
    <div className={`release-notes${className ? ` ${className}` : ""}`}>
      {parseNotes(markdown).map((block, index) => {
        switch (block.kind) {
          case "heading":
            return (
              <h4 className="release-notes-heading" key={index}>
                <InlineNotes text={block.text} />
              </h4>
            );
          case "list":
            return (
              <ul key={index}>
                {block.items.map((item, itemIndex) => (
                  <li key={itemIndex}>
                    <InlineNotes text={item} />
                  </li>
                ))}
              </ul>
            );
          default:
            return (
              <p key={index}>
                <InlineNotes text={block.text} />
              </p>
            );
        }
      })}
    </div>
  );
}

/** The release notes of the running version, from the changelog bundled in
 * the app, with a link to every release on GitHub. */
export function CurrentReleaseNotes() {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => setVersion(""));
  }, []);
  if (version === null) {
    return null;
  }
  const notes = version ? bundledNotes(version) : null;
  return (
    <div className="release-notes-current">
      {notes ? (
        <ReleaseNotesBody markdown={notes} />
      ) : (
        <p className="release-notes-empty">
          This build has no release notes. Every release's notes are on GitHub.
        </p>
      )}
      <button
        className="ghost-button release-notes-all"
        onClick={() => void openReleasePage(RELEASES_URL).catch(() => {})}
        type="button"
      >
        All release notes
      </button>
    </div>
  );
}

/** A dismissible "What's new in X.Y" card, shown once after an update. */
export function WhatsNewCard() {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    void whatsNewPending()
      .then((pending) => {
        if (!cancelled) {
          setVersion(pending);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const notes = version ? bundledNotes(version) : null;
  if (!version || !notes) {
    return null;
  }

  const dismiss = () => {
    setVersion(null);
    void dismissWhatsNew(version).catch(() => {});
  };

  return (
    <section
      aria-labelledby="whats-new-title"
      className="panel-card whats-new-card"
    >
      <header className="whats-new-head">
        <span className="whats-new-icon" aria-hidden="true">
          <Sparkles size={16} />
        </span>
        <h3 id="whats-new-title">What's new in {shortVersion(version)}</h3>
        <button
          aria-label="Dismiss what's new"
          className="icon-button whats-new-dismiss"
          onClick={dismiss}
          type="button"
        >
          <X aria-hidden="true" size={15} />
        </button>
      </header>
      <ReleaseNotesBody className="whats-new-notes" markdown={notes} />
      <div className="whats-new-actions">
        <button className="secondary-button" onClick={dismiss} type="button">
          Got it
        </button>
      </div>
    </section>
  );
}
