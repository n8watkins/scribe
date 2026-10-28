// Tauri v2 IPC mock for capturing the Scribe React UI in a plain browser.
// Injected via Playwright addInitScript; scenario comes from window.__SCRIBE_SCENARIO.
(() => {
  const S = Object.assign(
    {
      theme: "midnight",
      modelsDownloaded: true,
      mics: true,
      llmReachable: true,
      github: "connected", // connected | disconnected | unconfigured
      history: "full", // full | empty
      update: false,
      developer: false,
      status: "Idle",
      analysisEnabled: true,
      customTheme: null,
      replacements: true,
    },
    window.__SCRIBE_SCENARIO || {},
  );
  const now = Date.now();
  const iso = (minsAgo) => new Date(now - minsAgo * 60000).toISOString();

  const settings = {
    defaultsVersion: 5,
    launchAtStartup: false,
    minimizeToTray: true,
    showFloatingPill: true,
    pillDisplayMode: S.pillMode || "visualizer_with_text",
    dashboardHotkeyToggles: true,
    notificationsEnabled: true,
    soundsEnabled: true,
    developerSettingsEnabled: S.developer,
    autoUpdateCheckEnabled: true,
    autoInstallUpdates: !S.noAutoInstall,
    theme: S.theme,
    customTheme: S.customTheme || {
      background: "#0b0e14",
      surface: "#151a23",
      accent: "#818cf8",
      text: "#f1f5f9",
      textMuted: "#94a3b8",
    },
    devHotkeysSeeded: false,
    recordingMode: "both",
    minRecordingMs: 300,
    maxRecordingMs: 600000,
    silenceTrimEnabled: true,
    silenceAutoStopEnabled: true,
    silenceAutoStopMs: 60000,
    incrementalTranscriptionEnabled: true,
    segmentPauseEnabled: true,
    segmentPauseMs: 3000,
    segmentMaxMs: 25000,
    selectedMicId: null,
    selectedModelId: "small.en-q5_1",
    language: "en",
    translateToEnglish: false,
    gpuAcceleration: "auto",
    gpuDeviceIndex: null,
    vocabularyPrompt: S.replacements ? "Scribe, Tauri, whisper.cpp, Vulkan, n8builds, Appturnity" : "",
    textReplacements: S.replacements
      ? [
          { from: "new line", to: "\n" },
          { from: "my email", to: "me@example.com" },
          { from: "scribe app", to: "Scribe" },
          { from: "t hub", to: "T-Hub" },
        ]
      : [],
    outputMode: "auto_paste",
    pasteMethod: "clipboard_paste",
    historyEnabled: true,
    saveAudioClips: true,
    historyRetentionDays: 30,
    notesRetentionDays: null,
    notesAnalysisEnabled: S.analysisEnabled,
    notesAnalysisPrompt:
      'Summarize this dictated note in one or two sentences, then list any action items as bullet points. If there are none, say "No action items."',
    notesAnalysisEndpoint: "http://127.0.0.1:1234/v1",
    notesAnalysisModel: S.analysisEnabled ? "gemma-3-4b-it" : "",
    fillerSuppressionEnabled: false,
    fillerWords: ["um", "umm", "uh", "uhh", "er", "erm", "hmm"],
    fillerPauseThresholdMs: 300,
    githubSyncEnabled: S.github === "connected",
    githubRepo: S.github === "connected" ? "n8watkins/scribe-notes" : "",
    githubSyncAllTranscripts: false,
    githubAccountLogin: S.github === "connected" ? "n8watkins" : "",
    hotkeys: {
      holdToTalk: "Ctrl+Win",
      toggleDictation: "Backquote",
      pasteLastTranscript: "Ctrl+Alt+V",
      openDashboard: "Ctrl+Alt+F",
      discardDictation: "Ctrl+Alt+X",
      transformSelection: "Ctrl+Alt+R",
      toggleDictationTrigger: "release",
      pasteLastTranscriptTrigger: "press",
      openDashboardTrigger: "press",
      discardDictationTrigger: "press",
      transformSelectionTrigger: "press",
    },
    pillX: null,
    pillY: null,
    pillColorNormal: "#fbbf24",
    pillColorNote: "#38bdf8",
    pillColorBackground: "#0f1e38",
    dataDir: null,
    windowWidth: null,
    windowHeight: null,
  };

  const texts = [
    ["Can you take a look at the pull request when you get a chance? I think the retry logic in the sync worker is finally right, but I want a second pair of eyes on the backoff numbers.", false, 3],
    ["Remember to book the dentist for early December, and add a calendar reminder a week before so I can move meetings around it.", true, 18],
    ["Hey, thanks for the quick turnaround on the landing page copy. The new hero line reads much better. Let's ship it tomorrow morning after one more pass on mobile.", false, 47],
    ["Idea for Scribe: a settings search box at the top of the Settings view, so you can type paste or hotkey and jump straight to the right card.", true, 95],
    ["The meeting moved to Thursday at two. Please bring the latency numbers from the Vulkan benchmark and the installer size comparison.", false, 160],
    ["Okay so the bug only happens when the microphone is unplugged during a hands free recording. The pill stays red and the next dictation never starts until you restart the app.", false, 300],
    ["Grocery list: oat milk, coffee beans, spinach, two lemons, and the good sourdough from the bakery on Fifth.", true, 1440],
    ["Draft reply to the client: we can deliver the first milestone by the fifteenth if the design sign-off lands this week. Otherwise we would push to the end of the month.", false, 1500],
    ["Write the release notes for zero point nine in plain language. Lead with the security fixes, then the faster model loading, then the smaller fixes.", true, 2900],
    ["Function names should be verbs. Rename user data loader to load user data and keep the old export as a deprecated alias for one release.", false, 4400],
    ["Testing one two three. This is a quick microphone check after switching to the USB headset.", false, 5800],
    ["Book the dentist appointment, call the bank about the card, and pick up the package from the post office before Saturday.", true, 8600],
  ];
  const transcripts =
    S.history === "empty"
      ? []
      : texts.map(([text, isNote, mins], i) => {
          const words = text.split(/\s+/).length;
          return {
            id: `t${i + 1}`,
            text,
            createdAt: iso(mins),
            durationMs: Math.round(words * 380),
            wordCount: words,
            characterCount: text.length,
            modelId: "small.en-q5_1",
            language: "en",
            outputMode: isNote ? "save_only" : "auto_paste",
            pasteMethod: isNote ? null : "clipboard_paste",
            transcriptionLatencyMs: 280 + ((i * 97) % 400),
            audioPath: i < 6 ? `C:\\Users\\you\\AppData\\Roaming\\com.natkins.scribe\\audio\\t${i + 1}.wav` : null,
            isNote,
            analysis:
              isNote && i === 1
                ? "Book a dentist appointment for early December.\n\n- Call the dentist to book a slot\n- Add a calendar reminder one week before"
                : isNote && i === 3
                  ? "Suggests adding a search box to Settings for quick navigation.\n\nNo action items."
                  : null,
            analysisModel: isNote && (i === 1 || i === 3) ? "gemma-3-4b-it" : null,
            analysisCreatedAt: isNote && (i === 1 || i === 3) ? iso(mins - 1) : null,
          };
        });

  const catalog = [
    ["tiny.en", "Tiny English", "ggml-tiny.en.bin", "75 MiB", false],
    ["base.en", "Base English", "ggml-base.en.bin", "142 MiB", false],
    ["small.en", "Small English", "ggml-small.en.bin", "466 MiB", false],
    ["small.en-q5_1", "Small English Q5_1", "ggml-small.en-q5_1.bin", "181 MiB", false],
    ["medium.en", "Medium English", "ggml-medium.en.bin", "1.5 GiB", false],
    ["tiny", "Tiny (Multilingual)", "ggml-tiny.bin", "75 MiB", true],
    ["base", "Base (Multilingual)", "ggml-base.bin", "142 MiB", true],
    ["small", "Small (Multilingual)", "ggml-small.bin", "466 MiB", true],
    ["medium", "Medium (Multilingual)", "ggml-medium.bin", "1.5 GiB", true],
    ["large-v3-turbo-q5_0", "Large v3 Turbo Q5_0 (Multilingual)", "ggml-large-v3-turbo-q5_0.bin", "547 MiB", true],
    ["large-v3-turbo", "Large v3 Turbo (Multilingual)", "ggml-large-v3-turbo.bin", "1.5 GiB", true],
  ];
  const downloadedIds = S.modelsDownloaded ? ["small.en-q5_1", "base.en", "large-v3-turbo-q5_0"] : [];
  const models = catalog.map(([id, name, filename, size, multilingual]) => {
    const dl = downloadedIds.includes(id);
    const sel = id === settings.selectedModelId;
    return {
      id,
      name,
      filename,
      downloadUrl: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${filename}`,
      diskSizeLabel: size,
      localPath: dl ? `C:\\Users\\you\\AppData\\Roaming\\com.natkins.scribe\\models\\${filename}` : null,
      source: dl ? "app_data" : null,
      sizeBytes: dl ? 189000000 : null,
      status: dl ? (sel ? "loaded" : "downloaded") : "not_downloaded",
      checksum: multilingual ? null : { kind: "sha1", value: "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678" },
      selected: sel && dl,
      downloadedAt: dl ? iso(60 * 24 * 40) : null,
      multilingual,
    };
  });

  const mics = S.mics
    ? [
        { id: "default", name: "Microphone (Realtek(R) Audio)", endpointId: "{0.0.1.00000000}.{aa}", isDefault: true, isSelected: true, isAvailable: true },
        { id: "usb", name: "Headset Microphone (HyperX Cloud II)", endpointId: "{0.0.1.00000000}.{bb}", isDefault: false, isSelected: false, isAvailable: true },
      ]
    : [];

  const stats =
    S.history === "empty"
      ? { wordsToday: 0, dictationsToday: 0, averageWpm: null, averageTranscriptionLatencyMs: null, averageRecordingDurationMs: null, mostUsedModel: null, totalWordsTranscribed: 0 }
      : { wordsToday: 412, dictationsToday: 9, averageWpm: 148, averageTranscriptionLatencyMs: 412, averageRecordingDurationMs: 11800, mostUsedModel: "small.en-q5_1", totalWordsTranscribed: 48213 };

  const bindings = Object.entries({
    holdToTalk: "Ctrl+Win",
    toggleDictation: "Backquote",
    pasteLastTranscript: "Ctrl+Alt+V",
    openDashboard: "Ctrl+Alt+F",
    discardDictation: "Ctrl+Alt+X",
    transformSelection: "Ctrl+Alt+R",
  }).map(([action, shortcut]) => ({
    action,
    shortcut,
    normalizedShortcut: shortcut,
    trigger: action === "holdToTalk" ? null : action === "toggleDictation" ? "release" : "press",
    registered: action !== S.failedHotkey,
    error: null,
  }));

  const appState = () => ({ status: S.status, error: S.status === "Error" ? { code: "microphone_unavailable", message: "The selected microphone is unavailable." } : null, updatedAt: new Date().toISOString() });

  const handlers = {
    get_app_state: appState,
    get_settings: () => settings,
    update_settings: (a) => Object.assign(settings, a.settings),
    patch_settings: (a) => Object.assign(settings, a.patch),
    stop_dictation: () => null,
    get_last_transcript: () => transcripts.find((t) => !t.isNote) || null,
    list_recent_transcripts: (a) => transcripts.slice(0, (a && a.limit) || 8),
    search_transcripts: (a) => {
      let list = transcripts.filter((t) => (a.notesOnly ? t.isNote : !t.isNote));
      if (a.query) list = list.filter((t) => t.text.toLowerCase().includes(a.query.toLowerCase()));
      return { transcripts: list, total: list.length, limit: a.limit || 50, offset: a.offset || 0 };
    },
    get_transcript: (a) => transcripts.find((t) => t.id === a.id) || null,
    get_basic_stats: () => stats,
    refresh_basic_stats: () => stats,
    list_models: () => models,
    list_microphones: () => mics,
    get_data_dir: () => "C:\\Users\\you\\AppData\\Roaming\\com.natkins.scribe",
    get_logs_dir: () => "C:\\Users\\you\\AppData\\Roaming\\com.natkins.scribe\\logs",
    get_failed_recordings_dir: () => "C:\\Users\\you\\AppData\\Roaming\\com.natkins.scribe\\failed-recordings",
    llm_status: () =>
      S.llmReachable
        ? { reachable: true, endpoint: settings.notesAnalysisEndpoint, models: ["gemma-3-4b-it", "qwen2.5-7b-instruct"], error: null }
        : { reachable: false, endpoint: settings.notesAnalysisEndpoint, models: [], error: "Connection refused (os error 10061)" },
    probe_gpu_devices: () => ({ available: true, devices: [{ index: 0, name: "AMD Radeon RX 7800 XT", integrated: false }, { index: 1, name: "AMD Radeon(TM) Graphics", integrated: true }], probed: true, recommendedIndex: 0 }),
    github_status: () => ({
      configured: S.github !== "unconfigured",
      connected: S.github === "connected",
      username: S.github === "connected" ? "n8watkins" : "",
      repo: S.github === "connected" ? "n8watkins/scribe-notes" : "",
    }),
    github_sync_activity: () =>
      S.github === "connected"
        ? { completedAt: iso(22), outcome: "success", source: "automatic", repo: "n8watkins/scribe-notes", syncedItems: 5, filesWritten: 6, errorCode: null, errorMessage: null }
        : null,
    get_hotkey_status: () => ({ bindings, noteChordActive: true, holdReleaseVerificationRequired: false, windowsFallbackNote: "" }),
    check_for_update: () => ({ currentVersion: "0.8.0", latestVersion: S.update ? "0.9.0" : "0.8.0", updateAvailable: !!S.update, releaseUrl: "https://github.com/n8watkins/scribe/releases" }),
    whats_new_pending: () => (S.whatsNew ? "0.8.0" : null),
    dismiss_whats_new: () => null,
    "plugin:app|version": () => "0.8.0",
    "plugin:app|name": () => "Scribe",
    "plugin:app|tauri_version": () => "2.9.0",
    "plugin:updater|check": () =>
      S.update
        ? { rid: 1, currentVersion: "0.8.0", version: "0.9.0", date: iso(600), body: "Security fixes and faster model loading.", rawJson: {} }
        : null,
    "plugin:notification|is_permission_granted": () => true,
  };

  const listeners = new Map();
  const callbacks = new Map();
  let nextId = 1;
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    plugins: {},
    transformCallback(cb, once) {
      const id = nextId++;
      callbacks.set(id, (d) => {
        if (once) callbacks.delete(id);
        return cb && cb(d);
      });
      return id;
    },
    unregisterCallback(id) {
      callbacks.delete(id);
    },
    runCallback(id, d) {
      const cb = callbacks.get(id);
      if (cb) cb(d);
    },
    callbacks,
    convertFileSrc: (p) => p,
    async invoke(cmd, args) {
      if (cmd === "plugin:event|listen") {
        if (!listeners.has(args.event)) listeners.set(args.event, []);
        listeners.get(args.event).push(args.handler);
        return args.handler;
      }
      if (cmd === "plugin:event|unlisten") {
        const l = listeners.get(args.event) || [];
        const i = l.indexOf(args.eventId);
        if (i !== -1) l.splice(i, 1);
        return null;
      }
      if (cmd === "plugin:event|emit") return null;
      if (cmd in handlers) return handlers[cmd](args || {});
      if (cmd.startsWith("plugin:")) return null;
      (window.__SCRIBE_UNMOCKED = window.__SCRIBE_UNMOCKED || new Set()).add(cmd);
      return null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, id) {
      const l = listeners.get(event) || [];
      const i = l.indexOf(id);
      if (i !== -1) l.splice(i, 1);
      callbacks.delete(id);
    },
  };
  window.__SCRIBE_EMIT = (event, payload) => {
    for (const h of listeners.get(event) || []) {
      const cb = callbacks.get(h);
      if (cb) cb({ event, id: h, payload });
    }
  };
  window.__SCRIBE_SET_STATUS = (status) => {
    S.status = status;
    window.__SCRIBE_EMIT("scribe:app-state", appState());
  };
  try {
    localStorage.setItem("scribe.theme", S.theme);
  } catch (e) {}
})();
