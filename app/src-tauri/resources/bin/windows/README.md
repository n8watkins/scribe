# Windows whisper.cpp binary

Place the real `whisper-cli.exe` build here before creating a Windows release:

```text
app/src-tauri/resources/bin/windows/whisper-cli.exe
```

The Tauri bundle config maps the `resources/` directory to the app resource
root, so that file resolves to this runtime path:

```text
$RESOURCE/bin/windows/whisper-cli.exe
```

`src/whisper.rs` resolves exactly that resource path. Do not commit a fake
`.exe`; release builds should fail clearly until the real whisper.cpp binary is
provided.
