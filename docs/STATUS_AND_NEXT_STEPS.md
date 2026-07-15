# Scribe status and next steps

Status: Shipping as a public MIT-licensed project.

Current release: **0.8.0**, published on 2026-07-14.

Repository: <https://github.com/n8watkins/scribe>

Website: <https://scribe.n8builds.dev>

## Current product state

Scribe is a local-first Windows 10 and Windows 11 dictation application built with Tauri, React, TypeScript, Rust, SQLite, and whisper.cpp.
Core transcription runs locally, requires no account, and supports push-to-talk, toggle dictation, incremental transcription, a configurable floating pill, clipboard-preserving insertion, searchable history, notes, exports, multilingual models, and translation to English.
The bundled whisper.cpp runtime uses Vulkan acceleration when available and automatically falls back to CPU execution.

Optional local-LLM features provide note analysis and selected-text transformation.
The former per-dictation LLM cleanup path was removed because it duplicated deterministic cleanup and added latency.
Pause-aware filler suppression is implemented locally and remains optional.

Optional GitHub backup writes notes and, when selected, transcripts to an existing private repository controlled by the user.
The GitHub App uses Device Flow and repository Contents permission.
Scribe does not request repository-administration permission and does not create repositories automatically.

Automatic update checks run shortly after launch, when the window regains focus, and every six hours.
Updater artifacts are cryptographically signed so installed clients can reject tampered updates.
Windows executables and installers are intentionally not Authenticode-signed.

## Release health

The v0.8.0 release workflow validates version metadata, JavaScript dependencies, Rust code, the pinned whisper.cpp source, Vulkan binaries, installer contents, checksums, and the software bill of materials.
It smoke-tests both NSIS and MSI installers by installing, launching, and uninstalling each candidate before publication.
Published assets are downloaded and checked against `SHA256SUMS.txt` after release creation.

## Remaining opportunities

- Allow users to select an arbitrary compatible local Whisper model instead of only the curated catalog.
- Add literal spoken punctuation and voice-editing commands.
- Explore inserting stable transcript segments before dictation stops.
- Add a focused first-run onboarding flow.
- Consider FTS5 if transcript collections outgrow SQL `LIKE` search.
- Revisit automatic private-repository creation only if its additional GitHub administration permission and user experience are explicitly approved.

Authenticode signing is not planned.
Google Drive backup was replaced by private GitHub backup.
macOS and Linux support are not current goals.

## Documentation note

The [documentation index](README.md) identifies current operating guidance and historical project records.
Older plans remain useful for design rationale, but their version numbers, paths, status labels, and proposed work are not current.
