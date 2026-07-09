/**
 * MCP server wiring - tools + a subscribable resource over the StateEngine.
 *
 * Uses the low-level `Server` so we can declare `resources.subscribe` and drive
 * `notifications/resources/updated` directly from SSE-driven state changes.
 *
 * Surface (generic; no assumptions about any particular MCP host):
 *   - tool  `get_dictation_state` - fresh point-in-time snapshot view.
 *   - tool  `is_dictating`        - cheap {dictating, busy, online} for gating.
 *   - resource `scribe://dictation/state` - live state, subscribable.
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import {
  CallToolRequestSchema,
  ErrorCode,
  ListResourcesRequestSchema,
  ListToolsRequestSchema,
  McpError,
  ReadResourceRequestSchema,
  SubscribeRequestSchema,
  UnsubscribeRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import type { StateEngine } from "./engine.js";
import type { DictationState } from "./types.js";

export const RESOURCE_URI = "scribe://dictation/state";

const STATE_OUTPUT_SCHEMA = {
  type: "object",
  properties: {
    online: { type: "boolean", description: "Whether a live Scribe was reachable." },
    dictating: { type: "boolean", description: "Mic actively capturing. False when offline." },
    busy: { type: "boolean", description: "Inside a dictation cycle. False when offline." },
    status: { type: "string", description: 'Scribe status, or "Offline" when unreachable.' },
    since: { type: ["string", "null"] },
    updatedAt: { type: "string" },
    pid: { type: ["integer", "null"] },
    app: { type: ["string", "null"] },
    appVersion: { type: ["string", "null"] },
    schemaVersion: { type: ["integer", "null"] },
    reason: { type: "string", description: "Why offline (present only when offline)." },
    observedAt: { type: "string", description: "When this server produced the view." },
  },
  required: ["online", "dictating", "busy", "status", "observedAt"],
  additionalProperties: true,
} as const;

const IS_DICTATING_OUTPUT_SCHEMA = {
  type: "object",
  properties: {
    dictating: { type: "boolean" },
    busy: { type: "boolean" },
    online: { type: "boolean" },
    status: { type: "string" },
  },
  required: ["dictating", "busy", "online"],
  additionalProperties: true,
} as const;

function pretty(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export interface ServerMeta {
  name: string;
  version: string;
}

/** Build the MCP `Server`, wiring tools, the resource, and subscriptions. */
export function createServer(engine: StateEngine, meta: ServerMeta): Server {
  const server = new Server(
    { name: meta.name, version: meta.version },
    { capabilities: { tools: {}, resources: { subscribe: true } } },
  );

  // Track subscribed resource URIs so we only notify when someone is listening.
  const subscribers = new Set<string>();

  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: [
      {
        name: "get_dictation_state",
        title: "Get Scribe dictation state",
        description:
          "Point-in-time snapshot of Scribe's dictation state (status, dictating, " +
          "busy, since, updatedAt, pid). Returns online=false with dictating=false " +
          "and busy=false whenever Scribe is offline, dead, or unreachable.",
        inputSchema: { type: "object", properties: {}, additionalProperties: false },
        outputSchema: STATE_OUTPUT_SCHEMA,
      },
      {
        name: "is_dictating",
        title: "Is Scribe dictating?",
        description:
          "Cheap boolean gate. Returns {dictating, busy, online, status}. Both " +
          "booleans are false when Scribe is offline. Gate held output on `busy`; " +
          "gate a mic indicator on `dictating`.",
        inputSchema: { type: "object", properties: {}, additionalProperties: false },
        outputSchema: IS_DICTATING_OUTPUT_SCHEMA,
      },
    ],
  }));

  server.setRequestHandler(CallToolRequestSchema, async (req) => {
    const state = await engine.refreshNow();
    switch (req.params.name) {
      case "get_dictation_state":
        return {
          content: [{ type: "text", text: pretty(state) }],
          structuredContent: state as unknown as Record<string, unknown>,
        };
      case "is_dictating": {
        const out = {
          dictating: state.dictating,
          busy: state.busy,
          online: state.online,
          status: state.status,
        };
        return {
          content: [{ type: "text", text: pretty(out) }],
          structuredContent: out,
        };
      }
      default:
        throw new McpError(ErrorCode.MethodNotFound, `Unknown tool: ${req.params.name}`);
    }
  });

  server.setRequestHandler(ListResourcesRequestSchema, async () => ({
    resources: [
      {
        uri: RESOURCE_URI,
        name: "Scribe dictation state",
        title: "Scribe dictation state (live)",
        description:
          "Live dictation state. Subscribe to be notified the instant dictation " +
          "starts or stops. Reverts to not-dictating whenever Scribe goes offline.",
        mimeType: "application/json",
      },
    ],
  }));

  server.setRequestHandler(ReadResourceRequestSchema, async (req) => {
    if (req.params.uri !== RESOURCE_URI) {
      throw new McpError(ErrorCode.InvalidParams, `Unknown resource: ${req.params.uri}`);
    }
    // Prefer the SSE-driven cached value. If it is offline (including the brief
    // pre-connect window), re-check authoritatively before declaring offline.
    const cached = engine.getCurrent();
    const state: DictationState = cached.online ? cached : await engine.refreshNow();
    return {
      contents: [
        {
          uri: RESOURCE_URI,
          mimeType: "application/json",
          text: pretty(state),
        },
      ],
    };
  });

  server.setRequestHandler(SubscribeRequestSchema, async (req) => {
    subscribers.add(req.params.uri);
    return {};
  });

  server.setRequestHandler(UnsubscribeRequestSchema, async (req) => {
    subscribers.delete(req.params.uri);
    return {};
  });

  // Fan out engine state changes as resource-updated notifications.
  engine.onChange(() => {
    if (subscribers.has(RESOURCE_URI)) {
      server.sendResourceUpdated({ uri: RESOURCE_URI }).catch(() => {});
    }
  });

  return server;
}
