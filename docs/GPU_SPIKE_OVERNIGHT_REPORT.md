# Overnight Report — GPU Vulkan spike (CI-first) + 0.5.24 ship

**Date:** 2026-06-21 (overnight)
**Branch:** `feat/gpu-vulkan-spike` (GPU work) · `main` (the 0.5.24 release)
**Asked:** "do the recommended path [for the GPU build] while I sleep" + "deal with
the bug commit everything" + "[test] in CI".

---

## TL;DR

1. **0.5.24 shipped green.** The orphan UI-polish commit that was sitting on `main`
   but in no release is now released (`v0.5.24`, all assets published).
2. **The GPU spike is now CI-first**, which is a better path than the original doc:
   CI **builds the Vulkan binaries from source (green)**, proves the **CPU-fallback
   path is clean** (correct transcript, no crash, no missing-DLL), and **ships the
   binaries as an artifact** (incl. `ggml-vulkan.dll`, + `jfk.wav`) — so the only
   step left for you is a **~5-minute test on the 7800 XT** (no building). (The
   in-CI "Vulkan actually engages" check couldn't run — hosted runners have no
   usable Vulkan device even with a software rasterizer; that proof *is* the
   hardware test.)
3. **The research materially de-risked the feature**: the AMD-not-detected
   regression (#3455) that drove the whole risk profile was **fixed upstream in
   whisper.cpp v1.8.1** — so Scribe's pinned v1.8.6 and the latest v1.9.1 both have
   the fix.

---

## ✅ Done & shipped (on `main`)

- **v0.5.23** — confirmed complete and green from the prior session (the
  reliability/bug-fix pass).
- **v0.5.24** — cut tonight to ship the post-tag commit `023f9ad` (toast
  timer-leak fix, error-toast `role="alert"` a11y, `RecordingResult` type honesty).
  Version bumped in all 3 files + `Cargo.lock`, CHANGELOG added, `tsc` clean.
  **Release published green** — `latest.json`, `setup.exe`, `.sig`, `.msi` all
  attached. Nothing on `main` is unreleased anymore.

## ✅ Done (on `feat/gpu-vulkan-spike`)

- **Research refresh** (folded into `GPU_VULKAN_BUILD_PLAN.md` §0):
  - #3455 (AMD Vulkan not detected) **fixed in v1.8.1**; multiple AMD users
    confirmed `using Vulkan0`. The §5b "#1 gate" is largely retired upstream.
  - Latest whisper.cpp is **v1.9.1**. Candidate refs pinned by **commit SHA**
    (v1.9.1 / v1.8.6 / v1.7.6).
  - No trustworthy prebuilt Vulkan-Windows binary (community repo is stale) →
    build-from-source confirmed.
- **CI spike workflow** `.github/workflows/gpu-spike.yml` — on `windows-latest`:
  installs the Vulkan SDK + Lavapipe, builds whisper.cpp from source with
  `-DGGML_VULKAN=ON` (MSVC + Ninja), runs whisper-cli (GPU-attempt + `--no-gpu`),
  and **uploads the built Vulkan binaries** as an artifact.
- **CI status: green** (run `27905040000`; first green build `27904821083`).
  - ✅ **Build from source works** — artifact `whisper-vulkan-windows-v1.9.1`
    (~24 MB) contains `whisper-cli.exe`, `whisper-server.exe`, `whisper.dll`,
    `ggml.dll`, `ggml-base.dll`, `ggml-cpu.dll`, **`ggml-vulkan.dll`** (73.8 MB,
    shaders embedded), and `jfk.wav`.
  - ✅ **CPU fallback is clean** — with no Vulkan device, whisper-cli falls back to
    CPU, transcribes correctly, no crash, no missing-DLL. This is the Option A
    no-GPU-machine gate, and it **passed**.
  - ⚠️ **In-CI Vulkan-engage not demonstrable** — `ggml_vulkan: No devices found`
    even with Lavapipe installed + `VK_DRIVER_FILES` pointed at the registered ICD.
    Software Vulkan doesn't come up as a usable device on the hosted runner. This
    is a CI-environment limit, **not** a whisper-build problem — the real
    engagement proof is the 7800 XT test below.
  - Iterations to green: (1) replaced a full-`C:\`-drive `*.json` scan that hung
    the runner; (2) switched the cmake VS generator (not found on the runner) to
    MSVC-env + Ninja; (3) two tries to register Lavapipe (gave up — see above).
- **Docs:** `GPU_VULKAN_BUILD_PLAN.md` §0 (research + CI-first plan + review
  refinements); `GPU_VULKAN_SPIKE.md` (the turnkey hardware runbook).

---

## 👉 Your 5-minute morning action (the one hardware gate)

CI can't reach your discrete Radeon (hosted runners have no GPU). This is the only
step that needs the actual 7800 XT. Full steps in
[`GPU_VULKAN_SPIKE.md`](GPU_VULKAN_SPIKE.md); the short version:

```powershell
# 1. download CI's pre-built Vulkan binaries (no local build needed)
gh run list --workflow "GPU Vulkan Spike" --limit 3
gh run download <run-id> -n whisper-vulkan-windows-v1.9.1 -D $HOME\Downloads\whisper-vulkan
cd $HOME\Downloads\whisper-vulkan

# 2. run on a large model + the bundled jfk.wav, look for "using Vulkan0"
.\whisper-cli.exe -m C:\path\to\ggml-large-v3-turbo.bin -f .\jfk.wav -nt
```

Confirm: `ggml_vulkan: Found ... 7800 XT` + `whisper_backend_init_gpu: using
Vulkan0`, GPU utilization rises in Task Manager, and it's faster than `--no-gpu`.
Fill in the results table in the runbook and we pick the implementation path.

---

## ❌ What I could not do (and why)

- **Confirm the discrete 7800 XT engages / measure the speedup.** Needs the
  hardware + Adrenalin Vulkan ICD + eyes on GPU utilization. A WSL build would use
  a *different* Vulkan stack (D3D12/Dozen) and wouldn't be a valid gate. → that's
  the morning action above.

## Remaining work after the gate (if it's green)

**Option A** (single Vulkan build replaces the CPU set) — most of **WS1 is already
done** (the CI build *is* WS1; adapt that build step into `release.yml`, replacing
the CPU-only binary fetch). Then:

- **WS4** — `gpu_acceleration` setting (Auto/On/Off) + name-based device selection
  (`GGML_VK_VISIBLE_DEVICES`, pick the discrete card by name, not a fixed index).
- **WS5** — extend the existing server→CLI fallback so a GPU failure retries on CPU.
- **WS6** — one Models/Audio UI row: GPU status + the toggle.
- Estimate: **~2–3 days** after a green hardware gate.

## Housekeeping noted (not done — out of tonight's scope)

- `docs/HANDOFF.md` and `docs/STATUS_AND_NEXT_STEPS.md` still say "0.5.22" — they
  predate 0.5.23/0.5.24 and want a refresh (on `main`).
- **Decide on the branch:** `feat/gpu-vulkan-spike` holds the spike workflow + GPU
  docs. The workflow is exploratory; the docs are worth keeping. Merge the docs to
  `main` (and keep or drop the workflow) once you've run the hardware gate.

## Commit map (this session)

- `main`: `23f21c3` (0.5.24 release) ← `023f9ad` (the shipped UI-polish commit).
- `feat/gpu-vulkan-spike`: spike workflow + iterations, `GPU_VULKAN_BUILD_PLAN.md`
  §0, `GPU_VULKAN_SPIKE.md`, this report.
