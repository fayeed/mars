import { add } from "../src/math";

describe("add", () => {
  it("adds values", () => {
    expect(add(1, 2)).toBe(3);
  });
});
