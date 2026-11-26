CREATE TABLE IF NOT EXISTS transcripts (
  id TEXT PRIMARY KEY,
  text TEXT NOT NULL,
  created_at TEXT NOT NULL,
  duration_ms INTEGER,
  word_count INTEGER NOT NULL,
  character_count INTEGER NOT NULL,
  model_id TEXT,
  language TEXT,
  output_mode TEXT,
  paste_method TEXT,
  transcription_latency_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_transcripts_created_at
  ON transcripts(created_at DESC);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS models (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  filename TEXT NOT NULL,
  local_path TEXT,
  size_bytes INTEGER,
  status TEXT NOT NULL,
  checksum TEXT,
  selected INTEGER DEFAULT 0,
  downloaded_at TEXT
);

CREATE TABLE IF NOT EXISTS app_stats_daily (
  date TEXT PRIMARY KEY,
  dictation_count INTEGER DEFAULT 0,
  word_count INTEGER DEFAULT 0,
  total_recording_ms INTEGER DEFAULT 0,
  total_transcription_latency_ms INTEGER DEFAULT 0
);
