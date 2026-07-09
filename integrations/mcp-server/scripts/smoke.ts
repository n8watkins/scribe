/**
 * End-to-end stdio smoke test against the BUILT server (dist/index.js).
 *
 * Spins up a mock Scribe, writes a control file, launches the real server as a
 * child process over stdio (exactly how an MCP host does), then exercises the
 * tools, resource read, and a live subscription. Run: `npm run smoke`
 * (after `npm run build`). Exits non-zero on any failed assertion.
 */

import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { ResourceUpdatedNotificationSchema } from "@modelcontextprotocol/sdk/types.js";
import { MockScribe } from "../test/mock-server.js";
import { controlFor, waitFor, writeControlFile } from "../test/helpers.js";

const here = dirname(fileURLToPath(import.meta.url));
const serverEntry = join(here, "..", "dist", "index.js");

async function main() {
  const mock = new MockScribe({ initial: { status: "Idle" } });
  await mock.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock.baseUrl, mock.token));

  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [serverEntry],
    env: { ...process.env, SCRIBE_CONTROL_FILE: path, SCRIBE_MCP_DEBUG: "1" },
    stderr: "inherit",
  });
  const client = new Client({ name: "smoke-client", version: "0.0.0" });

  let updates = 0;
  client.setNotificationHandler(ResourceUpdatedNotificationSchema, () => {
    updates += 1;
  });

  await client.connect(transport);
  try {
    const tools = (await client.listTools()).tools.map((t) => t.name).sort();
    assert.deepEqual(tools, ["get_dictation_state", "is_dictating"]);
    console.log("✔ tools:", tools.join(", "));

    const resources = (await client.listResources()).resources.map((r) => r.uri);
    assert.ok(resources.includes("scribe://dictation/state"));
    console.log("✔ resource:", resources.join(", "));

    let res = await client.callTool({ name: "get_dictation_state", arguments: {} });
    let s = (res as { structuredContent?: Record<string, unknown> }).structuredContent!;
    assert.equal(s.online, true);
    assert.equal(s.dictating, false);
    console.log("✔ get_dictation_state (Idle):", JSON.stringify({ online: s.online, dictating: s.dictating }));

    await client.subscribeResource({ uri: "scribe://dictation/state" });
    mock.startDictation();
    await waitFor(() => updates >= 1, { timeoutMs: 3000 });
    console.log(`✔ subscription notified on start (${updates} update(s))`);

    res = await client.callTool({ name: "is_dictating", arguments: {} });
    s = (res as { structuredContent?: Record<string, unknown> }).structuredContent!;
    assert.equal(s.dictating, true);
    assert.equal(s.busy, true);
    console.log("✔ is_dictating after start:", JSON.stringify(s));

    mock.stopDictation();
    await waitFor(() => updates >= 2, { timeoutMs: 3000 });
    console.log(`✔ subscription notified on stop (${updates} total update(s))`);

    console.log("\nSMOKE OK");
  } finally {
    await client.close();
    await mock.close();
    await cleanup();
  }
}

main().catch((err) => {
  console.error("SMOKE FAILED:", err);
  process.exit(1);
});
