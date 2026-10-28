import {
  BookOpen,
  Cloud,
  Database,
  FileAudio,
  Gauge,
  History as HistoryIcon,
  Info,
  Keyboard,
  Mic,
  MonitorCog,
  Palette,
  Radio,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  type LucideIcon,
} from "lucide-react";
import type { ViewName } from "../types";

export type NavItem = { label: ViewName; Icon: LucideIcon };
export type NavGroup = { title: string; items: NavItem[] };

/** The sidebar, grouped by job: using Scribe, setting it up, the two
 * connections to outside services, and the app itself. */
export const NAV_GROUPS: readonly NavGroup[] = [
  {
    title: "Use",
    items: [
      { label: "Home", Icon: Gauge },
      { label: "History", Icon: HistoryIcon },
      { label: "Transcribe a file", Icon: FileAudio },
    ],
  },
  {
    title: "Set up",
    items: [
      { label: "Dictation", Icon: Mic },
      { label: "Words", Icon: BookOpen },
      { label: "Microphone", Icon: Radio },
      { label: "Models & language", Icon: Database },
      { label: "Hotkeys", Icon: Keyboard },
    ],
  },
  {
    title: "Integrations",
    items: [
      { label: "Local AI", Icon: Sparkles },
      { label: "GitHub backup", Icon: Cloud },
    ],
  },
  {
    title: "App",
    items: [
      { label: "Your data", Icon: ShieldCheck },
      { label: "Appearance", Icon: Palette },
      { label: "General", Icon: SlidersHorizontal },
      { label: "About", Icon: Info },
    ],
  },
];

/** The sidebar groups, with the opt-in Developer page placed just before
 * About when developer settings are on. */
export function visibleNavGroups(developerEnabled: boolean): NavGroup[] {
  if (!developerEnabled) {
    return [...NAV_GROUPS];
  }
  return NAV_GROUPS.map((group) =>
    group.title === "App"
      ? {
          ...group,
          items: [
            ...group.items.slice(0, -1),
            { label: "Developer", Icon: MonitorCog },
            ...group.items.slice(-1),
          ],
        }
      : group,
  );
}

/** Routes the backend can ask for (tray menu, hotkeys), mapped to pages. Older
 * route names keep working so a tray built before the regroup still lands on
 * the page that now holds that content. */
const ROUTES: Record<string, ViewName> = {
  home: "Home",
  dashboard: "Home",
  stats: "Home",
  history: "History",
  notes: "History",
  transcribe: "Transcribe a file",
  dictation: "Dictation",
  settings: "Dictation",
  words: "Words",
  dictionary: "Words",
  microphone: "Microphone",
  audio: "Microphone",
  models: "Models & language",
  hotkeys: "Hotkeys",
  ai: "Local AI",
  sync: "GitHub backup",
  data: "Your data",
  privacy: "Your data",
  appearance: "Appearance",
  themes: "Appearance",
  general: "General",
  about: "About",
};

export function routeToView(route: string): ViewName | null {
  return ROUTES[route.trim().toLowerCase()] ?? null;
}

/** Pages that used to be Settings tabs, for callers that still ask for a tab
 * by its old id (History's "Settings" button). */
export function settingsTabToView(tabId?: string): ViewName {
  switch (tabId) {
    case "notes":
      return "Local AI";
    case "dictionary":
      return "Words";
    case "history":
    case "output":
      return "Your data";
    default:
      return "Dictation";
  }
}
