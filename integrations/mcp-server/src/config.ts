/**
 * Runtime configuration: CLI flags + environment, resolved to concrete options.
 * All knobs have safe defaults; the only thing a host usually sets is `--dev`.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { controlFilePath } from "./discovery.js";

export interface Config {
  controlPath: string;
  dev: boolean;
  httpTimeoutMs: number;
  idleTimeoutMs: number;
  minBackoffMs: number;
  maxBackoffMs: number;
  debug: boolean;
}

function numEnv(raw: string | undefined, fallback: number): number {
  if (!raw) return fallback;
  const n = Number(raw);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

function flagValue(argv: string[], name: string): string | undefined {
  const i = argv.indexOf(name);
  if (i !== -1 && i + 1 < argv.length) return argv[i + 1];
  const eq = argv.find((a) => a.startsWith(`${name}=`));
  return eq ? eq.slice(name.length + 1) : undefined;
}

/** Resolve config from process argv + env. */
export function loadConfig(
  argv: string[] = process.argv.slice(2),
  env: NodeJS.ProcessEnv = process.env,
): Config {
  const dev =
    argv.includes("--dev") ||
    env.SCRIBE_MCP_DEV === "1" ||
    env.SCRIBE_MCP_FLAVOR === "dev";

  const explicit = flagValue(argv, "--control") ?? env.SCRIBE_CONTROL_FILE;
  const controlPath = explicit ?? controlFilePath({ dev });

  return {
    controlPath,
    dev,
    httpTimeoutMs: numEnv(env.SCRIBE_MCP_HTTP_TIMEOUT_MS, 5000),
    idleTimeoutMs: numEnv(env.SCRIBE_MCP_IDLE_TIMEOUT_MS, 30_000),
    minBackoffMs: numEnv(env.SCRIBE_MCP_MIN_BACKOFF_MS, 500),
    maxBackoffMs: numEnv(env.SCRIBE_MCP_MAX_BACKOFF_MS, 10_000),
    debug: env.SCRIBE_MCP_DEBUG === "1",
  };
}

/** Read this package's version from its package.json, with a safe fallback. */
export function packageVersion(): string {
  try {
    const here = dirname(fileURLToPath(import.meta.url));
    const pkg = JSON.parse(readFileSync(join(here, "..", "package.json"), "utf8")) as {
      version?: string;
    };
    return pkg.version ?? "0.1.0";
  } catch {
    return "0.1.0";
  }
}
