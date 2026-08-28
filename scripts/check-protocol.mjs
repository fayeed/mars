import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import { PROTOCOL_VERSION } from "../packages/protocol/src/generated.ts";

const schemaUrl = new URL("../packages/protocol/schema/core.schema.json", import.meta.url);
const schema = JSON.parse(await readFile(schemaUrl, "utf8"));
const generatedVersion = `${PROTOCOL_VERSION.major}.${PROTOCOL_VERSION.minor}.${PROTOCOL_VERSION.patch}`;

assert.equal(
  schema["x-mars-protocol-version"],
  generatedVersion,
  "Rust/TypeScript protocol artifacts must target the schema version",
);

process.stdout.write(`Mars protocol ${generatedVersion} artifacts agree.\n`);
