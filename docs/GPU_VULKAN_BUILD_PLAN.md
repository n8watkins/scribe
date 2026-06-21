# GPU Acceleration via Vulkan — Build Plan (design only)

> **Status: design / not started.** This outlines *how* a GPU-accelerated whisper
> build would work in Scribe and what it would take. No code here is committed to;
> it exists so the work is a concrete, scoped next task. See the open-items list in
> [`STATUS_AND_NEXT_STEPS.md`](STATUS_AND_NEXT_STEPS.md).

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
Build whisper.cpp `v1.8.6` with `-DGGML_VULKAN=ON` (cmake) on a Windows runner,
or consume a published Vulkan asset if one exists for the pinned version. Output
the GPU `whisper-server.exe`/`whisper-cli.exe` + `ggml-vulkan.dll` (keep
`ggml-cpu.dll` for fallback). Add this as a step in `release.yml` (and surface a
checksum). Pin the Vulkan SDK/headers version used to build.

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
XT **and** the CPU's integrated Radeon Graphics. ggml/whisper picks a device
index by default and can land on the weak iGPU. The build must enumerate the
Vulkan devices and target the **discrete** one (pick by device index — e.g.
whisper's `--gpu-device N` / ggml's main-gpu, or the `GGML_VK_VISIBLE_DEVICES`
env filter — and prefer the device with the most VRAM / a non-integrated type).
Surface the chosen device name in the UI, and ideally let the user pick when more
than one is present. Getting this wrong silently runs on the iGPU and looks like
"GPU acceleration barely helped."

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

## 6. Acceptance criteria

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

- **Option A:** ~1–1.5 days — mostly WS1 (CI build) + WS4/WS5 + the no-GPU
  verification. Little-to-no runtime code if the single-build fallback holds.
- **Option B:** ~3–4 days — adds WS2 (download) + WS3 (binary-set switching) +
  more UI.

## 8. Open questions

1. Does the Vulkan build load & run on a no-GPU machine (decides A vs B)?
2. Base-installer size budget — is +a few MB for Option A acceptable?
3. Is `large-v3-turbo` the target model, or also `medium`/multilingual large?
4. Build Vulkan binaries ourselves in CI, or is there a trustworthy pinned
   upstream Vulkan asset for `v1.8.6`?
