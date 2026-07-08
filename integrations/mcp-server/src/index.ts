#!/usr/bin/env node
/**
 * Entry point: launch the Scribe dictation MCP server over stdio.
 *
 * A host (Claude, another MCP client) spawns this process and speaks MCP on
 * stdin/stdout. All diagnostics go to stderr - stdout is reserved for the
 * protocol. The background SSE subscription starts immediately so the resource
 * is live before the first read.
 */

import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { loadConfig, packageVersion } from "./config.js";
import { StateEngine } from "./engine.js";
import { createServer } from "./server.js";

const SERVER_NAME = "scribe-dictation";

async function main(): Promise<void> {
  const config = loadConfig();
  const version = packageVersion();

  const logger = (msg: string) => {
    if (config.debug) process.stderr.write(`[scribe-mcp] ${msg}\n`);
  };

  const engine = new StateEngine({
    controlPath: config.controlPath,
    httpTimeoutMs: config.httpTimeoutMs,
    idleTimeoutMs: config.idleTimeoutMs,
    minBackoffMs: config.minBackoffMs,
    maxBackoffMs: config.maxBackoffMs,
    log: logger,
  });

  const server = createServer(engine, { name: SERVER_NAME, version });

  // Begin subscribing to Scribe before wiring stdio, so state is warm.
  engine.start();
  logger(
    `starting v${version} (control=${config.controlPath}${config.dev ? ", dev flavor" : ""})`,
  );

  const transport = new StdioServerTransport();
  await server.connect(transport);

  let shuttingDown = false;
  const shutdown = async (signal: string) => {
    if (shuttingDown) return;
    shuttingDown = true;
    logger(`shutting down (${signal})`);
    await engine.stop();
    await server.close().catch(() => {});
    process.exit(0);
  };

  process.on("SIGINT", () => void shutdown("SIGINT"));
  process.on("SIGTERM", () => void shutdown("SIGTERM"));
  // The host closing our stdin signals us to exit.
  process.stdin.on("close", () => void shutdown("stdin-close"));
}

main().catch((err) => {
  process.stderr.write(`[scribe-mcp] fatal: ${(err as Error).stack ?? String(err)}\n`);
  process.exit(1);
});
