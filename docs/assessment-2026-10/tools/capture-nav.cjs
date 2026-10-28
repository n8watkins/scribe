// Captures every sidebar page with the mocked Tauri IPC at 946x630 and 820x540
// in Midnight and Pewter, reports horizontal overflow, cut-off selects and
// scroll depth, and checks that every settings search entry jumps to and
// highlights its row. Usage, with Vite running from app/ on <port>:
//   node capture-nav.cjs <outDir> <port> <path/to/mock-ipc.js> [prefix]
// Set DEV=1 to include the Developer page.
const { chromium } = require("/home/natkins/.npm-global/lib/node_modules/playwright");
const fs = require("fs");
const path = require("path");
const [OUT, PORT, MOCKPATH, PREFIX = ""] = process.argv.slice(2);
const MOCK = fs.readFileSync(MOCKPATH, "utf8");
fs.mkdirSync(OUT, { recursive: true });
const slug = (s) => s.toLowerCase().replace(/&/g, "and").replace(/[^a-z0-9]+/g, "-").replace(/-+$/, "");
const report = [];
(async () => {
  const browser = await chromium.launch();
  for (const theme of ["midnight", "daylight"]) {
    for (const size of [{ width: 946, height: 630 }, { width: 820, height: 540 }]) {
      const page = await browser.newPage({ viewport: size, deviceScaleFactor: 1 });
      page.on("pageerror", (e) => console.log("PAGEERR", e.message));
      page.on("console", (m) => { if (m.type() === "error") console.log("CONSOLE", m.text()); });
      await page.addInitScript({ content: `window.__SCRIBE_SCENARIO=${JSON.stringify({ theme, developer: process.env.DEV === "1", failedHotkey: "openDashboard" })};\n${MOCK}` });
      await page.goto(`http://localhost:${PORT}/`);
      await page.waitForTimeout(1500);
      const labels = await page.$$eval("nav.nav-list button", (bs) => bs.map((b) => b.textContent.trim()));
      const side = await page.evaluate(() => { const n = document.querySelector("aside.sidebar"); return { sh: n.scrollHeight, ch: n.clientHeight }; });
      report.push({ theme, size: `${size.width}x${size.height}`, view: "(sidebar)", sidebarScrolls: side.sh > side.ch + 1, ...side });
      for (const [i, label] of labels.entries()) {
        await page.locator("nav.nav-list button").nth(i).click();
        await page.waitForTimeout(600);
        const m = await page.evaluate(() => {
          const main = document.querySelector("main.dashboard");
          const over = [];
          for (const el of main.querySelectorAll("*")) {
            if (el.scrollWidth > el.clientWidth + 1 && getComputedStyle(el).overflowX !== "visible" && el.clientWidth > 0 && !["TEXTAREA","SELECT","INPUT"].includes(el.tagName)) over.push(el.className || el.tagName);
          }
          const canvas = document.createElement("canvas").getContext("2d");
          for (const sel of main.querySelectorAll("select")) {
            if (!sel.offsetParent) continue;
            const cs = getComputedStyle(sel);
            canvas.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
            const text = sel.options[sel.selectedIndex]?.text ?? "";
            const need = canvas.measureText(text).width + parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
            if (need > sel.clientWidth + 1) over.push(`select:${text}`);
          }
          return { sw: main.scrollWidth, cw: main.clientWidth, sh: main.scrollHeight, ch: main.clientHeight, over: over.slice(0, 5) };
        });
        const name = `${PREFIX}${theme === "daylight" ? "pewter" : "midnight"}-${size.width}x${size.height}-${String(i + 1).padStart(2, "0")}-${slug(label)}`;
        await page.screenshot({ path: path.join(OUT, `${name}.png`) });
        report.push({ theme, size: `${size.width}x${size.height}`, view: label, hScroll: m.sw > m.cw, screens: +(m.sh / m.ch).toFixed(2), clipped: m.over });
      }
      if (theme === "midnight" && size.width === 946) {
        // Every search entry must jump to its page and highlight its target.
        const entries = await page.evaluate(async () => (await import("/src/lib/settingsSearch.ts")).SETTINGS_INDEX);
        for (const e of entries) {
          await page.keyboard.press("Control+K");
          await page.keyboard.type(e.label);
          await page.waitForTimeout(80);
          const first = await page.$eval(".settings-search-result span", (n) => n.textContent).catch(() => null);
          await page.keyboard.press("Enter");
          await page.waitForTimeout(450);
          const ok = await page.evaluate((t) => { const el = document.querySelector(`main [data-setting="${CSS.escape(t)}"]`); return el ? el.classList.contains("search-hit") : false; }, e.target ?? e.label);
          const title = await page.$eval("main h1", (n) => n.textContent);
          if (!ok || title !== e.page || first !== e.label) console.log("SEARCH-MISS", JSON.stringify({ label: e.label, page: e.page, first, title, highlighted: ok }));
        }
        for (const q of ["paste", "clipboard", "hotkey", "shortcut", "retention", "delete", "audio"]) {
          await page.keyboard.press("Control+K");
          await page.keyboard.type(q);
          await page.waitForTimeout(120);
          const hits = await page.$$eval(".settings-search-result", (ns) => ns.map((n) => n.querySelector("span").textContent + " / " + n.querySelector("small").textContent));
          console.log("QUERY", q, "=>", hits.join(" | "));
          if (q === "paste" || q === "retention") await page.screenshot({ path: path.join(OUT, `${PREFIX}search-${q}.png`) });
          await page.keyboard.press("Escape");
        }
        await page.keyboard.press("Control+K");
        await page.keyboard.type("auto-stop pause");
        await page.keyboard.press("Enter");
        await page.waitForTimeout(500);
        await page.screenshot({ path: path.join(OUT, `${PREFIX}search-jump-advanced.png`) });
      }
      await page.close();
    }
  }
  await browser.close();
  fs.writeFileSync(path.join(OUT, `${PREFIX}report.json`), JSON.stringify(report, null, 1));
  for (const r of report) console.log(JSON.stringify(r));
})();
