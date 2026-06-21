# GPU Vulkan Spike — the 5-minute hardware test (RX 7800 XT)

> **One question this answers:** does a Vulkan-built whisper.cpp **detect and
> actually run on the discrete Radeon RX 7800 XT** on Windows, and is it
> **materially faster** than CPU on a large model? Everything else (does it build,
> does the Vulkan code path work, does CPU fallback work) is already proven in CI
> by `.github/workflows/gpu-spike.yml` — this is the only part that needs the
> physical GPU. No building required: you download CI's pre-built binaries.

See [`GPU_VULKAN_BUILD_PLAN.md`](GPU_VULKAN_BUILD_PLAN.md) §0 for why this is the
only remaining gate.

## Prereqs (you almost certainly already have these)

- **AMD Adrenalin driver** installed (ships `vulkan-1.dll` — the Vulkan loader).
- A whisper `ggml` model on disk. Use one Scribe already downloaded (Models tab →
  the files live under `%APPDATA%\com.natkins.scribe\` or the models folder shown
  in Data & Privacy), or download `large-v3-turbo` for the real speed test (it's
  the model GPU acceleration is *for*). The artifact bundles `jfk.wav` as the test
  clip.

## Step 1 — Download the CI-built Vulkan binaries

From the latest green **GPU Vulkan Spike** run (artifact
`whisper-vulkan-windows-v1.9.1`):

```powershell
# in the scribe repo, with gh authed:
gh run list --workflow "GPU Vulkan Spike" --limit 3
gh run download <run-id> -n whisper-vulkan-windows-v1.9.1 -D $HOME\Downloads\whisper-vulkan
cd $HOME\Downloads\whisper-vulkan
dir   # whisper-cli.exe, whisper-server.exe, *.dll incl. ggml-vulkan.dll, jfk.wav
```

(Or grab it from the run's **Artifacts** section in the GitHub Actions web UI.)

## Step 2 — Does the 7800 XT get detected + engaged?

```powershell
# point -m at a model you have; jfk.wav is in the folder
.\whisper-cli.exe -m C:\path\to\ggml-large-v3-turbo.bin -f .\jfk.wav -nt
```

**Look for these lines in the output (this is the whole test):**

```
ggml_vulkan: Found 1 Vulkan devices:
ggml_vulkan: 0 = AMD Radeon RX 7800 XT (...) | ...
whisper_backend_init_gpu: using Vulkan0 backend     <-- GPU is ENGAGED
```

- ✅ **Device named "...7800 XT" + "using Vulkan0"** → detection confirmed. Go to
  Step 4.
- ⚠️ **`Found 0 Vulkan devices`** → driver/loader issue; confirm Adrenalin is
  current and `vulkaninfo --summary` lists the card. Try the `v1.8.6` artifact
  (re-run the workflow with `whisper_ref=v1.8.6`) before concluding.
- ⚠️ **Found the card but `using CPU` / no `init_gpu`** → silent fallback; capture
  the full output and check it against the §5b notes.

## Step 3 — If there are TWO Vulkan devices (likely: dGPU + integrated)

Your box exposes both the discrete 7800 XT and the CPU's integrated Radeon. ggml
may pick the weak one. Pin the discrete card by **name/index** with the
ecosystem-standard env var, then re-run Step 2:

```powershell
# whichever index is the 7800 XT in the "Found N Vulkan devices" list (often 0 or 1)
$env:GGML_VK_VISIBLE_DEVICES = "0"
.\whisper-cli.exe -m C:\path\to\ggml-large-v3-turbo.bin -f .\jfk.wav -nt
```

Confirm the engaged device name is the **7800 XT**, not the integrated Radeon.
(In Scribe this becomes WS4: enumerate, prefer the non-integrated / largest-VRAM
device by **name**, set this env var on the spawned `whisper-server.exe`.)

## Step 4 — Is it actually faster? (the payoff)

Time the same clip GPU vs forced-CPU on a *large* model (small models are already
fast on CPU — they won't show the win):

```powershell
$m = "C:\path\to\ggml-large-v3-turbo.bin"
Measure-Command { .\whisper-cli.exe -m $m -f .\jfk.wav -nt } | Select TotalSeconds          # GPU
Measure-Command { .\whisper-cli.exe -m $m -f .\jfk.wav -nt --no-gpu } | Select TotalSeconds  # CPU
```

Watch **Task Manager → Performance → GPU (Compute)** during the GPU run — utilization
should spike. Latency alone isn't proof (silent CPU fallback is a known mode), so
rely on the `using Vulkan0` line *and* visible GPU utilization.

## Results — fill in and paste back

| check | result |
|---|---|
| whisper ref tested | `v1.9.1` (artifact) |
| `Found N Vulkan devices` | N = ___ |
| device name engaged | ___ |
| `using Vulkan0` present? | yes / no |
| GPU utilization rose in Task Mgr? | yes / no |
| needed `GGML_VK_VISIBLE_DEVICES`? | yes (=___) / no |
| GPU time (large-v3-turbo, jfk.wav) | ___ s |
| CPU time (`--no-gpu`, same) | ___ s |
| transcript GPU == CPU? | yes / minor diff / no |

## What each outcome means

- **Detected + engaged + faster** → green light. Proceed to **Option A** (single
  Vulkan build replaces the CPU set). The CI build *is* WS1; remaining work is WS4
  (name-based device selection), WS5 (GPU→CPU fallback hardening), WS6 (Auto/On/Off
  toggle + GPU-status UI row). ~2–3 days.
- **Detected but needs the env var** → still Option A; just bake the device pin
  into the server spawn (WS4).
- **Not detected on v1.9.1** → re-run the workflow with `whisper_ref=v1.8.6`
  (Scribe's current CPU pin) and `v1.7.6` (legacy). If none engage the discrete
  card on Windows/Adrenalin, the feature parks — but #3455 being fixed in v1.8.1
  makes this unlikely.
