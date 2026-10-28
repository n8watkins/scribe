# Scribe assessment, October 2026

A top-to-bottom assessment of Scribe at `main` `1d88fe9` (v0.8.0 plus 15 unreleased commits), run per [ASSESSMENT_PLAN_2026-10.md](../ASSESSMENT_PLAN_2026-10.md).
The merged report, with the ranked top 10, the roadmap and the decisions for the owner, is a hosted page; the link is in the pull request that added this folder.

| File | Lane |
| --- | --- |
| [lane1-code.md](lane1-code.md) | Architecture and code quality, with check results |
| [lane2-security.md](lane2-security.md) | Security, threat model and privacy claims versus behavior |
| [lane3-ux.md](lane3-ux.md) | UI and UX, information architecture, accessibility |
| [lane4-features-perf.md](lane4-features-perf.md) | Feature verification, latency, storage and reliability |
| [lane5-release-notes.md](lane5-release-notes.md) | Release notes in the repo, the app and the website |
| [DECISIONS.md](DECISIONS.md) | What the owner decided after reading the report |
| [screens/mock/](screens/mock/) | 63 screenshots of the React UI with a mocked backend |
| [tools/](tools/) | The IPC mock and capture script, to redo the screenshots after changes |
| [lane3-scroll-metrics.json](lane3-scroll-metrics.json) | Scroll depth per view and window size |

The screenshots use made-up data and Chromium on Linux, so fonts differ slightly from Windows.
Real captures of the installed app (tray, update overlay, pill text mode, SmartScreen) are still to do.
