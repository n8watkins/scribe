import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import type { ViewName } from "../types";
import { searchSettings, type SettingsEntry } from "../lib/settingsSearch";

const HIGHLIGHT_MS = 1800;

/** Once the page has rendered, scrolls the row or panel tagged
 * `data-setting="<target>"` into view and flashes it. A row inside a closed
 * "Advanced" disclosure opens it first. Gives up quietly if the page never
 * shows the target (for example a panel that only appears when a feature is
 * on). */
function highlightSetting(target: string) {
  let attempts = 0;
  const tryHighlight = () => {
    const element = document.querySelector<HTMLElement>(
      `main [data-setting="${CSS.escape(target)}"]`,
    );
    if (!element) {
      attempts += 1;
      if (attempts < 10) {
        window.setTimeout(tryHighlight, 60);
      }
      return;
    }
    const disclosure = element.closest("details");
    if (disclosure && !disclosure.open) {
      disclosure.open = true;
    }
    element.scrollIntoView({ block: "center", behavior: "smooth" });
    element.classList.remove("search-hit");
    // Restart the flash if the same row is picked twice in a row.
    void element.offsetWidth;
    element.classList.add("search-hit");
    window.setTimeout(() => element.classList.remove("search-hit"), HIGHLIGHT_MS);
  };
  window.setTimeout(tryHighlight, 60);
}

/** Sidebar search over every setting. Ctrl+K focuses it from anywhere;
 * arrows move through results, Enter jumps to the page and highlights the
 * setting, Escape clears. */
export function SettingsSearch({
  onNavigate,
}: {
  onNavigate: (view: ViewName) => void;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const results = useMemo(() => searchSettings(query), [query]);
  const open = query.trim().length > 0;

  const choose = (entry: SettingsEntry) => {
    onNavigate(entry.page);
    highlightSetting(entry.target ?? entry.label);
    setQuery("");
    inputRef.current?.blur();
  };

  return (
    <div className="settings-search">
      <label className="settings-search-field">
        <Search aria-hidden="true" size={14} />
        <input
          aria-activedescendant={
            open && results[active] ? `${listId}-${active}` : undefined
          }
          aria-autocomplete="list"
          aria-controls={listId}
          aria-expanded={open}
          aria-label="Search settings"
          onBlur={() => {
            // Close the list when focus leaves, unless it came straight back
            // (Ctrl+K again) before the timer ran.
            window.setTimeout(() => {
              if (document.activeElement !== inputRef.current) {
                setQuery("");
              }
            }, 150);
          }}
          onChange={(event) => {
            setQuery(event.currentTarget.value);
            setActive(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setActive((i) => Math.min(i + 1, results.length - 1));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((i) => Math.max(i - 1, 0));
            } else if (event.key === "Enter" && results[active]) {
              event.preventDefault();
              choose(results[active]);
            } else if (event.key === "Escape") {
              setQuery("");
              event.currentTarget.blur();
            }
          }}
          placeholder="Search"
          ref={inputRef}
          role="combobox"
          spellCheck={false}
          value={query}
        />
        <kbd aria-hidden="true">Ctrl K</kbd>
      </label>
      {open ? (
        <div className="settings-search-results" id={listId} role="listbox">
          {results.length === 0 ? (
            <div className="settings-search-empty">No setting matches</div>
          ) : (
            results.map((entry, index) => (
              <div
                aria-selected={index === active}
                className={
                  index === active
                    ? "settings-search-result is-active"
                    : "settings-search-result"
                }
                id={`${listId}-${index}`}
                key={`${entry.page}-${entry.label}`}
                onMouseDown={(event) => {
                  // Keep focus in the input so blur doesn't close the list first.
                  event.preventDefault();
                  choose(entry);
                }}
                onMouseEnter={() => setActive(index)}
                role="option"
              >
                <span>{entry.label}</span>
                <small>{entry.page}</small>
              </div>
            ))
          )}
        </div>
      ) : null}
    </div>
  );
}
