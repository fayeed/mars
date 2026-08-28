import assert from "node:assert/strict";
import test from "node:test";

import {
  PROTOCOL_VERSION,
  decodeCommandRequest,
  decodeCommandResponse,
  decodeCoreEvent,
  type CommandRequest,
  type CommandResponse,
  type CoreEvent,
} from "./generated.ts";

test("command request round-trips with its discriminated payload", () => {
  const request = {
    protocolVersion: PROTOCOL_VERSION,
    correlationId: "req-0198",
    command: { type: "coreInfo" },
  } satisfies CommandRequest;

  assert.deepEqual(decodeCommandRequest(JSON.stringify(request)), request);
});

test("command response round-trips success and structured errors", () => {
  const responses = [
    {
      status: "success",
      protocolVersion: PROTOCOL_VERSION,
      correlationId: "req-0198",
      result: { type: "coreInfo", protocolVersion: PROTOCOL_VERSION },
    },
    {
      status: "failure",
      protocolVersion: PROTOCOL_VERSION,
      correlationId: "req-0199",
      error: {
        code: "core.unavailable",
        message: "Core is unavailable",
        retryable: true,
        details: { component: "storage" },
      },
    },
  ] satisfies CommandResponse[];

  for (const response of responses) {
    assert.deepEqual(decodeCommandResponse(JSON.stringify(response)), response);
  }
});

test("event decoding preserves fields introduced by newer cores", () => {
  const event = {
    protocolVersion: PROTOCOL_VERSION,
    sequence: 7,
    event: {
      type: "indexProgress",
      payload: { filesSeen: 42 },
      futureField: { supportedLater: true },
    },
  } satisfies CoreEvent;

  assert.deepEqual(decodeCoreEvent(JSON.stringify(event)), event);
});

test("decoders reject malformed envelopes", () => {
  assert.throws(
    () => decodeCommandRequest('{"correlationId":"missing-version"}'),
    /Invalid CommandRequest/,
  );
  assert.throws(
    () =>
      decodeCommandRequest(
        JSON.stringify({
          protocolVersion: PROTOCOL_VERSION,
          correlationId: "req-0198",
          command: { type: "coreInfo", futureField: true },
        }),
      ),
    /Invalid CommandRequest/,
  );
});
