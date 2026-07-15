# Windows whisper.cpp binary

Local Windows builds need the reviewed whisper.cpp runtime files in this directory:

```text
whisper-cli.exe
whisper-server.exe
whisper.dll
ggml.dll
ggml-base.dll
ggml-cpu.dll
ggml-vulkan.dll
```

The Tauri bundle config maps the `resources/` directory to the app resource
root, so that file resolves to this runtime path:

```text
$RESOURCE/bin/windows/whisper-cli.exe
```

`src/whisper.rs` and `src/whisper_server.rs` resolve the executable paths under this resource directory.
The official release workflow compiles the pinned whisper.cpp source with Vulkan enabled, stages this complete set, and validates it before packaging.
Do not commit placeholder executables or generated binaries.
