import assert from "node:assert/strict";
import { test } from "node:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { ResourceUpdatedNotificationSchema } from "@modelcontextprotocol/sdk/types.js";
import { StateEngine } from "../src/engine.js";
import { RESOURCE_URI, createServer } from "../src/server.js";
import { MockScribe } from "./mock-server.js";
import { controlFor, waitFor, writeControlFile } from "./helpers.js";

const FAST = { minBackoffMs: 20, maxBackoffMs: 60, httpTimeoutMs: 1000, idleTimeoutMs: 500 };

async function connectClient(engine: StateEngine): Promise<{ client: Client; close: () => Promise<void> }> {
  const server = createServer(engine, { name: "scribe-dictation", version: "0.1.0" });
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "test-client", version: "0.0.0" });
  await Promise.all([server.connect(serverTransport), client.connect(clientTransport)]);
  return {
    client,
    close: async () => {
      await client.close();
      await server.close();
    },
  };
}

test("MCP: lists both tools and the dictation resource", async () => {
  const engine = new StateEngine({ controlPath: "/no/such.json", ...FAST });
  const { client, close } = await connectClient(engine);
  try {
    const tools = await client.listTools();
    const names = tools.tools.map((t) => t.name).sort();
    assert.deepEqual(names, ["get_dictation_state", "is_dictating"]);

    const resources = await client.listResources();
    assert.ok(resources.resources.some((r) => r.uri === RESOURCE_URI));
  } finally {
    await close();
    await engine.stop();
  }
});

test("MCP: get_dictation_state returns a live snapshot view when online", async () => {
  const mock = new MockScribe({ initial: { status: "Recording", dictating: true, busy: true } });
  await mock.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock.baseUrl, mock.token));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  const { client, close } = await connectClient(engine);
  try {
    const res = await client.callTool({ name: "get_dictation_state", arguments: {} });
    const structured = (res as { structuredContent?: Record<string, unknown> }).structuredContent;
    assert.ok(structured, "tool should return structuredContent");
    assert.equal(structured.online, true);
    assert.equal(structured.dictating, true);
    assert.equal(structured.busy, true);
    assert.equal(structured.status, "Recording");
  } finally {
    await close();
    await engine.stop();
    await mock.close();
    await cleanup();
  }
});

test("MCP: is_dictating returns cheap booleans", async () => {
  const mock = new MockScribe({ initial: { status: "Idle" } });
  await mock.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock.baseUrl, mock.token));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  const { client, close } = await connectClient(engine);
  try {
    const res = await client.callTool({ name: "is_dictating", arguments: {} });
    const s = (res as { structuredContent?: Record<string, unknown> }).structuredContent!;
    assert.equal(s.online, true);
    assert.equal(s.dictating, false);
    assert.equal(s.busy, false);
  } finally {
    await close();
    await engine.stop();
    await mock.close();
    await cleanup();
  }
});

test("MCP: offline -> tool reports not-dictating, not an error", async () => {
  const engine = new StateEngine({ controlPath: "/no/such/control.json", ...FAST });
  const { client, close } = await connectClient(engine);
  try {
    const res = await client.callTool({ name: "get_dictation_state", arguments: {} });
    const s = (res as { structuredContent?: Record<string, unknown> }).structuredContent!;
    assert.equal(s.online, false);
    assert.equal(s.dictating, false);
    assert.equal(s.busy, false);
    assert.equal(s.reason, "control-file-missing");
  } finally {
    await close();
    await engine.stop();
  }
});

test("MCP: reading the resource returns the current state JSON", async () => {
  const mock = new MockScribe({ initial: { status: "Recording", dictating: true, busy: true } });
  await mock.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock.baseUrl, mock.token));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  engine.start();
  const { client, close } = await connectClient(engine);
  try {
    await waitFor(() => engine.getCurrent().online === true);
    const res = await client.readResource({ uri: RESOURCE_URI });
    const first = res.contents[0];
    assert.equal(first.uri, RESOURCE_URI);
    assert.equal(first.mimeType, "application/json");
    const parsed = JSON.parse(first.text as string);
    assert.equal(parsed.dictating, true);
  } finally {
    await close();
    await engine.stop();
    await mock.close();
    await cleanup();
  }
});

test("MCP: subscription delivers resource-updated on start/stop", async () => {
  const mock = new MockScribe({ initial: { status: "Idle" } });
  await mock.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock.baseUrl, mock.token));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  engine.start();
  const { client, close } = await connectClient(engine);

  let updates = 0;
  client.setNotificationHandler(ResourceUpdatedNotificationSchema, (n) => {
    if (n.params.uri === RESOURCE_URI) updates += 1;
  });

  try {
    await waitFor(() => engine.getCurrent().online === true);
    await client.subscribeResource({ uri: RESOURCE_URI });

    mock.startDictation();
    await waitFor(() => updates >= 1, { timeoutMs: 2000 });
    const afterStart = updates;

    mock.stopDictation();
    await waitFor(() => updates > afterStart, { timeoutMs: 2000 });
    assert.ok(updates >= 2, "expected at least two resource-updated notifications");
  } finally {
    await close();
    await engine.stop();
    await mock.close();
    await cleanup();
  }
});
