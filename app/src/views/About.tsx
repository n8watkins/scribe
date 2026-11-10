import { useEffect, useState, type ReactNode } from "react";
import { getVersion as getAppVersion } from "@tauri-apps/api/app";
import { forPlatform } from "../lib/platform";
import {
  ArrowUpRight,
  Check,
  Clock,
  History,
  Info,
  Keyboard,
  ListChecks,
  Mic,
  Replace,
  ScrollText,
  StickyNote,
} from "lucide-react";
import type { ViewName } from "../types";
import { ReleaseHistory } from "../components/ReleaseNotes";
import { CopyDiagnostics } from "../components/CopyDiagnostics";
import "./about.css";

const FEATURES = [
  {
    Icon: Keyboard,
    title: "Dictate anywhere",
    text: "Hold the hotkey, talk, let go. The text lands at your cursor, in any app.",
  },
  {
    Icon: Mic,
    title: "Runs on your computer",
    text: "Whisper or Parakeet transcribes on this machine. Your audio never leaves it, and dictation works offline.",
  },
  {
    Icon: Clock,
    title: "Paste it again",
    text: "Your last dictation stays one hotkey away. Scribe puts your clipboard back after every paste.",
  },
  {
    Icon: StickyNote,
    title: "Quick notes",
    text: "Hold the toggle key and tap Q to save a spoken note instead of pasting it. Local AI can sum it up and pull out the to-dos.",
  },
  {
    Icon: History,
    title: "Searchable history",
    text: "Every dictation is kept on this device, searchable by text and date, for as long as you choose.",
  },
  {
    Icon: Replace,
    title: "Your words",
    text: "A context hint teaches Scribe your names and jargon; replacements fix the mistakes it keeps making.",
  },
];

const SETUP_STEPS: {
  title: string;
  text: string;
  action: string;
  target: ViewName;
}[] = [
  {
    title: "Download a speech model",
    text: "Pick one on Models and language. Base English is small and quick; Small English and Parakeet v3 get more of your words right.",
    action: "Open Models and language",
    target: "Models & language",
  },
  {
    title: "Pick your microphone",
    text: "Choose it on Microphone and record a short test to hear yourself back.",
    action: "Open Microphone",
    target: "Microphone",
  },
  {
    title: "Try your hotkeys",
    text: "Check hold to talk and paste last on Hotkeys, then click into any text box, hold the key and talk.",
    action: "Open Hotkeys",
    target: "Hotkeys",
  },
];

export function AboutView({
  setActiveView,
}: {
  setActiveView?: (view: ViewName) => void;
}) {
  const [version, setVersion] = useState("...");
  useEffect(() => {
    void getAppVersion().then(setVersion).catch(() => setVersion("unknown"));
  }, []);

  const tabs: {
    id: string;
    title: string;
    icon: ReactNode;
    render: () => ReactNode;
  }[] = [
    {
      id: "app-details",
      title: "App details",
      icon: <Info aria-hidden="true" size={16} />,
      render: () => (
        <article className="panel-card">
          <div className="settings-list">
          <p className="about-lead">
            {forPlatform({
              windows: "Scribe is a Windows tray utility for private speech-to-text.",
              macos: "Scribe is a macOS menu bar utility for private speech-to-text.",
              linux: "Scribe is a Linux tray utility for private speech-to-text.",
            })}
          </p>

          <div className="about-block">
            <div className="about-block-head">
              <strong>Version</strong>
              <span className="about-version">v{version}</span>
            </div>
          </div>

          <div className="about-block">
            <div className="about-block-head">
              <strong>Updates</strong>
            </div>
            <small>Checking and installing updates is on General.</small>
            <div className="about-block-actions">
              <button
                className="secondary-button about-update-button"
                disabled={!setActiveView}
                onClick={() => setActiveView?.("General")}
                type="button"
              >
                Open General
                <ArrowUpRight aria-hidden="true" size={15} />
              </button>
            </div>
          </div>

          <div className="about-block">
            <div className="about-block-head">
              <strong>Privacy</strong>
              <span className="pill preserve">Local-first</span>
            </div>
            <small>
              Your audio is recorded and transcribed on this device and never
              uploaded. Your files are listed under Your data.
            </small>
          </div>

          <CopyDiagnostics />
          </div>
        </article>
      ),
    },
    {
      id: "release-notes",
      title: "Release notes",
      icon: <ScrollText aria-hidden="true" size={16} />,
      render: () => (
        <article className="panel-card about-releases-card">
          <ReleaseHistory />
        </article>
      ),
    },
    {
      id: "features",
      title: "What Scribe does",
      icon: <ListChecks aria-hidden="true" size={16} />,
      render: () => (
        <article className="panel-card about-features-card">
          <p className="about-lead">
            Private speech-to-text that works in every app and never sends
            your voice anywhere.
          </p>
          <ul className="feature-grid">
            {FEATURES.map(({ Icon, title, text }) => (
              <li className="feature-card" key={title}>
                <span className="feature-card-icon" aria-hidden="true">
                  <Icon size={18} />
                </span>
                <strong className="feature-card-title">{title}</strong>
                <span className="feature-card-text">{text}</span>
              </li>
            ))}
          </ul>
        </article>
      ),
    },
    {
      id: "setup",
      title: "Setup",
      icon: <Check aria-hidden="true" size={16} />,
      render: () => (
        <article className="panel-card">
          <div className="settings-list">
          <p className="about-lead">
            Three steps and you're dictating.
          </p>
          <ol className="about-steps">
            {SETUP_STEPS.map(({ title, text, action, target }) => (
              <li key={title}>
                <span className="about-step-body">
                  <strong>{title}</strong>
                  <small>{text}</small>
                  <button
                    className="secondary-button about-step-button"
                    disabled={!setActiveView}
                    onClick={() => setActiveView?.(target)}
                    type="button"
                  >
                    {action}
                    <ArrowUpRight aria-hidden="true" size={15} />
                  </button>
                </span>
              </li>
            ))}
          </ol>
          </div>
        </article>
      ),
    },
  ];

  const [activeTab, setActiveTab] = useState(tabs[0].id);
  const active = tabs.find((tab) => tab.id === activeTab) ?? tabs[0];

  return (
    <section className="view-grid">
      <div className="settings-tabs" role="tablist" aria-label="About sections">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            id={`about-tab-${tab.id}`}
            aria-controls={`about-panel-${tab.id}`}
            aria-selected={tab.id === active.id}
            className={`settings-tab${tab.id === active.id ? " is-active" : ""}`}
            data-setting={tab.title}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.icon}
            <span>{tab.title}</span>
          </button>
        ))}
      </div>
      <div
        className="settings-tabpanel"
        role="tabpanel"
        id={`about-panel-${active.id}`}
        aria-labelledby={`about-tab-${active.id}`}
      >
        {active.render()}
      </div>
    </section>
  );
}
