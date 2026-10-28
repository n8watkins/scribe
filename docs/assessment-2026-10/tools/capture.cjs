// Captures every Scribe view with a mocked Tauri IPC.
// Usage: node capture.cjs <outDir>   (Vite dev server must be on :1477)
const { chromium } = require("/home/natkins/.npm-global/lib/node_modules/playwright");
const fs = require("fs");
const path = require("path");

const OUT = process.argv[2];
const BASE = "http://localhost:1477/";
const MOCK = fs.readFileSync(path.join(__dirname, "mock-ipc.js"), "utf8");
fs.mkdirSync(OUT, { recursive: true });
const metrics = [];

async function open(browser, scenario, size = { width: 960, height: 680 }, query = "") {
  const page = await browser.newPage({ viewport: size, deviceScaleFactor: 1 });
  page.on("pageerror", (e) => console.log("PAGEERR", e.message));
  await page.addInitScript({ content: `window.__SCRIBE_SCENARIO=${JSON.stringify(scenario)};\n${MOCK}` });
  await page.goto(BASE + query);
  await page.waitForTimeout(query ? 900 : 1600);
  return page;
}

async function nav(page, view) {
  await page.locator("nav.nav-list button", { hasText: new RegExp(`^${view.replace("&", "\\&")}$`) }).click();
  await page.waitForTimeout(700);
}

async function shot(page, name) {
  await page.screenshot({ path: path.join(OUT, `${name}.png`) });
}

// Measures the main scroll area and records how many viewport-heights it spans.
async function measure(page, name) {
  const m = await page.evaluate(() => {
    const main = document.querySelector("main.dashboard");
    return { scrollHeight: main.scrollHeight, clientHeight: main.clientHeight };
  });
  metrics.push({ name, ...m, screens: +(m.scrollHeight / m.clientHeight).toFixed(2) });
  return m;
}

// Full capture: grow the viewport until the main area no longer scrolls.
async function full(page, name) {
  const vp = page.viewportSize();
  const m = await measure(page, name);
  if (m.scrollHeight > m.clientHeight + 2) {
    await page.setViewportSize({ width: vp.width, height: vp.height + (m.scrollHeight - m.clientHeight) + 4 });
    await page.waitForTimeout(300);
    await shot(page, `${name}--full`);
    await page.setViewportSize(vp);
    await page.waitForTimeout(200);
  }
}

const VIEWS = ["Dashboard", "Transcribe", "History", "Notes", "Sync", "Stats", "Settings", "Data & Privacy", "Hotkeys", "Models", "Audio", "Themes", "About", "Developer"];
const slug = (s) => s.toLowerCase().replace(/ & /g, "-").replace(/\s+/g, "-");

(async () => {
  const browser = await chromium.launch();

  // 1. Every view at the default window (960x680), midnight theme, with full-length captures.
  {
    const page = await open(browser, { developer: true });
    for (const [i, v] of VIEWS.entries()) {
      await nav(page, v);
      const n = `${String(i + 1).padStart(2, "0")}-${slug(v)}`;
      await shot(page, n);
      await full(page, n);
    }
    // Settings and Transcribe sub-tabs.
    await nav(page, "Settings");
    for (const tab of ["Dictionary", "Notes"]) {
      await page.getByRole("tab", { name: tab }).click();
      await page.waitForTimeout(400);
      await shot(page, `07-settings-${slug(tab)}`);
      await full(page, `07-settings-${slug(tab)}`);
    }
    await page.getByRole("tab", { name: "Notes" }).click();
    await page.getByRole("button", { name: "Test connection" }).click();
    await page.waitForTimeout(400);
    await page.locator(".llm-status-card").scrollIntoViewIfNeeded();
    await shot(page, "07-settings-notes-llm-connected");
    await nav(page, "Transcribe");
    const fileTab = page.getByRole("tab", { name: /file/i });
    if (await fileTab.count()) {
      await fileTab.first().click();
      await page.waitForTimeout(400);
      await shot(page, "02-transcribe-file");
    }
    // Models with the catalog expanded.
    await nav(page, "Models");
    await page.getByText("Browse models").click();
    await page.waitForTimeout(500);
    await shot(page, "10-models-browse");
    await full(page, "10-models-browse");
    // About sub-tabs.
    await nav(page, "About");
    for (const tab of ["What Scribe does", "Setup"]) {
      await page.getByRole("tab", { name: tab }).click();
      await page.waitForTimeout(400);
      await shot(page, `13-about-${slug(tab)}`);
      await full(page, `13-about-${slug(tab)}`);
    }
    // History with an expanded row.
    await nav(page, "History");
    const firstRow = page.locator(".history-row, .transcript-row, li").first();
    await firstRow.click().catch(() => {});
    await page.waitForTimeout(400);
    await shot(page, "03-history-expanded");
    // Hotkey capture state.
    await nav(page, "Hotkeys");
    await page.locator(".hotkeys-bind").nth(1).click();
    await page.waitForTimeout(300);
    await shot(page, "09-hotkeys-capturing");
    await page.keyboard.press("Escape");
    // Keyboard focus ring check on the sidebar.
    await nav(page, "Dashboard");
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    await shot(page, "a11y-focus-tab3");
    // Recording state with a live transcript.
    await page.evaluate(() => {
      window.__SCRIBE_SET_STATUS("Recording");
      window.__SCRIBE_EMIT("audio://recording-started", { sessionId: "s1", microphoneId: "default", microphoneName: "Realtek", sampleRate: 16000, channels: 1, startedAt: new Date().toISOString(), maxDurationMs: 600000, isTestClip: false, isNote: false });
      window.__SCRIBE_EMIT("scribe:partial-transcript", { sessionId: "s1", text: "So the plan for tomorrow is to finish the assessment and", segments: 1, finalized: false });
    });
    await page.waitForTimeout(500);
    await shot(page, "15-dashboard-recording");
    await page.close();
  }

  // 2. Smallest window (the configured minimum 820x540) and a larger one.
  for (const [label, size] of [["min-820x540", { width: 820, height: 540 }], ["large-1440x900", { width: 1440, height: 900 }]]) {
    const page = await open(browser, {}, size);
    for (const v of ["Dashboard", "Settings", "History", "Models", "Hotkeys"]) {
      await nav(page, v);
      await shot(page, `size-${label}-${slug(v)}`);
      await measure(page, `size-${label}-${slug(v)}`);
    }
    await page.close();
  }

  // 3. Themes.
  for (const theme of ["daylight", "ocean", "emerald", "violet", "slate", "custom"]) {
    const scenario = theme === "custom" ? { theme, customTheme: { background: "#1a1410", surface: "#2a2018", accent: "#f59e0b", text: "#fef3c7", textMuted: "#a8a29e" } } : { theme };
    const page = await open(browser, scenario);
    await shot(page, `theme-${theme === "daylight" ? "pewter" : theme}-dashboard`);
    if (theme === "daylight") {
      for (const v of ["Settings", "History", "Models", "Hotkeys", "Themes", "Sync"]) {
        await nav(page, v);
        await shot(page, `theme-pewter-${slug(v)}`);
      }
    }
    await page.close();
  }

  // 4. Error, empty and first-run states.
  {
    const page = await open(browser, { modelsDownloaded: false, history: "empty", github: "disconnected", analysisEnabled: false, replacements: false });
    await shot(page, "state-first-run-dashboard");
    await nav(page, "Models");
    await shot(page, "state-first-run-models");
    await full(page, "state-first-run-models");
    await nav(page, "History");
    await shot(page, "state-empty-history");
    await nav(page, "Notes");
    await shot(page, "state-empty-notes");
    await nav(page, "Stats");
    await shot(page, "state-empty-stats");
    await nav(page, "Sync");
    await shot(page, "state-github-disconnected");
    await nav(page, "Settings");
    await page.getByRole("tab", { name: "Dictionary" }).click();
    await page.waitForTimeout(300);
    await shot(page, "state-empty-dictionary");
    await page.close();
  }
  {
    const page = await open(browser, { mics: false });
    await shot(page, "state-no-mic-dashboard");
    await nav(page, "Audio");
    await shot(page, "state-no-mic-audio");
    await page.close();
  }
  {
    const page = await open(browser, { llmReachable: false });
    await nav(page, "Settings");
    await page.getByRole("tab", { name: "Notes" }).click();
    await page.getByRole("button", { name: "Test connection" }).click();
    await page.waitForTimeout(400);
    await page.locator(".llm-status-card").scrollIntoViewIfNeeded();
    await shot(page, "state-llm-down");
    await nav(page, "Notes");
    await shot(page, "state-llm-down-notes");
    await page.close();
  }
  {
    const page = await open(browser, { github: "unconfigured" });
    await nav(page, "Sync");
    await shot(page, "state-github-unconfigured");
    await page.close();
  }
  {
    const page = await open(browser, { status: "Error" });
    await shot(page, "state-app-error-dashboard");
    await page.close();
  }
  {
    const page = await open(browser, { update: true });
    await page.waitForTimeout(1500);
    await shot(page, "state-update-launch");
    await page.close();
  }

  // 5. Floating pill (230x56 window) in its modes and states.
  for (const mode of ["dot", "visualizer", "visualizer_with_text"]) {
    for (const status of ["Recording", "Idle"]) {
      const p2 = await open(browser, { status, pillMode: mode }, { width: 230, height: 56 }, "?view=pill");
      if (status === "Recording") {
        await p2.evaluate(() => {
          let t = 0;
          const id = setInterval(() => window.__SCRIBE_EMIT("audio://level", { sessionId: "s1", level: 0.3 + 0.5 * Math.abs(Math.sin(t++)), peak: 0.8, rms: 0.4 }), 50);
          window.__SCRIBE_EMIT("scribe:partial-transcript", { sessionId: "s1", text: "So the plan for tomorrow is", segments: 1, finalized: false });
          setTimeout(() => clearInterval(id), 1500);
        });
        await p2.waitForTimeout(900);
      }
      await p2.screenshot({ path: path.join(OUT, `pill-${mode}-${status.toLowerCase()}.png`) });
      await p2.close();
    }
  }

  fs.writeFileSync(path.join(OUT, "..", "..", "lane3-scroll-metrics.json"), JSON.stringify(metrics, null, 2));
  await browser.close();
})();
