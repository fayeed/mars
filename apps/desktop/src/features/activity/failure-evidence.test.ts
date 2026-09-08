import { describe, expect, it } from "vitest";

import { comparedValues, failureEvidence, sourceReferences } from "./failure-evidence";

const RUST = `
running 1 test
test rate_limit::rejects_over_budget ... FAILED

failures:

---- rate_limit::rejects_over_budget stdout ----
thread 'rate_limit::rejects_over_budget' panicked at crates/ingest/src/rate_limit.rs:82:9:
assertion \`left == right\` failed
  left: 429
 right: 200
note: run with \`RUST_BACKTRACE=1\` to display a backtrace
`;

const VITEST = `
 FAIL  src/shell/composer-input.test.ts > readTrigger
AssertionError: expected 'skill' to be 'mention'

- Expected: "mention"
+ Received: "skill"

 ❯ src/shell/composer-input.test.ts:41:24
`;

describe("sourceReferences", () => {
  it("finds a path with a line and a column", () => {
    expect(sourceReferences(RUST)).toContainEqual({
      path: "crates/ingest/src/rate_limit.rs",
      line: 82,
      column: 9,
      raw: "crates/ingest/src/rate_limit.rs:82:9",
    });
  });

  it("normalises Windows separators to the ones the rest of Mars speaks", () => {
    const [reference] = sourceReferences("at crates\\ingest\\src\\lib.rs:12");
    expect(reference.path).toBe("crates/ingest/src/lib.rs");
    expect(reference.line).toBe(12);
    expect(reference.column).toBeNull();
  });

  it("does not turn a timestamp or a duration into a file", () => {
    expect(sourceReferences("finished at 12:30:01 after 4:20")).toHaveLength(0);
  });

  it("reports each location once, in the order it was mentioned", () => {
    const found = sourceReferences("a.rs:1\nb.rs:2\na.rs:1\n");
    expect(found.map((reference) => reference.path)).toEqual(["a.rs", "b.rs"]);
  });
});

describe("comparedValues", () => {
  it("reads a Rust assert_eq with the runner's own labels", () => {
    expect(comparedValues(RUST)).toEqual({
      expected: "429",
      actual: "200",
      expectedLabel: "left",
      actualLabel: "right",
    });
  });

  it("reads an expected/received pair", () => {
    expect(comparedValues(VITEST)).toMatchObject({
      expected: '"mention"',
      actual: '"skill"',
      expectedLabel: "expected",
      actualLabel: "actual",
    });
  });

  it("gives up rather than guessing when only one side is named", () => {
    expect(comparedValues("expected: 3\nbut something else happened")).toBeNull();
  });
});

describe("failureEvidence", () => {
  it("lifts the assertion, the comparison and the location out of Rust output", () => {
    const evidence = failureEvidence(RUST);

    expect(evidence.assertion).toContain("assertion `left == right` failed");
    expect(evidence.compared?.actual).toBe("200");
    expect(evidence.references[0].path).toBe("crates/ingest/src/rate_limit.rs");
  });

  it("recognises an AssertionError", () => {
    expect(failureEvidence(VITEST).assertion).toContain("AssertionError");
  });

  it("returns nothing rather than inventing structure for output it cannot read", () => {
    const evidence = failureEvidence("Segmentation fault (core dumped)");

    expect(evidence.assertion).toBeNull();
    expect(evidence.compared).toBeNull();
    expect(evidence.references).toHaveLength(0);
  });
});
