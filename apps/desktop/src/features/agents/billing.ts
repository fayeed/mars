import type { AdapterKind, AgentAccount, AgentBilling } from "@mars/shared-types";

const PROVIDERS: Record<string, string> = {
  bedrock: "Amazon Bedrock",
  vertex: "Google Cloud",
  foundry: "Microsoft Foundry",
};

function capitalised(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

/**
 * Who pays, as a phrase that completes "Runs bill …".
 *
 * Keys are named by where they come from and never by value, which Mars does
 * not have anyway: the CLI reports the source, and the environment check only
 * looks at whether a variable is set.
 */
export function billingPhrase(kind: AdapterKind, billing: AgentBilling): string {
  switch (billing.type) {
    case "plan":
      if (kind === "codex") return "your ChatGPT plan";
      return billing.plan ? `your Claude ${capitalised(billing.plan)} plan` : "your Claude subscription";
    case "apiKey":
      return /^[A-Z][A-Z0-9_]*$/.test(billing.source)
        ? `the API key in ${billing.source}`
        : `an API key (${billing.source})`;
    case "cloudProvider":
      return PROVIDERS[billing.provider] ?? billing.provider;
    case "signedOut":
      return "nothing yet: the CLI is not signed in";
    case "unknown":
      return `an account Mars could not read (${billing.detail})`;
  }
}

export function isSignedIn(billing: AgentBilling): boolean {
  return billing.type === "plan" || billing.type === "apiKey" || billing.type === "cloudProvider";
}

/**
 * Whether a variable in Mars's environment is the thing deciding billing, with
 * a sign-in underneath that the person could use instead.
 *
 * This is the case Claude Code asks about once in a terminal and never in a
 * `-p` run, which is how Mars starts it, so Mars has to ask on its behalf.
 */
export function hasOverride(account: AgentAccount): boolean {
  return account.overrides.length > 0 && isSignedIn(account.signIn);
}
