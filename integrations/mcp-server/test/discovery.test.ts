import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DISCOVERY_SCHEMA_MAJOR,
  controlFilePath,
  endpointUrl,
  isProcessAlive,
  readDiscovery,
} from "../src/discovery.js";
import { offlineState, onlineState, parseSnapshot } from "../src/types.js";
import { controlFor, writeControlFile } from "./helpers.js";

test("controlFilePath honors dev flavor and explicit override", () => {
  const home = "/home/u";
  assert.equal(controlFilePath({ home }), "/home/u/.scribe/control.json");
  assert.equal(controlFilePath({ home, dev: true }), "/home/u/.scribe/control.dev.json");
  assert.equal(controlFilePath({ path: "/tmp/x.json", home }), "/tmp/x.json");
});

test("isProcessAlive: current process is alive, absurd pid is dead", () => {
  assert.equal(isProcessAlive(process.pid), true);
  assert.equal(isProcessAlive(2_147_483_646), false);
  assert.equal(isProcessAlive(-1), false);
  assert.equal(isProcessAlive(0), false);
});

test("readDiscovery: missing file -> control-file-missing", async () => {
  const r = await readDiscovery("/nonexistent/dir/control.json");
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "control-file-missing");
});

test("readDiscovery: invalid JSON -> control-file-invalid", async () => {
  const { path, cleanup } = await writeControlFile({});
  const { writeFile } = await import("node:fs/promises");
  await writeFile(path, "{ not json", "utf8");
  try {
    const r = await readDiscovery(path);
    assert.equal(r.ok, false);
    assert.equal(r.ok === false && r.reason, "control-file-invalid");
  } finally {
    await cleanup();
  }
});

test("readDiscovery: missing required fields -> control-file-invalid", async () => {
  const { path, cleanup } = await writeControlFile({ schemaVersion: 1, app: "scribe" });
  try {
    const r = await readDiscovery(path);
    assert.equal(r.ok, false);
    assert.equal(r.ok === false && r.reason, "control-file-invalid");
  } finally {
    await cleanup();
  }
});

test("readDiscovery: schema too new -> schema-too-new", async () => {
  const doc = controlFor("http://127.0.0.1:9", "tok", { schemaVersion: DISCOVERY_SCHEMA_MAJOR + 1 });
  const { path, cleanup } = await writeControlFile(doc);
  try {
    const r = await readDiscovery(path);
    assert.equal(r.ok, false);
    assert.equal(r.ok === false && r.reason, "schema-too-new");
  } finally {
    await cleanup();
  }
});

test("readDiscovery: dead pid -> pid-dead", async () => {
  const doc = controlFor("http://127.0.0.1:9", "tok", { pid: 2_147_483_646 });
  const { path, cleanup } = await writeControlFile(doc);
  try {
    const r = await readDiscovery(path);
    assert.equal(r.ok, false);
    assert.equal(r.ok === false && r.reason, "pid-dead");
  } finally {
    await cleanup();
  }
});

test("readDiscovery: valid file -> ok with parsed target and endpoint URLs", async () => {
  const doc = controlFor("http://127.0.0.1:52431/", "tok-123");
  const { path, cleanup } = await writeControlFile(doc);
  try {
    const r = await readDiscovery(path);
    assert.equal(r.ok, true);
    if (!r.ok) return;
    assert.equal(r.doc.readToken, "tok-123");
    assert.equal(endpointUrl(r.doc, "status"), "http://127.0.0.1:52431/v1/status");
    assert.equal(endpointUrl(r.doc, "events"), "http://127.0.0.1:52431/v1/events");
  } finally {
    await cleanup();
  }
});

test("parseSnapshot: accepts a valid snapshot, ignores unknown fields", () => {
  const snap = parseSnapshot({
    schemaVersion: 1,
    app: "scribe",
    status: "Recording",
    dictating: true,
    busy: true,
    somethingNew: "ignored",
  });
  assert.ok(snap);
  assert.equal(snap.dictating, true);
});

test("parseSnapshot: rejects too-new major and malformed payloads", () => {
  assert.equal(parseSnapshot({ schemaVersion: 2, status: "X", dictating: true, busy: true }), null);
  assert.equal(parseSnapshot({ schemaVersion: 1, status: "X", dictating: "yes", busy: true }), null);
  assert.equal(parseSnapshot(null), null);
  assert.equal(parseSnapshot("nope"), null);
});

test("onlineState: enforces dictating -> busy invariant", () => {
  const s = onlineState({ schemaVersion: 1, app: "scribe", status: "Recording", dictating: true, busy: false });
  assert.equal(s.online, true);
  assert.equal(s.dictating, true);
  assert.equal(s.busy, true, "busy must be forced true when dictating is true");
});

test("offlineState: always not-dictating and not-busy", () => {
  const s = offlineState("connection-refused");
  assert.equal(s.online, false);
  assert.equal(s.dictating, false);
  assert.equal(s.busy, false);
  assert.equal(s.status, "Offline");
  assert.equal(s.reason, "connection-refused");
});
