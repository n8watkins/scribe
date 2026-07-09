import assert from "node:assert/strict";
import { after, test } from "node:test";
import { StateEngine } from "../src/engine.js";
import { MockScribe, snapshot } from "./mock-server.js";
import { controlFor, waitFor, writeControlFile } from "./helpers.js";

// Fast backoff so reconnect tests don't dawdle.
const FAST = { minBackoffMs: 20, maxBackoffMs: 60, httpTimeoutMs: 1000, idleTimeoutMs: 500 };

test("refreshNow: online snapshot maps to a dictating state", async () => {
  const mock = new MockScribe({ initial: { status: "Recording", dictating: true, busy: true } });
  await mock.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock.baseUrl, mock.token));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  try {
    const state = await engine.refreshNow();
    assert.equal(state.online, true);
    assert.equal(state.dictating, true);
    assert.equal(state.busy, true);
    assert.equal(state.status, "Recording");
  } finally {
    await engine.stop();
    await mock.close();
    await cleanup();
  }
});

test("refreshNow: missing control file -> offline, not dictating", async () => {
  const engine = new StateEngine({ controlPath: "/no/such/control.json", ...FAST });
  const state = await engine.refreshNow();
  assert.equal(state.online, false);
  assert.equal(state.dictating, false);
  assert.equal(state.reason, "control-file-missing");
  await engine.stop();
});

test("refreshNow: control points at a closed port -> connection-refused", async () => {
  const { path, cleanup } = await writeControlFile(controlFor("http://127.0.0.1:1", "tok"));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  try {
    const state = await engine.refreshNow();
    assert.equal(state.online, false);
    assert.equal(state.dictating, false);
    assert.equal(state.reason, "connection-refused");
  } finally {
    await engine.stop();
    await cleanup();
  }
});

test("subscription: SSE start/stop events drive current state + notifications", async () => {
  const mock = new MockScribe(); // starts Idle
  await mock.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock.baseUrl, mock.token));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  const changes: boolean[] = [];
  engine.onChange((s) => changes.push(s.dictating));

  try {
    engine.start();
    // Initial replay: Idle, online, not dictating.
    await waitFor(() => engine.getCurrent().online === true);
    assert.equal(engine.getCurrent().dictating, false);

    mock.startDictation();
    await waitFor(() => engine.getCurrent().dictating === true);
    assert.equal(engine.getCurrent().busy, true);
    assert.equal(engine.getCurrent().status, "Recording");

    mock.stopDictation();
    await waitFor(() => engine.getCurrent().dictating === false);
    assert.equal(engine.getCurrent().online, true);

    assert.ok(changes.includes(true), "should have observed a dictating=true change");
    assert.ok(changes.includes(false), "should have observed a dictating=false change");
  } finally {
    await engine.stop();
    await mock.close();
    await cleanup();
  }
});

test("liveness: a mid-stream crash never leaves a stuck 'dictating'", async () => {
  const mock = new MockScribe({ initial: { status: "Recording", dictating: true, busy: true } });
  await mock.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock.baseUrl, mock.token));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  try {
    engine.start();
    // The replayed snapshot has us dictating.
    await waitFor(() => engine.getCurrent().dictating === true);

    // Scribe crashes: streams drop mid-recording.
    mock.dropStreams();

    // The belief must die with the socket - quickly, without waiting a TTL.
    await waitFor(() => engine.getCurrent().dictating === false, { timeoutMs: 1000 });
    assert.equal(engine.getCurrent().online, false);
    assert.equal(engine.getCurrent().busy, false);
  } finally {
    await engine.stop();
    await mock.close();
    await cleanup();
  }
});

test("reconnect: engine recovers and re-establishes truth after Scribe returns", async () => {
  const token = "same-token";
  const mock1 = new MockScribe({ token, initial: { status: "Idle" } });
  await mock1.listen();
  const { path, cleanup } = await writeControlFile(controlFor(mock1.baseUrl, token));
  const engine = new StateEngine({ controlPath: path, ...FAST });
  try {
    engine.start();
    await waitFor(() => engine.getCurrent().online === true);

    // Scribe goes away entirely.
    await mock1.close();
    await waitFor(() => engine.getCurrent().online === false, { timeoutMs: 1500 });

    // Scribe restarts on a new ephemeral port, mid-recording; control file updated.
    const mock2 = new MockScribe({ token, initial: { status: "Recording", dictating: true, busy: true } });
    await mock2.listen();
    const { writeFile } = await import("node:fs/promises");
    await writeFile(path, JSON.stringify(controlFor(mock2.baseUrl, token)), "utf8");

    // Reconnect + replay re-establishes truth: now dictating.
    await waitFor(() => engine.getCurrent().dictating === true, { timeoutMs: 2000 });
    assert.equal(engine.getCurrent().online, true);
    await mock2.close();
  } finally {
    await engine.stop();
    await cleanup();
  }
});

// Guard against a leaked timer/loop keeping the runner alive.
after(() => {});
