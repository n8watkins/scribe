# Slint trial: results

> **The trial's main window is a test rig, not a design.**
> It uses Slint's stock Fluent widgets in dark mode and does not try to look like Scribe.
> Only the floating pill is a copy of today's Scribe, and that copy is checked pixel by pixel below.
> Whether Slint can make the whole app look exactly like today's Scribe is answered in "Can Slint look exactly like Scribe?" further down.

A standalone go or no-go build for moving Scribe to Slint.
It lives in `spikes/slint-trial`, outside every workspace, is never packaged, uses fixture data and no Scribe backend code.
Measured on the owner's Windows 11 PC (two monitors at 100%: 2560x1440 primary, 1920x1080 to its left), 2026-11-25.

## Verdict

**Go, with conditions.**
Every memory pass mark is met by Slint's own `software` renderer with room to spare, and the pill is a near pixel copy of today's.
Nothing tested so far took the focus.
The conditions are three real gaps found on the way, each with a working workaround in this crate but needing a decision in the rewrite: Chinese text falls back to a font without the glyphs, right-to-left rows do not align right by themselves, and `skia-software` keeps about 5.6 MB after the main window closes.
Input methods, screen readers, real clicks and drags have not been tried by a person yet (see the checklist); a failure there could still turn this into a no-go.

## Pass marks

Memory is Task Manager's "Memory" column (private working set), mean of five samples; private bytes in brackets.
Idle is the tray icon only, nothing created.

| Mark | Target | `skia-software` | `software` |
| --- | --- | --- | --- |
| Idle in the tray | 15 MB or less | 2.0 MB (2.7) **pass** | 2.0 MB (2.7) **pass** |
| Pill animating, bar | 20 MB or less | 2.8 MB (3.7) **pass** | 2.8 MB (3.7) **pass** |
| Pill animating, text growing | 20 MB or less | 3.3 MB (4.2) **pass** | 3.1 MB (4.0) **pass** |
| Main window open on the 2,000-row history | 40 MB or less | 11.2 MB (12.6) **pass** | 7.3 MB (8.3) **pass** |
| 10 s after closing it, as built (no trim) | idle + 5 = 7.0 MB | 7.6 MB (8.7) **fail** | 3.8 MB (4.8) **pass** |
| 10 s after closing it, trimmed on close (as Pulse does) | idle + 5 = 7.0 MB | 0.2 MB (9.0) pass on the column, but private bytes still +6.3 MB | 0.2 MB (4.8) **pass** |
| GPU driver DLLs loaded | none | none **pass** | none **pass** |
| Foreground window unchanged (pill shown, hidden, animated, clicked) | never ours | 0 of about 2,800 samples **pass** | same run |
| History scroll, no frame over 50 ms | 0 | 0 (worst gap 26.9 ms, worst draw 16.4 ms) **pass** | 0 (18.9 ms, 4.6 ms) **pass** |
| Editor with 60,000 characters opens | under 200 ms | 14 to 19 ms **pass** | 14 to 24 ms **pass** |
| No missing-glyph boxes | none | boxes in Chinese with the default fallback **fail**; none with a per-language font | same |
| Right-to-left in the right order | yes | order right; alignment needs a workaround | same |

Notes on the numbers:
- The main window is destroyed on close (the process has no top-level window left but the tray's hidden ones), and the history model and transcript go with it.
- What `skia-software` keeps after closing is Skia's own caches (glyphs, typefaces); `software` keeps 1.8 MB.
- No GPU driver DLL (AMD, NVIDIA, Intel) is ever loaded, with either renderer.
  The system graphics DLLs `d3d12`, `dxgi`, `dxcore`, `D3DCOMPILER_47` and `opengl32` are loaded from the start, even idle and with `software`, because the binary links Skia; a build without `renderer-skia` should drop them (not tried).
- The scroll, editor and window-open timings come from the run before the owner asked to keep the trial off his screen, when the main window was still on screen.
  Measuring runs now open the main window off every monitor, and Windows does not paint such a window on its own, so frame timings are only valid with `measure.ps1 -Probes`, which puts it back on screen.
  The WSL side of the PC was busy at the time (load average about 28); the Windows side was not measured.
- Opening the main window from nothing to its first frame took 86 ms the first time and 16 to 20 ms after that.
- The release exe is 17.5 MB with both renderers and Skia in it.

## The pill

`pictures/pill-compare-small.png` and `pictures/pill-compare-text.png` put today's pill (left two columns, over a dark and a light desktop) next to the trial's (right two) in all 13 states.
Today's pill was rendered from the repo's own `app/src/PillApp.tsx` and `pill.css`, built with Vite and driven by the repo's IPC mock (`docs/assessment-2026-10/tools/mock-ipc.js`), in headless Edge on the same PC, which is Chromium with DirectWrite as in WebView2.
The "Copied" state is from headless Chromium on Linux, because the Copy click did not register in headless Edge.
The trial's pictures are Slint's own rendering of the pill window (`Window::take_snapshot`), with alpha.

What matches:
- Window sizes in every state, including the text layout growing to the same heights (99, 134 and 93 px) with the bottom edge anchored.
- Shell, margins, border colours, radii, the 84% background, bar geometry and envelope, dot sizes and colours, the check icon, the Copy button, label positions and text widths.
- Line breaks in the wrapped transcript are identical.

What differs, measured over a dark desktop:
- Static states differ in 0.3% to 2.2% of pixels (by more than 24 of 255), all in glyph anti-aliasing; Slint's text has about 5% less ink than DirectWrite's.
- Wrapped text differs in about 5% of pixels: the first line sits 1 px lower than Chromium's because the line spacing is a fraction of a pixel tighter.
- Bars and the pulsing dot differ only because each side was caught at a different moment of a random feed.

Behaviour, checked without input:
- Extended style `0x08000198`: tool window, no-activate, topmost, no app-window bit, so no taskbar button and no Alt+Tab entry.
- Shown with `SW_SHOWNOACTIVATE` and hidden with `SW_HIDE` directly, never through Slint (see the surprises).
- Fed one level every 40 ms with the React constants for attack, decay, envelope and the perceptual curve (unit-tested against the React numbers).
- A click posted to the pill window itself (press, 1 px move, release) stopped recording and ran Saving, Transcribing, Inserting, then the confirmation; the foreground window did not change.
- 13 shows and 13 hides in 30 s while animating: the foreground window never became the trial's.
- A saved position on the second monitor is kept (the pill appeared at -1040,537); the Windows minimise sentinel (-32000,-32000) falls back to the bottom centre of the primary monitor, 90 px up, as today.

## The main window

- History: 2,000 rows, one to three lines each, in English, Arabic, Hebrew, mixed right-to-left and left-to-right, Japanese, Chinese, Korean, Hindi, Thai, Russian and emoji (`pictures/history.png`).
- Editor: a 60,000-character transcript in a `TextEdit`, with a right-to-left paragraph near the top, which Slint lays out right-aligned in the right order.
- Input: a single-line field and a multi-line field for the input-method checks.
- Controls: a checkbox, two switches, a slider and three buttons, one disabled.
- UI Automation, read from outside without input, reports the sidebar as a tab list with named tab items and the selected one, the checkbox with its toggle state, the switches as toggle buttons with their state, the slider with its value 70, the buttons with Invoke and the disabled one as disabled, and the history rows as list items with their text.
  The two switches report that they cannot take keyboard focus; check this on the checklist.

## Surprises and workarounds

1. **Slint reactivates a window it shows.**
   winit's `with_active(false)` only covers the first show; every later `show()` is `SW_SHOW`.
   The pill is therefore shown and hidden with Win32 calls only, and Slint never sees it hidden.
2. **No way to set `WS_EX_NOACTIVATE` at creation.**
   The style is added right after the window exists, while it is still off screen.
3. **Chinese falls back to a Japanese font.**
   Slint's font fallback has no language, so Han text goes to one font for the script, which lacks simplified glyphs such as 话, 发 and 给: they show as boxes (`pictures/history-default-font-fallback.png`).
   Adding Microsoft YaHei to the chain (`SLINT_FONT_PATH`) fixes Chinese but draws Japanese in a Chinese font.
   The trial gives each history row a font from its language (Scribe knows the dictation language), which fixes both, but free text in the editor and in input-method fields has no language and still shows boxes for such characters.
   This needs a real answer before the rewrite: a locale-aware fallback in Slint, or a font chain chosen per user language.
4. **`horizontal-alignment: start` does not follow a right-to-left paragraph in `Text`** (it does in `TextEdit`).
   The order of the text is right; rows that start with Hebrew or Arabic are aligned right by hand.
5. **The `TextEdit` opens scrolled about two lines down.**
   Its keep-the-cursor-visible rule runs before its visible height is known; a one-shot `Timer` scrolls it back to the top.
6. **Off-screen windows get no first paint, and later frames are throttled there.**
   The trial asks for the first frame itself; frame timings need the window on screen.
7. **The `software` renderer has no rendering notifier**, so frame times come from winit's `RedrawRequested` events and the next turn of the event loop, the same for both renderers.
8. **`skia-software` keeps about 5.6 MB after the window is destroyed**; `software` keeps 1.8 MB.
9. **Text sits 1 px lower than Chromium's** at 100%: Slint centres a line on the font's ascent and descent, Chromium on its line box.
   The pill's labels are moved up 1 px; CSS `line-height: 1.45` maps to `line-height-factor: 1.09` for Segoe UI Variable.
10. Small API notes: `viewport-*` on `ListView` is deprecated for `content-*`; `character-count` counts grapheme clusters (59,985 for the 60,000-character transcript); tray-icon 0.27's menu handler takes an `Arc`.
11. `PrintWindow` does not keep per-pixel alpha (the pill's corners come out black), so pill pictures use `take_snapshot`.
    Both renderers present through the same softbuffer and DWM path, so transparency should look the same; the checklist asks to confirm it.

## Can Slint look exactly like Scribe?

Read against `app/src/App.css` (3,698 lines) and the theme blocks in it.
Short answer: the geometry, colours, gradients, radii, shadows and motion can be matched to the pixel, as the pill shows; text cannot be guaranteed identical to the pixel, because Slint rasterises glyphs itself instead of through DirectWrite, and two CSS features have no Slint equivalent.
At normal viewing distance it can be indistinguishable; under a pixel diff, text edges will differ.

Can be matched exactly (Slint 1.19 has the feature):
- Fonts and weights: Segoe UI Variable is a variable font and `font-weight` takes any number; the pill's 400 and 600 match DirectWrite's text widths to the pixel.
  Scribe's other weights (650, 700, 720, 750, 760, 780, 800) were not checked one by one.
- Line height (`line-height-factor`), letter spacing, ellipsis and `-webkit-line-clamp` (`max-lines` with `overflow: elide`).
- Linear gradients on cards and progress bars, and the two page glows (`radial-gradient(circle at top left ...)`: Slint 1.19 accepts `at` positions).
- Radii, 1 px borders and translucent colours.
- Shadows: `box-shadow` with offset, blur and spread (`drop-shadow-*` including `drop-shadow-spread`), inset shadows (`inner-shadow-*`), and focus rings (`0 0 0 2px`) as a spread shadow or a border.
  Elements with two shadows need a second element.
- Transitions and animations: `animate` with CSS easing curves, and keyframes written as functions of `animation-tick()`, as the pill's pulse is.
- The custom title bar: `no-frame` with a drag area and the three buttons drawn by hand, as today.
- Custom scrollbars (10 px, accent thumb, transparent track) by building a small scroll view; the stock Fluent one looks different.

Cannot be matched pixel for pixel, and why:
- **Text rasterisation.** WebView2 draws text with DirectWrite; Slint shapes with Parley and draws glyphs with Skia or its own rasteriser.
  Glyph positions and widths match, but anti-aliasing differs: about 1% to 2% of pixels in the pill's labels, about 5% less ink overall.
- **`backdrop-filter: blur()`** (the sidebar's 20 px blur and the modal scrim's 2 px blur): Slint has no backdrop blur.
  Over Scribe's smooth page background the sidebar blur is close to invisible and can be replaced by a flat colour; a modal over busy content would look sharper behind the scrim.
- **`filter: drop-shadow()` on the title-bar logo** follows the image's alpha in CSS; Slint's shadows follow a rectangle's shape, so the glow around the logo would be a pre-rendered image or an approximation.
- **Fallback fonts** for scripts Segoe UI does not cover (CJK, Thai, Hindi, emoji) are chosen by different rules (surprise 3), so mixed-script text can use a different font than WebView2 would.
- **Native pieces drawn by Chromium**: text selection colour and caret, input-method composition underline, the `<select>` drop-down list and the spelling squiggle would all be Slint's own drawing, close but not identical.
- **Sub-pixel layout**: CSS lays out in fractional pixels and snaps differently from Slint; some 1 px differences, like the label baseline here, need a measured correction each.

## Owner checklist (needs you at the keyboard)

Build it (see below) and run `scribe-slint-trial.exe` with no arguments: an amber dot appears in the tray and nothing else.
Quit from the tray menu when done; nothing else is left running.

1. **Tray.** Left-click the tray icon: the trial window opens and takes the focus.
   Right-click: Open, Show the pill, a "Show floating pill" check item and Quit.
   Untick "Show floating pill", open Controls: its checkbox is unticked too (and the other way round).
2. **Pill never takes the focus.** Open Notepad and type a few letters.
   Tray menu, "Show the pill (recording)", then click back into Notepad (the menu itself takes the focus).
   Keep typing: the letters go to Notepad while the pill animates and its text grows upward.
3. **Transparency.** The pill's rounded corners show the desktop behind them, not black; start once with `--renderer=software` too.
4. **Click to stop.** Click the pill once without moving: Saving, Transcribing, Inserting, then "Transcribed" with Copy; Notepad's caret still blinks.
5. **Copy.** Click Copy: it turns green and says Copied, Notepad stays active; paste in Notepad: the sentence arrives.
6. **Drag.** Show the pill again, press on it and move 4 px or more: it follows the mouse and recording does not stop; Notepad stays active.
7. **Second monitor.** Drag the pill onto the other monitor and let go; show it again from the tray: it comes back there.
   If the monitors use different scaling, check it is the same physical size as today's pill on both.
8. **Taskbar and Alt+Tab.** With the pill on screen: no taskbar button for it, and Alt+Tab does not list it.
9. **Input method, single line.** Input page, single-line field, Japanese (Microsoft IME): type `nihongo`, Space, pick a candidate, Enter.
   The candidate window sits next to the caret and the committed text is right.
   Repeat with Chinese (Pinyin), Korean and Hindi if installed; expect boxes for some simplified Chinese characters (surprise 3) and say if you see any elsewhere.
10. **Input method, multi-line.** The same in the multi-line field, then Enter for a new line after committing.
11. **Editor.** Editor page: type in the middle, select with Shift+arrows and with the mouse, Ctrl+C and paste into Notepad, Ctrl+Z to undo.
    Click inside the Arabic paragraph near the top and use the arrow keys: the caret moves through the text in reading order and the selection looks right.
    Scroll to the end with the wheel and with the scroll bar: no stalls.
12. **Narrator.** Ctrl+Win+Enter, then Tab through the window.
    Expected: the four sidebar entries as tabs with their names and which is selected; "Show floating pill" as a checkbox with its state; the two switches with on or off; "Cue volume" as a slider with its value; the buttons by name and "Delete history" as unavailable; on History, each row's text.
13. **NVDA.** The same as 12, if installed.
14. **Keyboard only.** Tab to a sidebar entry and press Enter or Space: the page changes.
    Tab to the checkbox and to each switch and press Space (UI Automation says the switches cannot take focus: does Tab skip them?).
15. **Smoothness on screen.** Scroll the history fast with the wheel and drag the scroll bar: no stalls you can see.

## Build, run and measure

Windows, from WSL (the source can stay in WSL; the target folder must be on C:):

```
cargo.exe build --release --manifest-path <\\wsl.localhost\...\spikes\slint-trial\Cargo.toml> --target-dir <C:\...\target>
```

Linux: `cargo build` in `spikes/slint-trial` (the Linux build has no tray icon and places the pill with Slint's own calls; it was built and its unit tests run, not measured).

Every state can be reached without a keyboard or mouse, with `--key=value` on the command line or `TRIAL_KEY` in the environment:

| Hook | What it does |
| --- | --- |
| `--renderer=skia-software` (default), `software`, `skia` | Slint renderer |
| `--open=history`, `editor`, `input`, `controls` | open the main window on that page, not activated and off screen |
| `--main-onscreen=1` | keep the main window on screen (for frame timings) |
| `--close-after=<ms>` | destroy the main window |
| `--pill=<state>` | show the pill in a state: `dot`, `dot-error`, `bar`, `note`, `text`, `text-full`, `pending`, `pending-text`, `ready`, `error`, `confirm`, `confirm-copied`, `confirm-text`, or `cycle` |
| `--hold=1` | keep the pill's state (no auto-hide) |
| `--monitor=<n>`, `--pill-pos=x,y` | pretend the saved pill position is monitor n's centre, or this point |
| `--fake-click=<ms>` | post a click to the pill window itself |
| `--probe=scroll`, `editor`, `open` | time history scrolling, opening the editor, opening the window |
| `--snapshot=<dir>`, `--snapshot-main=<file>` | save Slint's own pictures of every pill state, or of the main window |
| `--status=<file>` | write events as JSON lines |
| `--exit-after=<ms>` | quit (a measuring run always quits within 10 minutes) |
| `--no-clipboard=1`, `--no-trim=1`, `--font-path=<files>`, `--pill-font=<family>`, `--row-fonts=0` | as named |

`scripts/measure.ps1 -Exe <exe> -Out <folder>` runs everything above that needs no input and never takes the focus, for both renderers, and writes `report.json`, pictures, module lists and UI Automation dumps.
Add `-Probes` for frame timings only when nobody is using the PC: it puts the main window on screen.

## Hands-on checks with real input (2026-11-25, 22:04 to 22:09)

Run by `scripts/checks/checks.ps1` in a hands-off window the owner gave, with real synthesized mouse and keyboard input (SendInput), the `software` renderer unless stated, and the owner's clipboard saved first and restored at the end (confirmed equal).
A watchdog would have killed everything and restored the clipboard; it did not need to fire.
Alt was never pressed, none of the owner's windows was touched, and nothing was left running.
Input-method checks and the right-to-left caret were out of scope (Scribe is unlikely to be used for Chinese).
Two other builds were compiling on the PC: CPU use was 85% to 94% during F and G, so **their timings are not trustworthy**; the pass or fail results are.
Instead of Notepad the checks typed into a small text window of their own, because Windows 11 Notepad can open as a tab in the owner's existing Notepad window.

| Check | Result | Evidence |
| --- | --- | --- |
| A. Pill shown and hidden while typing | **pass** | 4 shows and 4 hides in 10 s; the foreground stayed the text window after every chunk; every typed character arrived, in order |
| A. Real drag of the pill | **pass** | pressed and moved 120 px left and 200 px up in 20 steps: the pill followed exactly (1120,1268 to 1000,1068), recording did not stop, focus did not move |
| A. Real click stops recording | **pass** | Saving, Transcribing, Inserting, then the confirmation; focus did not move |
| A. Copy button click | **pass** | focus did not move |
| A. No taskbar button, not in Alt+Tab | **pass** | extended style `0x08000198` (tool window, no-activate, topmost, no app-window bit), the shell's Alt+Tab rule says no, and the taskbar's automation tree has no trial entry (its only "Scribe" entry, "Scribe - 1 running window", is the installed Scribe) |
| B. Corners transparent, `software` | **pass** | over a light (240) and a dark (16) backdrop of our own, the corner pixels read the backdrop exactly |
| B. Corners transparent, `skia-software` | **pass** | same |
| C. Copy puts the transcript on the clipboard, paste works | **pass** | the clipboard held the live words so far ("So the plan for tomorrow ... look at the installer"); Ctrl+V put exactly that at the end of the text window |
| D. Drag onto the second monitor and back | **pass** | dragged to -1121,496 on the 1920x1080 monitor and back to 1000,1060; size stayed 320x99 and recording continued; both monitors are at 96 DPI, so mixed scaling was not tested |
| E. Tray icon and its check item | **not run** | the trial's icon sits in the hidden overflow area; it was not rearranged and not clicked |
| F. Editor typing, select, copy, undo, key latency | **inconclusive** | the 60,000-character editor took real typing and 30 Ctrl+Z brought it back to exactly 60,000 characters, but the automation text did not show the typed run where expected, Shift+End then Ctrl+C copied 60 characters that did not start with it, and 97 of 220 keys showed no change in the watched strip within 500 ms (median 11.9 ms for the rest), at 94% CPU; the caret position after the click needs a person to check |
| G. Real mouse-wheel scrolling of 2,000 rows | **pass** (timing under load) | 16 s of wheel steps every 16 ms: 3,082 frames, worst draw 4.8 ms, none over 50 ms, at 85% CPU |
| H. Keyboard reach, controls page | **pass** | Tab order: the four sidebar tabs, "Show floating pill" (checkbox, on), "Play sounds" (toggle, on), "Launch at login" (toggle, off), "Cue volume" (slider, 70), "Show pill", "Copy transcript", then back to the tabs; Shift+Tab walks it backwards; the disabled "Delete history" is skipped as it should be |
| H. Keyboard reach, history list | **fail** | Tab only cycles through the four sidebar tabs; the 2,000-row list can never take keyboard focus, so a keyboard or screen-reader user cannot reach the transcripts |
| H. Keyboard reach, editor page | **pass** | tabs, "Copy all", then the editor ("Transcript editor", Edit) |
| H. Names | **pass** | every focus stop had a name and a role |
| H. Focus visible | **pass** for the tabs, checkbox and switches | a ring is drawn on each (`focus-rings.png`); the checkbox's ring spans the whole row, which looks off; the slider's ring was not confirmed |

What this is and is not: H reads the UI Automation tree, which is what Narrator and NVDA read, after real Tab and Shift+Tab keys; it is not a listening test.

Surprises:
- The two switches report through UI Automation that they cannot take keyboard focus, yet real Tab reaches them and draws a ring: the automation property is wrong, the behaviour is right.
- The history `ListView` is not in the Tab order at all.
- The first attempts stopped on bugs in the check script itself (a hidden-start window, a taskbar lookup, reading the text window), each stopped safely with the clipboard restored; the fourth attempt ran to the end.
