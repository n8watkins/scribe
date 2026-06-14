# Scribe — Competitive Analysis & Gap Review

Status: Reference / strategy doc
Date: 2026-06-14
Scope: Scribe **0.5.18** vs. the local/private dictation field
Method: feature audit of the Scribe codebase (`app/src`, `app/src-tauri/src`) +
web research on competitors (June 2026). Competitor pricing and feature splits
shift often — verify on each vendor's site before quoting externally.

---

## 1. Where Scribe stands, in one paragraph

Scribe is a **free, source-available (MIT), fully-local, Windows-only**
push-to-talk dictation app built on whisper.cpp, with a feature set that has
grown well past "dictate into the focused app." It pairs on-device transcription
(warm `whisper-server` + live/incremental segmentation for near-instant
stop-to-text) with an **optional local-LLM layer** — AI dictation cleanup,
on-demand note analysis, and a **selected-text transform** that turns any
highlighted text into an inline voice/typed AI edit — plus quick notes, a
searchable local history with combine/export, a dictionary (Whisper priming +
deterministic replacements), Google Drive sync, a configurable floating pill, and
a signed in-app auto-updater. Against the field it occupies a distinctive corner:
most polished commercial rivals (Wispr Flow, Aqua) are **cloud-only
subscriptions**; the strongest local commercial tool (Superwhisper) is paid and
Mac-first; and the open-source local tools that *do* run on Windows (Whispering,
Buzz, OpenWhispr, WhisperWriter, Vibe) generally either lack Scribe's
notes/LLM-transform/sync polish or are file-transcription-first rather than
type-anywhere. Scribe's clearest weaknesses are **no Authenticode signing**
(SmartScreen friction), **English-only transcription today**, and **no
arbitrary/custom local model selection**.

---

## 2. The field at a glance

| Tool | Platform(s) | Pricing | Local / Cloud | Open source | One-liner |
|---|---|---|---|---|---|
| **Scribe** | Windows 10/11 | **Free** | **Local** (LLM features hit a local server you run) | **Yes (MIT)** | Local dictation + local-LLM cleanup/transform + notes + sync |
| Wispr Flow | Mac, Win, iOS, Android | Sub ($12–15/mo) | **Cloud only** | No | Polished AI formatting & dictation everywhere |
| Superwhisper | Mac, Win, iOS | Sub $8.49/mo or **$249.99 lifetime** | Both (local default, BYO cloud) | No | Private on-device dictation w/ model choice + per-app modes |
| MacWhisper | **Mac only** | One-time €59/$69 (or App Store sub) | Local | No | File/media transcription & subtitles, local |
| Aqua Voice | Mac, Win, iOS | Free tier + $8/mo Pro | **Cloud only** | No | Context-aware auto-formatting + voice editing |
| Whispering | Win, Mac, Linux | **Free** | Both (whisper.cpp local + BYO Groq/OpenAI/ElevenLabs) | **Yes (AGPL/MIT)** | Hackable local-first dictation w/ chainable transforms |
| Talon Voice | Win, Mac, Linux(X11) | Free core / $25 beta | **Local** | Mostly no | Voice **control** + voice coding (accessibility/RSI) |
| Vibe | Win, Mac, Linux | **Free** | Local (local/Ollama summaries) | **Yes (MIT)** | Offline file/URL transcription + subtitles + diarization |
| Windows Voice Access | **Win 11 only** | Free (built-in) | Local | No | Voice **control** + dictation, accessibility-first |
| Win+H Voice typing | Windows | Free (built-in) | **Cloud** (Azure) | No | The easy default dictation toolbar |
| Dragon Professional | **Windows only** | One-time ~$699 | Local (Anywhere=cloud) | No | Pro-grade accuracy + custom vocab/macros (aging) |
| OpenWhispr | Win, Mac, Linux | Free local / paid cloud | Both | **Yes** | Cross-platform privacy-first push-to-talk |
| Buzz | Win, Mac, Linux | **Free** | Local (+opt OpenAI) | **Yes (MIT)** | Popular OSS file transcription + translate + subtitles |
| WhisperWriter | Win, Mac, Linux | **Free** | Local (faster-whisper) | **Yes (GPL-3)** | Minimal type-anywhere push-to-talk |

(VoiceInk — Mac/iOS-only OSS — and the legacy Windows Speech Recognition are
noted but excluded from the Windows-competitor matrix.)

---

## 3. Feature-comparison matrix

Legend: ✅ yes · ⚠️ partial / caveated · ❌ no · — n/a. "Local-capable" means it
can run transcription on-device with no audio leaving the machine.

| Capability | **Scribe** | Wispr Flow | Superwhisper | MacWhisper | Aqua | Whispering | Talon | Vibe | Voice Access | Dragon | OpenWhispr | Buzz / WhisperWriter |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Runs on **Windows** | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Local / offline** transcription | ✅ | ❌ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Free** | ✅ | ⚠️ tier | ⚠️ tier | ⚠️ tier | ⚠️ tier | ✅ | ⚠️ core | ✅ | ✅ | ❌ | ⚠️ tier | ✅ |
| **Open source** | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | ⚠️ | ✅ | ❌ | ❌ | ✅ | ✅ |
| **No account** required | ✅ | ❌ | ⚠️ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ✅ |
| Push-to-talk + toggle | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ |
| **Type/paste into any app** | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ |
| Live / **incremental** (low stop latency) | ✅ | ✅ | ⚠️ | ⚠️ | ✅ | ⚠️ | ✅ | ⚠️ | ✅ | ✅ | ⚠️ | ⚠️ |
| **Real-time streaming insert** (text appears as you speak) | ❌ | ⚠️ | ❌ | ❌ | ⚠️ | ❌ | ✅ | ❌ | ⚠️ | ⚠️ | ❌ | ❌ |
| **AI cleanup** (filler/punct/format) | ✅ (local LLM) | ✅ | ✅ | ⚠️ | ✅ | ✅ | ❌ | ⚠️ | ❌ | ⚠️ | ⚠️ | ❌ |
| **Per-context modes** (email/chat/code…) | ✅ | ✅ (app-aware) | ✅ (per-app) | ❌ | ✅ (app-aware) | ⚠️ | ⚠️ | ❌ | ❌ | ⚠️ | ⚠️ | ❌ |
| **Selected-text transform** (rewrite highlighted text) | ✅ | ⚠️ cmd mode | ❌ | ❌ | ⚠️ | ✅ transforms | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Voice editing / commands** ("make this a list") | ⚠️ via transform | ✅ | ⚠️ | ❌ | ✅ | ⚠️ | ✅ | ❌ | ✅ | ✅ | ⚠️ | ❌ |
| **Spoken punctuation / dictation commands** | ❌ | ✅ | ✅ | — | ✅ | ⚠️ | ✅ | — | ✅ | ✅ | ⚠️ | ❌ |
| **Custom vocabulary / replacements** | ✅ | ✅ | ✅ | ⚠️ | ✅ | ⚠️ | ✅ | ❌ | ❌ | ✅ | ✅ | ❌ |
| **Model selection** (catalog) | ✅ | ❌ | ✅ | ✅ | ❌ | ✅ | ⚠️ | ✅ | ❌ | ❌ | ✅ | ✅ |
| **Custom / arbitrary local model** | ❌ | ❌ | ⚠️ | ✅ | ❌ | ✅ | ⚠️ | ✅ | ❌ | ❌ | ⚠️ | ✅ |
| **Multilingual** transcription | ❌ (EN only) | ✅ 100+ | ✅ 100+ | ✅ ~99 | ✅ 49 | ✅ ~99 | ❌ EN | ✅ ~99 | ⚠️ ~4 | ⚠️ per-edition | ✅ 100+ | ✅ ~99 |
| **Translation** | ⚠️ via LLM transform | ⚠️ | ✅ →EN | ✅ →EN | ❌ | ⚠️ | ❌ | ✅ →EN | ❌ | ❌ | ⚠️ | ✅ →EN |
| **Quick notes** capture | ✅ | ⚠️ | ⚠️ meetings | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Searchable history** | ✅ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ❌ | ⚠️ | ❌ | ⚠️ | ⚠️ | ⚠️ |
| **Export** (MD/CSV/JSON) | ✅ | ⚠️ | ⚠️ | ✅ (SRT/etc.) | ❌ | ⚠️ | ❌ | ✅ (many) | ❌ | ✅ | ⚠️ | ✅ (SRT/etc.) |
| **Cloud sync / backup** | ✅ (your Drive) | ✅ | ⚠️ | ❌ | ⚠️ | ❌ | ❌ | ❌ | ❌ | ⚠️ | ⚠️ | ❌ |
| **File / media transcription** | ✅ | ❌ | ✅ | ✅ | ❌ | ⚠️ | ❌ | ✅ | ❌ | ✅ | ⚠️ | ✅ |
| **Floating status UI / visualizer** | ✅ | ✅ | ✅ | ⚠️ | ✅ | ⚠️ | ⚠️ | ⚠️ | ✅ | ⚠️ | ✅ | ❌ |
| **Auto-updater** (in-app) | ✅ signed | ✅ | ✅ | ✅ | ✅ | ⚠️ | ✅ | ⚠️ | ✅ (Windows) | ✅ | ⚠️ | ⚠️ |
| **Authenticode code-signed** installer | ❌ | ✅ | ✅ | ✅ | ✅ | ⚠️ | ✅ | ⚠️ | ✅ | ✅ | ⚠️ | ❌ |
| **macOS / Linux** support | ❌ | ⚠️ Mac | ⚠️ Mac | ✅ Mac | ⚠️ Mac | ✅ | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ |

Notes on a few cells: Wispr Flow / Aqua "local" = ❌ because transcription is
cloud-only. Talon's strength is *control/coding*, not freeform dictation
formatting, hence ⚠️ on AI cleanup. Vibe / MacWhisper / Buzz are
file-transcription-first, so "type into any app" is ⚠️/❌. Scribe's
"translation" is ⚠️ because the **selected-text transform** can translate via the
LLM, but Whisper-level multilingual/translate transcription is not wired up.

---

## 4. Scribe's differentiators

These are the things that, in combination, no single competitor matches:

1. **Free + source-available + fully local + no account — all four at once.**
   The polished commercial tools (Wispr Flow, Aqua) drop "local"; Superwhisper
   and Dragon drop "free"; the free OSS local tools (Whispering, Buzz,
   OpenWhispr, WhisperWriter, Vibe) are real peers here but trail on the points
   below. For a privacy-sensitive Windows user who wants zero subscription and
   zero data egress, Scribe is in a small group.

2. **A local-LLM productivity layer, not just transcription.** AI dictation
   cleanup with built-in **Standard / Email / Chat / Code** modes (or a custom
   prompt), **on-demand note analysis**, and an **end-of-day "organize my notes"**
   pass — all pointed at a local OpenAI-compatible server (LM Studio / Ollama),
   so the "AI" features are *also* private. Most local OSS rivals stop at raw
   transcription; the tools with strong AI formatting (Wispr, Aqua) do it in the
   cloud.

3. **Selected-text transform — an inline, local AI editor.** Highlight text in
   any app, tap a hotkey, and **speak or type** an instruction ("make this
   concise", "translate to Spanish", "fix grammar") to rewrite the selection in
   place. Closest analogues are Wispr's Command Mode and Whispering's chainable
   transforms, but Scribe's "transform what's already selected, by voice,
   locally" is a genuinely differentiated workflow.

4. **Quick notes as a first-class capture mode.** Hold the toggle key + tap `Q`
   to dictate a **note** (distinct blue pill) that's saved to a Notes list
   instead of pasted, with its own retention (defaults to *keep forever*) and
   on-demand LLM analysis. None of the dictation-first competitors have an
   equivalent "capture a thought without disturbing the focused app" primitive.

5. **Sane data ownership: local SQLite history with combine/export + sync to
   *your* Drive.** Date-range search, sort, multi-select → combine, export to
   **Markdown/CSV/JSON**, separate transcript vs. note retention, and optional
   Google Drive backup to the user's own account. The data is the user's, on
   disk, in an open format — the opposite of the cloud subscription model.

6. **Thoughtful Windows insertion + UX details.** Instant clipboard paste that
   snapshots and **restores the full clipboard (text, images, files)**, held-
   modifier release backstops so paste can't scramble terminals, a draggable
   always-on-top pill with a live visualizer that works while minimized,
   per-bind **press/release** triggers, inline hotkey conflict reporting, and a
   **signed** in-app auto-updater with OS notifications.

---

## 5. Gaps — what rivals have that Scribe lacks

Ordered roughly by competitive impact. Each notes who has it and how much
plumbing Scribe already has toward it.

### High impact

1. **Authenticode (OS) code signing.** Scribe's installer isn't Authenticode-
   signed, so Windows SmartScreen throws a scary "unrecognized app" warning on
   first run — a major drop-off point for new users and the #1 thing that makes
   Scribe *feel* less trustworthy than commercial rivals (all of which are
   signed). Note this is **separate** from the updater's minisign artifact
   signing, which Scribe already does. Cost/process, not engineering: an OV or
   EV certificate (EV best clears SmartScreen reputation immediately).

2. **Multilingual transcription + Whisper translate.** Scribe ships **English-
   only** today: the model catalog is all `.en` models and the language picker
   resolves to `en`. Nearly every serious competitor (Wispr, Superwhisper,
   Whispering, Vibe, Buzz, MacWhisper, OpenWhispr) offers ~99–100 languages, and
   most offer Whisper's **translate-to-English**. This is the single biggest
   *feature* gap, and the plumbing is close: `--language` is already passed
   through `whisper.rs`/`whisper_server.rs`/`incremental.rs` and there's a
   `whisper_language()` mapping — what's missing is multilingual `ggml` models in
   the catalog, exposing the real language list in the UI, and a `--translate`
   path.

3. **Custom / arbitrary local model selection.** The Models view is a fixed
   curated catalog; you can't point Scribe at an arbitrary whisper.cpp `ggml`
   `.bin` (a distilled model, a fine-tune, or one you already downloaded).
   MacWhisper, Vibe, Buzz, Whispering, OpenWhispr all allow this. The storage
   plumbing exists (models already live under `models/`, and there's an
   `ExternalCache` source) — what's missing is UI + non-catalog selection logic
   and surfacing language/quantization so the right flags are passed. (Already on
   the FAQ backlog.)

### Medium impact

4. **Spoken punctuation, voice commands, and voice editing.** Scribe transcribes
   words literally; it has no "new line / comma / period", no "select that /
   delete that / scratch that", and no in-dictation commands. Wispr, Aqua,
   Dragon, and Voice Access all do voice editing; Talon and Voice Access do full
   voice control. Scribe's selected-text transform partially covers "edit by
   voice," but spoken punctuation/commands during dictation are a common
   expectation it doesn't meet. (AI cleanup mitigates punctuation somewhat.)

5. **Real-time streaming insertion.** Scribe transcribes incrementally in the
   background (so stop-to-text is fast) but inserts only after you stop — text
   doesn't *appear in the field as you speak*. Talon and (to a degree) Voice
   Access and the cloud tools show text live. This is a perceived-speed and
   "feels modern" gap; harder with the borrow-the-clipboard paste model.

6. **Onboarding wizard / first-run experience.** New users must independently
   find Models → download, Audio → pick mic, and (for AI features) install and
   point at a local LLM server. There's no guided first-run flow. Commercial
   tools onboard in a minute. Cheap to build, outsized effect on activation —
   especially given the SmartScreen warning already taxes first impressions.

### Lower impact (still worth tracking)

7. **macOS / Linux support.** Deliberately out of scope (Windows-only "by
   design"), but it caps the addressable audience and is the obvious axis where
   Whispering / Vibe / OpenWhispr / Talon out-reach Scribe. Listing it as an
   explicit non-goal is fine; just know it's a ceiling.

8. **GPU acceleration.** Vibe, Buzz, OpenWhispr advertise CUDA/Vulkan/Metal.
   Scribe is CPU whisper.cpp. For `large-v3-turbo` on long dictations a GPU build
   would help; for short push-to-talk on small models it matters less.

9. **Speaker diarization / file-transcription depth.** Vibe and MacWhisper do
   diarization, batch/folder transcription, and URL/YouTube import. Scribe has
   single-file transcription but isn't a media-transcription product — fine if
   that's not the positioning.

10. **Accessibility / full voice control.** Talon and Voice Access let users
    operate the whole OS by voice (clicking, navigation, eye tracking). Scribe is
    a dictation utility, not an accessibility control surface — a different
    product, but worth naming so expectations are set.

11. **Team / sharing / cross-device sync of settings & vocabulary.** Wispr syncs
    dictionary/snippets/style across devices; enterprise tiers add SSO/admin.
    Scribe's sync is single-user note/transcript backup. Likely out of scope for
    a local-first solo tool, but it's a differentiator competitors market.

---

## 6. Prioritized "build next to be competitive"

Effort is rough: **S** ≈ days, **M** ≈ 1–2 weeks, **L** ≈ multi-week, **$** ≈
mostly money/process rather than code.

| # | Build | Why it moves the needle | Effort |
|---|---|---|---|
| **1** | **Authenticode code signing** (EV/OV cert + sign NSIS in CI) | Removes the SmartScreen warning — the biggest trust/drop-off blocker for *every* new install. Highest impact-per-effort; it's procurement + a CI signing step, not feature work. | **$ / S** |
| **2** | **Multilingual transcription + translate-to-English** (multilingual `ggml` models in the catalog, real language list in the UI, `--translate` path) | Closes the single biggest feature gap vs. nearly the entire field; unlocks a global audience. Plumbing largely exists (`--language` already threaded). | **M** |
| **3** | **First-run onboarding wizard** (guided mic pick → model download → optional LLM setup, with the SmartScreen heads-up) | Cheap, big activation win; compensates for the install friction and the multi-step setup that local/LLM features require. | **S–M** |
| **4** | **Custom / arbitrary local model selection** (scan the models folder, allow a user-supplied `ggml` `.bin`, expose language/quantization) | Matches MacWhisper/Vibe/Buzz/Whispering/OpenWhispr; the obvious power-user ask (already on the FAQ backlog); storage plumbing exists. | **M** |
| **5** | **Spoken punctuation + basic dictation/voice-edit commands** ("new line", "comma", "scratch that", "select that") | Closes a baseline expectation that Wispr/Aqua/Dragon/Voice Access all meet; complements the existing transform feature. | **M–L** |
| **6** | **Real-time streaming insertion** (show partial text in the field as you speak, building on the incremental pipeline) | "Feels instant/modern" parity with Talon and the cloud tools; leverages the partial-transcript events that already exist. Harder under the clipboard-paste model. | **L** |
| **7** | **GPU whisper build as an optional download** | Speeds up larger models / long dictations; matches Vibe/Buzz/OpenWhispr marketing. Lower urgency for short push-to-talk on small models. | **M–L** |
| **8** | **(Optional, if positioning expands) file-transcription depth** — diarization, batch/folder, URL import | Only if Scribe wants to also compete with Vibe/MacWhisper as a media-transcription tool; otherwise skip to stay focused. | **L** |

**Recommended near-term sequence:** **1 → 3 → 2** first — signing and onboarding
fix the *first-impression* funnel cheaply, and multilingual is the headline
feature unlock that rides mostly-existing plumbing. **4** and **5** follow as the
power-user / parity round; **6**, **7**, **8** are larger bets to schedule once
the funnel and language gaps are closed.

---

## 7. Caveats

- Competitor pricing, platform support, and feature splits change frequently;
  the data here was gathered in June 2026 from vendor sites, repos, and reviews.
  Re-verify before quoting any of it externally.
- A few competitor cells are best-effort judgments (e.g. how "real-time" a tool's
  insertion feels, or how deep its history/export is) rather than spec sheets.
- Scribe rows reflect a direct read of the 0.5.18 source tree; if features land
  after 0.5.18, update §3–§6 to match.
