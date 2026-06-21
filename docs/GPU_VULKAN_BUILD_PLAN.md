# GPU Acceleration via Vulkan — Build Plan (design only)

> **Status: design + spike in progress.** This outlines *how* a GPU-accelerated
> whisper build would work in Scribe and what it would take. The §0 update below
> supersedes the more cautious framing in §5b/§8 — read it first. See the
> open-items list in [`STATUS_AND_NEXT_STEPS.md`](STATUS_AND_NEXT_STEPS.md), the
> turnkey hardware test in [`GPU_VULKAN_SPIKE.md`](GPU_VULKAN_SPIKE.md), and the
> CI spike workflow `.github/workflows/gpu-spike.yml`.

## 0a. Implementation status (2026-06-21) — Option A built

Option A is **implemented** on `feat/gpu-vulkan-spike` (hardware gate already
green, see §0). Shipped in this branch:

- **WS1** — `release.yml` builds whisper.cpp from source with `-DGGML_VULKAN=ON`
  (Vulkan SDK + MSVC + Ninja, pinned to `v1.9.1`) and bundles the Vulkan binaries
  incl. `ggml-vulkan.dll` into `resources/bin/windows`, replacing the CPU-only
  prebuilt fetch. `ggml-cpu.dll` stays for no-GPU machines.
- **WS4** — `gpu_acceleration` setting (`Auto`/`Off`, default `Auto`, serde-default
  so existing installs adopt the GPU) + `gpu_device_index` pin; shared
  `push_gpu_args` (`--no-gpu`) and `GGML_VK_VISIBLE_DEVICES` env across the CLI and
  warm-server paths; a best-effort Vulkan device **probe** (`gpu.rs`, parser
  unit-tested vs the real 7800 XT output) behind a `probe_gpu_devices` command.
- **WS5** — a whisper-cli GPU failure retries once on CPU (driver crash / VRAM OOM
  degrades to slower-but-working, never loses the dictation).
- **WS6** — Audio view "GPU acceleration" panel: probe-backed status, Use-GPU
  toggle, and a device picker shown only on multi-GPU boxes.
- **Catalog/QoL** — full fp16 `large-v3-turbo` added next to the q5_0 default; an
  English/Multilingual/All filter in the model browser.

Installer-size impact (measured from the green branch build vs the CPU-only
v0.5.24 release): **the download barely grows.** `ggml-vulkan.dll` is ~74 MB
uncompressed, but its SPIR-V shaders compress extremely well under NSIS/lzma:

| | CPU-only v0.5.24 | this Vulkan build | delta |
|---|---|---|---|
| `setup.exe` (download + auto-update) | 5 MB | 12 MB | **+7 MB** |
| MSI | 7 MB | 30 MB | +23 MB |
| on-disk `ggml-vulkan.dll` | — | 74 MB | +74 MB |

So the **download** (the NSIS setup.exe the updater uses) only grows ~7 MB; the
~74 MB is the on-disk footprint. That makes **Option A acceptable** — no pivot to
Option B needed. (If on-disk footprint ever matters, Option B's WS4/5/6 all carry
over.)

Verification: 220 backend lib tests pass; frontend tsc + build clean; CI's
Windows `cargo check --all-targets` green; full `release.yml` build (Vulkan from
source + installer) validated via `workflow_dispatch`; the GPU build was installed
and smoke-tested on the RX 7800 XT.

**Measured speedup (large-v3-turbo q5_0, jfk.wav 11s, on the 7800 XT):**

| | encode | total |
|---|---|---|
| CPU (`--no-gpu`) | 11,065 ms | 11,935 ms |
| GPU (Vulkan0) | 126 ms | 746 ms |
| speedup | ~88× | ~16× |

Identical transcript both ways (no accuracy regression). On CPU large-v3-turbo
runs ~real-time (unusably slow for dictation); on the GPU it's ~15× faster than
real-time. Option A is validated end-to-end.

## 0. Update 2026-06-21 — research refresh + CI-first approach

Re-research + the "can we test in CI?" question reshaped the plan. Net: **the
risk profile is materially better than §5b assumed, and most of the validation is
CI-automatable. Only one step truly needs the RX 7800 XT.**

**The §5b #1 gate is largely retired upstream.** Issue **#3455 (AMD GPU not
detected with Vulkan, the regression that drove the whole §5b risk table) was
fixed in whisper.cpp `v1.8.1`** (issue closed 2025-10-14; PR #3469). Multiple AMD
users confirmed `ggml_vulkan: Found 1 Vulkan devices` + `whisper_backend_init_gpu:
using Vulkan0`. So Scribe's currently-pinned **`v1.8.6` already contains the fix**,
and so does the latest **`v1.9.1`**. The confirmations were on *integrated* AMD
(RADV/Linux), not a *discrete* RDNA3 card on Windows/Adrenalin — so empirical
confirmation on the 7800 XT is still the closing step, but the *regression itself
is gone*, not an open unknown.

**Latest release is `v1.9.1` (2026-06-19).** Candidate refs for the spike, pinned
by commit SHA (pin SHAs, not tags — see review note below):

| ref | SHA | why |
|---|---|---|
| `v1.9.1` | `f049fff95a089aa9969deb009cdd4892b3e74916` | latest; primary spike target |
| `v1.8.6` | `23ee03506a91ac3d3f0071b40e66a430eebdfa1d` | **what Scribe ships for CPU today** — using it for Vulkan guarantees identical accuracy |
| `v1.7.6` | `a8d002cfd879315632a579e73f0148d06959de36` | pre-regression legacy known-good (fallback only) |

**No trustworthy prebuilt Vulkan-Windows binary exists.** Official releases are
CPU+CUDA only (#3673, still open). The community repo
`jerryshell/whisper.cpp-windows-vulkan-bin` is stale (`v1.0.0`, Aug 2025 —
predates the AMD fix). **Build from source in CI** is confirmed as the path (WS1).

**CI-first split (the "test in CI" answer).** What CI on `windows-latest` can and
cannot prove:

- ✅ **WS1 build-from-source** with `-DGGML_VULKAN=ON` — **done & green** (run
  `27905040000`). Retires the "can we even build it" risk; the artifact
  `whisper-vulkan-windows-v1.9.1` (incl. `ggml-vulkan.dll`) is ready to download.
- ✅ **Forced-CPU + no-GPU fallback** — **proven**. With no Vulkan device,
  whisper-cli falls back to CPU, transcribes correctly, no crash, no missing-DLL.
  This is the Option A no-GPU-machine gate.
- ⚠️ **Vulkan backend *engages* (in CI)** — attempted via a **Lavapipe** software
  device but **could not be demonstrated**: `ggml_vulkan: No devices found` even
  with `VK_DRIVER_FILES` pointed at the registered ICD. Software Vulkan doesn't
  come up as a usable device on a hosted runner. Not a build problem — folded into
  the hardware test below. (A self-hosted runner on the Windows box would close
  this "in CI".)
- ✅ **Discrete 7800 XT detection / engagement** — **DONE & GREEN (2026-06-21).**
  Ran the CI artifact on the box: Vulkan found 2 devices, ggml auto-selected
  device 0 = the discrete 7800 XT (`fp16/bf16`, `KHR_coopmat`) over device 1 = the
  integrated Radeon (no matrix cores), `using Vulkan0 backend`, correct transcript.
  Still open: the **speed number** on `large-v3-turbo` (base.en is too small to
  show the win). Implication for WS4: pin the discrete card **by name**, since the
  device-index order isn't guaranteed (index 1 is the slow iGPU).

**Revised recommended path:** (1) CI builds the Vulkan binaries + proves
build/engage/parity/fallback overnight (workflow `gpu-spike.yml`); (2) maintainer
downloads the artifact and runs the one-liner on the 7800 XT to confirm detection
+ a real speedup on `large-v3-turbo`; (3) if green, proceed to Option A — most of
WS1 is already done (the CI build *is* WS1), leaving device selection (WS4),
fallback hardening (WS5), and the settings toggle/UI (WS6).

**Review refinements (carried into the sections below):**

- **Pin the whisper.cpp build to a commit SHA, not a tag.** Tags have shifted
  behavior mid-line before (#3455 lived inside the `v1.8.x` line).
- **Select the GPU by device name/type, never a persisted index** (WS4). Indices
  aren't stable across driver updates / reboots / monitor changes; enumerate and
  prefer the non-integrated, largest-VRAM device, resolving to an index right
  before spawn.
- **Soften the accuracy criterion** (§6) from "byte-identical CPU vs GPU" to "no
  accuracy regression on a small benchmark set" — backends can differ by a float
  ULP and pick a different token on greedy ties.
- **VRAM contention is unaddressed** — if the local LLM (notes analysis /
  dictation cleanup) also uses the GPU, it contends with `large-v3-turbo` for the
  16 GB. Out of scope for v1, but a known, not a surprise.

## 1. Goal & why Vulkan

Let users run the **large, most-accurate models** (`large-v3-turbo`, `medium`)
at usable latency. On CPU those models are slow; on a GPU they're near-realtime.
The small/base/tiny models are already fast on CPU, so this is specifically about
unlocking the high-accuracy tier without the wait.

**Backend choice: Vulkan**, not CUDA.

| | CUDA | **Vulkan (recommended)** |
|---|---|---|
| GPUs covered | NVIDIA only | **NVIDIA + AMD + Intel** (one build) |
| Runtime size | large — cuBLAS/cudart DLLs, ~100s of MB | small — `ggml-vulkan.dll` (a few MB) + the system `vulkan-1.dll` (ships with the GPU driver) |
| Version sensitivity | high (CUDA toolkit/driver coupling) | low |
| Peak speed | highest | slightly below CUDA, still a huge win over CPU |

One Vulkan build benefits every modern GPU owner, at a fraction of CUDA's
footprint. If we ever want absolute peak NVIDIA performance later, a CUDA pack
can be added as a second optional download — but Vulkan is the right first (and
likely only) backend.

**On AMD — the dev/owner box (Radeon RX 7800 XT, RDNA3, 16 GB) — Vulkan isn't
just recommended, it's the only practical path.** CUDA is NVIDIA-only and does
nothing on a Radeon; AMD's ROCm/HIP compute stack is effectively unavailable for
whisper.cpp on *Windows* (it's Linux-focused with limited consumer-card support).
Vulkan is the sole mature GPU route for AMD on Windows. RDNA3 supports Vulkan 1.3
fully, the loader (`vulkan-1.dll`) already ships with the Adrenalin driver, and
16 GB VRAM comfortably holds `large-v3-turbo`. This makes the §3 "no-GPU
fallback" risk moot *for the owner's box* (a GPU is present) but it still gates
the choice for users on machines with no GPU at all.

## 2. Current state (what we're changing)

- Scribe bundles the **CPU** whisper.cpp binaries under `$RESOURCE/bin/windows/`:
  `whisper-server.exe`, `whisper-cli.exe`, and DLLs `whisper.dll`, `ggml.dll`,
  `ggml-base.dll`, `ggml-cpu.dll` (fetched in `release.yml`,
  `WHISPER_CPP_VERSION = v1.8.6`).
- The backend resolves an executable via `whisper::resolve_bundled_executable(app,
  name)` → `resource_dir/bin/windows/<name>` (`whisper.rs:105`). The warm path
  spawns `SERVER_EXECUTABLE = "whisper-server.exe"` (`whisper_server.rs:38`) and
  **already falls back to `whisper-cli.exe` on any server failure**.
- Model files (the curated catalog) download at runtime via `model_manager`
  (download registry, progress, cancel) — the pattern an optional binary-pack
  download would mirror.

## 3. The key design decision: one build or two?

A Vulkan-enabled ggml build **still contains the CPU backend and auto-selects it
at runtime when no Vulkan device is present.** That makes a Vulkan build a
*superset* of the CPU build, which gives us two shapes:

> ⚠️ **The fallback direction is the easy one. The real risk is the opposite:**
> getting Vulkan to *actually engage* on AMD/Windows instead of silently running
> on CPU. The whisper.cpp v1.8.x line has a known AMD-GPU-not-detected regression
> and several silent-CPU-fallback reports — see the researched risk table in
> **§5b**, which is now the most important part of this plan.

### Option A — Single Vulkan build replaces the CPU set *(recommended, simplest)*
Ship the Vulkan-enabled `whisper-server.exe`/`whisper-cli.exe` + `ggml-vulkan.dll`
as the *only* binary set. ggml picks the GPU when available, CPU otherwise. No
optional-download machinery, no binary-set switching — the runtime code is
**unchanged**. Cost: base installer grows by a few MB (`ggml-vulkan.dll`), and a
one-time SPIR-V shader warm-up on the first GPU transcription.

- **Risk to close first:** a machine with *no* Vulkan loader at all (no GPU
  driver / very old Windows) — confirm the Vulkan build still loads and runs on
  CPU there (delay-loaded `vulkan-1.dll`), or keep the CPU `ggml-cpu.dll` present
  as the guaranteed fallback. This is the single most important thing to verify
  before choosing Option A.

### Option B — CPU stays the base, Vulkan is an optional download
Keep today's CPU set as the install default; offer a "GPU acceleration" pack that
downloads the Vulkan binaries into a separate `bin/windows-vulkan/` dir, selected
at runtime when present. Smaller base install, fully opt-in, zero risk to
non-GPU users — at the cost of the download mechanism + binary-set switching
(WS2/WS3 below).

**Recommendation:** start by validating Option A's no-GPU fallback. If it holds,
Option A is dramatically less code. Fall back to Option B only if base-installer
size or the no-loader risk proves unacceptable.

## 4. Workstreams

### WS1 — Produce the Vulkan binaries (CI) · required for both options
Build whisper.cpp from source with `-DGGML_VULKAN=ON` (cmake) on a Windows
runner — **there is no official prebuilt Vulkan Windows binary** (releases ship
CPU + CUDA only; the request to add one is open, #3673), so this is build-from-
source, not fetch. Install the Vulkan SDK on the runner; output the GPU
`whisper-server.exe`/`whisper-cli.exe` + `ggml-vulkan.dll`, keep the shared/DLL
layout (so `ggml-cpu.dll` stays present for fallback and we dodge the static-link
registration bug #3750 — see §5b), add it as a `release.yml` step, surface a
checksum. **Do not assume `v1.8.6` works on AMD** (regression #3455) — the chosen
version must be the one validated by the spike in §5b. Pin both the whisper.cpp
ref and the Vulkan SDK version.

### WS2 — Optional-download mechanism *(Option B only)*
Mirror `model_manager`: a registry entry for the "vulkan-pack", download with
progress/cancel, checksum verify, unzip into `bin/windows-vulkan/`, and a
persisted "installed" flag. Reuse the existing download UI patterns from Models.

### WS3 — Runtime binary-set resolution *(Option B only)*
Teach `resolve_bundled_executable` (and the server spawn) to prefer
`bin/windows-vulkan/<name>` when the pack is installed and GPU use is enabled,
else `bin/windows/<name>`. One resolution helper, threaded through the server +
CLI paths. (Option A needs none of this.)

### WS4 — GPU detection, device selection + settings toggle
A `gpu_acceleration` setting (`Auto` / `On` / `Off`, default `Auto`). `Auto`
uses the GPU when a Vulkan device is detected. Add an `llm_status`-style probe
that reports whether a usable Vulkan device exists, for the UI to show "GPU:
detected / not found". whisper.cpp's `--no-gpu` flag forces CPU when `Off`.

**Multi-adapter selection (required, not optional).** Many machines expose more
than one Vulkan device — the owner's box has *two*: the discrete Radeon RX 7800
XT **and** the CPU's integrated Radeon Graphics. ggml picks a device index by
default and can land on the weak iGPU. The mechanism is settled (see §5b): set
the **`GGML_VK_VISIBLE_DEVICES`** env var on the spawned `whisper-server.exe` to
pin the discrete card (commonly index 1; enumerate once to confirm and prefer the
non-integrated / largest-VRAM device). Surface the chosen device name in the UI,
and ideally let the user pick when more than one is present. Getting this wrong
silently runs on the iGPU and looks like "GPU acceleration barely helped."

### WS5 — Fallback hardening
GPU init failure (driver crash, OOM on a huge model) must fall back to CPU, not
fail the dictation — extend the existing server→CLI fallback so a GPU server
failure also retries on the CPU path. Log which backend actually ran.

### WS6 — UI
One row in the Models (or Audio) view: GPU status + the Auto/On/Off toggle, and
for Option B the "Download GPU acceleration (~X MB)" button with progress. No
redesign — matches the existing model-download rows.

## 5. whisper.cpp specifics to verify against `v1.8.6`

- cmake flag spelling (`-DGGML_VULKAN=ON`) and resulting DLL name(s).
- That `whisper-server.exe` uses the GPU by default and honors `--no-gpu`.
- The device-selection flag/env for `v1.8.6` (`--gpu-device` / main-gpu /
  `GGML_VK_VISIBLE_DEVICES`) and how it enumerates a multi-adapter box — verify
  it targets the RX 7800 XT, not the integrated Radeon Graphics (WS4).
- Whether `vulkan-1.dll` is delay-loaded (decides the no-GPU-machine risk in §3).
- First-run shader-compile latency (warm the server once at startup to hide it).
- Confirm the Vulkan build's CPU fallback matches CPU-build accuracy (same model,
  same flags) so output doesn't change based on backend.

## 5b. Known whisper.cpp Vulkan risks (researched 2026-06)

Searching the whisper.cpp issue tracker turned up real, current problems that
reshape the risk profile. **Vulkan-on-Windows in the v1.8.x line is fragile, and
the dominant failure mode is silently running on CPU — not crashing.** The plan
must be driven by this section, not by §1's optimism.

| Risk | Evidence | Disposition for Scribe |
|---|---|---|
| **AMD GPU not detected in the v1.8.x line.** v1.8.0 + Vulkan didn't detect the reporter's AMD GPU; v1.7.6 worked. Resolution **could not be confirmed** either way, and AMD+Vulkan has a cluster of ongoing issues (#3611, #2828, 2026 discussions). **Caveat found on re-check:** that reporter ran *integrated* AMD graphics + ROCm, **not** a discrete RDNA3 card — so this is a reason to **test**, not a confirmed blocker for the 7800 XT. | #3455 (AMD reporter) | **Don't pin `v1.8.6` on faith — verify empirically.** The spike confirms detection on the actual discrete 7800 XT across a couple of refs (e.g. v1.7.6 vs latest **v1.9.1**). Still the #1 gate before WS1. |
| **Vulkan backend silently fails to register on Windows MSVC *static* builds** (swallowed C++ exception in the static-init constructor → CPU-only). | #3750 | **Likely does NOT affect Scribe** — we bundle the standalone `whisper-server.exe`/`whisper-cli.exe` + DLLs (shared/dynamic), not static FFI linking (`whisper-rs-sys`). Keeping the shared/DLL layout sidesteps it. The good news of this review. |
| **Silent CPU fallback even *with* a working GPU** — downstream app saw a Vulkan whisper-cli run on CPU. | chidiwilliams/buzz#1443 | **Reframed on re-check:** whisper auto-uses the GPU once built with Vulkan (no runtime flag), so this is a *symptom*, not an independent flaw — it traces to a root cause: registration failure (#3750, static builds → Scribe likely immune) or non-detection (#3455). The risks therefore **consolidate to one gate: version+hardware detection.** The "verify GPU engaged" acceptance check still applies. |
| **No CLI flag to pick a preferred device** in whisper's examples. | #3205 | **Solved without a patch (validated).** `GGML_VK_VISIBLE_DEVICES` is a confirmed, ecosystem-standard ggml-vulkan env var (shared by llama.cpp/Ollama/whisper) — set it on the spawned `whisper-server.exe` to pick the discrete card (the dGPU is commonly index 1). WS4 is just "set an env var", not a code patch. |
| **AMD-specific Vulkan crashes exist** (RDNA1 buffer-init crash). | #3611 | RDNA3 (7800 XT) is newer and likely fine, but AMD Vulkan isn't bulletproof — real-hardware testing on the 7800 XT is mandatory. |
| **No official prebuilt Vulkan Windows binary** (CPU + CUDA only; request open). | #3673 | Confirms **WS1 builds from source**. Community zips (jerryshell/whisper.cpp-windows-vulkan-bin) and the `Whisper.net.Runtime.Vulkan` NuGet exist but aren't a trustworthy pinned supply chain. |

**Net effect:** Option A's "runtime unchanged, it just works" is the *destination*,
not the *path*. The path runs through a version-compatibility spike on the actual
7800 XT. Do that spike first; everything else is cheap once a version reliably
detects and uses the GPU.

## 6. Acceptance criteria

- **The GPU is provably engaged** (`ggml_vulkan: Found N Vulkan devices` + the
  device shows in the per-op log, and GPU utilization rises during transcription)
  — not inferred from latency alone, because silent CPU fallback is a known mode
  (§5b).
- On a GPU machine, `large-v3-turbo` transcription latency drops materially vs the
  CPU build (measure stop-to-text on a fixed clip).
- On a machine with **no** GPU/driver, dictation still works (CPU), with no crash
  and no missing-DLL error — the gating test.
- Output text for a given clip is identical CPU vs GPU (no accuracy regression).
- `gpu_acceleration = Off` forces CPU; `Auto` uses GPU only when detected.
- A GPU failure mid-session falls back to CPU and still produces the transcript.
- On a multi-GPU box (the owner's discrete RX 7800 XT + integrated Radeon), the
  **discrete** card is the device used — confirmed via the reported device name.

## 7. Effort

Revised up after the §5b research — the earlier estimates assumed Vulkan "just
works" on the pinned version, which the AMD regression (#3455) makes unsafe.

- **Spike (do first):** ~0.5–1 day — build/grab a Vulkan whisper for a couple of
  candidate refs and confirm one actually detects + uses the 7800 XT. Gates
  everything; if no ref works cleanly on AMD/Windows, the feature is parked.
- **Option A:** ~2–3 days *after a green spike* — WS1 (build-from-source CI, incl.
  Vulkan SDK) + WS4 device selection + WS5 + the GPU-engaged verification.
- **Option B:** ~4–5 days — adds WS2 (download) + WS3 (binary-set switching) +
  more UI.

## 8. Open questions

1. **(Gating) Which whisper.cpp ref reliably detects + uses the RX 7800 XT via
   Vulkan on Windows?** `v1.8.6` is suspect (#3455). Answer via the §5b spike
   before any other work.
2. Does the Vulkan build load & run on a no-GPU machine (decides A vs B)?
3. Base-installer size budget — is +a few MB for Option A acceptable?
4. Is `large-v3-turbo` the target model, or also `medium`/multilingual large?
5. ~~Build ourselves or fetch a published asset?~~ **Answered:** no official
   prebuilt Vulkan Windows binary exists (#3673) — build from source in CI.
