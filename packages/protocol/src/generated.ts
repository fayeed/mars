export interface ProtocolVersion {
  major: number;
  minor: number;
  patch: number;
}

export const PROTOCOL_VERSION = {
  major: 0,
  minor: 1,
  patch: 0,
} as const satisfies ProtocolVersion;

export type Command = { type: "coreInfo" };

export interface CommandRequest {
  protocolVersion: ProtocolVersion;
  correlationId: string;
  command: Command;
}

export type ResponsePayload = {
  type: "coreInfo";
  protocolVersion: ProtocolVersion;
};

export interface ProtocolError {
  code: string;
  message: string;
  retryable: boolean;
  details?: unknown;
}

export type CommandResponse =
  | {
      status: "success";
      protocolVersion: ProtocolVersion;
      correlationId: string;
      result: ResponsePayload;
    }
  | {
      status: "failure";
      protocolVersion: ProtocolVersion;
      correlationId: string;
      error: ProtocolError;
    };

export interface EventPayload {
  type: string;
  payload: unknown;
  [extension: string]: unknown;
}

export interface CoreEvent {
  protocolVersion: ProtocolVersion;
  sequence: number;
  event: EventPayload;
}

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(value: JsonObject, allowed: readonly string[]): boolean {
  const allowedKeys = new Set(allowed);
  return Object.keys(value).every((key) => allowedKeys.has(key));
}

function isProtocolVersion(value: unknown): value is ProtocolVersion {
  return (
    isObject(value) &&
    hasOnlyKeys(value, ["major", "minor", "patch"]) &&
    [value.major, value.minor, value.patch].every(
      (part) => Number.isInteger(part) && Number(part) >= 0 && Number(part) <= 65_535,
    )
  );
}

function isCorrelationId(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function parseJson(serialized: string, envelope: string): unknown {
  try {
    return JSON.parse(serialized) as unknown;
  } catch {
    throw new TypeError(`Invalid ${envelope}: expected JSON`);
  }
}

function isCommand(value: unknown): value is Command {
  return isObject(value) && value.type === "coreInfo" && Object.keys(value).length === 1;
}

function isResponsePayload(value: unknown): value is ResponsePayload {
  return (
    isObject(value) &&
    hasOnlyKeys(value, ["type", "protocolVersion"]) &&
    value.type === "coreInfo" &&
    isProtocolVersion(value.protocolVersion)
  );
}

function isProtocolError(value: unknown): value is ProtocolError {
  return (
    isObject(value) &&
    hasOnlyKeys(value, ["code", "message", "retryable", "details"]) &&
    typeof value.code === "string" &&
    value.code.length > 0 &&
    typeof value.message === "string" &&
    value.message.length > 0 &&
    typeof value.retryable === "boolean"
  );
}

export function decodeCommandRequest(serialized: string): CommandRequest {
  const value = parseJson(serialized, "CommandRequest");
  if (
    !isObject(value) ||
    !hasOnlyKeys(value, ["protocolVersion", "correlationId", "command"]) ||
    !isProtocolVersion(value.protocolVersion) ||
    !isCorrelationId(value.correlationId) ||
    !isCommand(value.command)
  ) {
    throw new TypeError("Invalid CommandRequest: envelope does not match protocol 0.1.0");
  }
  return value as unknown as CommandRequest;
}

export function decodeCommandResponse(serialized: string): CommandResponse {
  const value = parseJson(serialized, "CommandResponse");
  const validEnvelope =
    isObject(value) &&
    isProtocolVersion(value.protocolVersion) &&
    isCorrelationId(value.correlationId);
  const validPayload =
    validEnvelope &&
    ((value.status === "success" &&
      hasOnlyKeys(value, ["status", "protocolVersion", "correlationId", "result"]) &&
      isResponsePayload(value.result)) ||
      (value.status === "failure" &&
        hasOnlyKeys(value, ["status", "protocolVersion", "correlationId", "error"]) &&
        isProtocolError(value.error)));

  if (!validPayload) {
    throw new TypeError("Invalid CommandResponse: envelope does not match protocol 0.1.0");
  }
  return value as unknown as CommandResponse;
}

export function decodeCoreEvent(serialized: string): CoreEvent {
  const value = parseJson(serialized, "CoreEvent");
  if (
    !isObject(value) ||
    !hasOnlyKeys(value, ["protocolVersion", "sequence", "event"]) ||
    !isProtocolVersion(value.protocolVersion) ||
    !Number.isSafeInteger(value.sequence) ||
    Number(value.sequence) < 0 ||
    !isObject(value.event) ||
    typeof value.event.type !== "string" ||
    value.event.type.length === 0 ||
    !("payload" in value.event)
  ) {
    throw new TypeError("Invalid CoreEvent: envelope does not match protocol 0.1.0");
  }
  return value as unknown as CoreEvent;
}
