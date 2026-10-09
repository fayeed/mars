import type { AgentAccount } from "@mars/shared-types";
import { describe, expect, it } from "vitest";

import { billingPhrase, hasOverride } from "./billing";

const account = (overrides: Partial<AgentAccount> = {}): AgentAccount => ({
  kind: "claude",
  billing: { type: "plan", plan: "max" },
  signIn: { type: "plan", plan: "max" },
  overrides: [],
  useSignIn: false,
  signInCommand: "claude auth login",
  ...overrides,
});

describe("billingPhrase", () => {
  it("names the plan by provider and tier", () => {
    expect(billingPhrase("claude", { type: "plan", plan: "team" })).toBe("your Claude Team plan");
    expect(billingPhrase("claude", { type: "plan", plan: null })).toBe("your Claude subscription");
    expect(billingPhrase("codex", { type: "plan", plan: null })).toBe("your ChatGPT plan");
  });

  it("names a key by where it came from, never by what it is", () => {
    expect(billingPhrase("claude", { type: "apiKey", source: "ANTHROPIC_API_KEY" })).toBe(
      "the API key in ANTHROPIC_API_KEY",
    );
    expect(billingPhrase("claude", { type: "apiKey", source: "apiKeyHelper" })).toBe(
      "an API key (apiKeyHelper)",
    );
  });

  it("names a cloud provider the way its owner does", () => {
    expect(billingPhrase("claude", { type: "cloudProvider", provider: "bedrock" })).toBe(
      "Amazon Bedrock",
    );
  });
});

describe("hasOverride", () => {
  it("is only a choice when there is a sign-in to fall back to", () => {
    expect(hasOverride(account())).toBe(false);
    expect(
      hasOverride(
        account({
          billing: { type: "apiKey", source: "ANTHROPIC_API_KEY" },
          overrides: ["ANTHROPIC_API_KEY"],
        }),
      ),
    ).toBe(true);
    expect(
      hasOverride(
        account({
          billing: { type: "apiKey", source: "ANTHROPIC_API_KEY" },
          signIn: { type: "signedOut" },
          overrides: ["ANTHROPIC_API_KEY"],
        }),
      ),
    ).toBe(false);
  });
});
