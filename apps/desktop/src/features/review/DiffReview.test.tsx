import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { DiffReview } from "./DiffReview";
import { parseUnifiedDiff, untrackedFiles } from "./unified-diff";

const DIFF = `diff --git a/crates/ingest/src/lib.rs b/crates/ingest/src/lib.rs
--- a/crates/ingest/src/lib.rs
+++ b/crates/ingest/src/lib.rs
@@ -80,3 +80,4 @@ impl Ingest {
     pub fn accept(&self) {
-        self.inner.accept()
+        self.limiter.check()?;
+        self.inner.accept()
     }
diff --git a/logo.png b/logo.png
index 1a2b3c4..5d6e7f8 100644
Binary files a/logo.png and b/logo.png differ
`;

const files = [...parseUnifiedDiff(DIFF), ...untrackedFiles(["notes.md"])];

function renderReview(props: Partial<Parameters<typeof DiffReview>[0]> = {}) {
  return render(
    <DiffReview
      // Highlighting is an enhancement; the tests are about the diff itself.
      colorize={() => Promise.resolve([])}
      files={files}
      onComment={vi.fn()}
      state="idle"
      {...props}
    />,
  );
}

describe("DiffReview file list", () => {
  it("lists every changed file with its counts", () => {
    renderReview();

    const list = screen.getByRole("list", { name: "Changed files" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(3);
    expect(within(list).getByText("+2")).toBeVisible();
    expect(within(list).getByText("−1")).toBeVisible();
  });

  it("marks a binary file instead of claiming counts for it", () => {
    renderReview();

    expect(screen.getByText("binary")).toBeVisible();
  });

  it("includes files Git has never seen", () => {
    renderReview();

    expect(screen.getByText("notes.md")).toBeVisible();
  });
});

describe("DiffReview diff body", () => {
  it("numbers both sides and marks added and removed lines", () => {
    renderReview();

    expect(screen.getByText("@@ -80,3 +80,4 @@ impl Ingest {")).toBeVisible();
    expect(screen.getByText("self.limiter.check()?;")).toBeVisible();
    // The unchanged call appears on both sides of the diff: removed at old 81
    // and added back at new 82. Both rows are shown, which is the point.
    expect(screen.getAllByText("self.inner.accept()")).toHaveLength(2);
  });

  it("says which side of the change a line number belongs to", () => {
    renderReview();

    // Two rows carry the number 81; only one of them is the file as it is now.
    expect(
      screen.getByRole("button", { name: "Comment on line 81 before the change" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Comment on line 81" })).toBeInTheDocument();
  });

  it("says a binary file has nothing to show, rather than showing nothing", async () => {
    const user = userEvent.setup();
    renderReview();

    await user.click(screen.getByRole("button", { name: /logo\.png/ }));
    expect(screen.getByText("Binary file")).toBeVisible();
    expect(screen.getByText(/not how/)).toBeVisible();
  });

  it("explains why an untracked file has no diff, and offers to open it", async () => {
    const user = userEvent.setup();
    const onOpenFile = vi.fn();
    renderReview({ onOpenFile });

    await user.click(screen.getByRole("button", { name: /notes\.md/ }));
    expect(screen.getByText("New file, not yet staged")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Open notes.md" }));
    expect(onOpenFile).toHaveBeenCalledWith("notes.md", null);
  });
});

describe("DiffReview commenting", () => {
  it("posts a line comment into the conversation", async () => {
    const user = userEvent.setup();
    const onComment = vi.fn();
    renderReview({ onComment });

    await user.click(screen.getByRole("button", { name: "Comment on line 81" }));
    const input = screen.getByRole("textbox", { name: "Comment on line 81" });
    await user.type(input, "This can fail before the limiter runs.");
    await user.click(screen.getByRole("button", { name: "Post" }));

    expect(onComment).toHaveBeenCalledWith(
      "crates/ingest/src/lib.rs",
      81,
      "This can fail before the limiter runs.",
      "new",
    );
  });

  it("anchors a comment on a removed line to the file as it was", async () => {
    const user = userEvent.setup();
    const onComment = vi.fn();
    renderReview({ onComment });

    await user.click(
      screen.getByRole("button", { name: "Comment on line 81 before the change" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "Comment on line 81 before the change" }),
      "Why was this dropped?",
    );
    await user.click(screen.getByRole("button", { name: "Post" }));

    expect(onComment).toHaveBeenCalledWith(
      "crates/ingest/src/lib.rs",
      81,
      "Why was this dropped?",
      "old",
    );
  });

  it("says where the comment is going before it is written", async () => {
    const user = userEvent.setup();
    renderReview();

    await user.click(screen.getByRole("button", { name: "Comment on line 81" }));
    expect(
      screen.getByPlaceholderText("This posts into the conversation."),
    ).toBeVisible();
  });

  it("will not post an empty comment", async () => {
    const user = userEvent.setup();
    const onComment = vi.fn();
    renderReview({ onComment });

    await user.click(screen.getByRole("button", { name: "Comment on line 81" }));
    expect(screen.getByRole("button", { name: "Post" })).toBeDisabled();
    expect(onComment).not.toHaveBeenCalled();
  });

  it("abandons the comment on Escape", async () => {
    const user = userEvent.setup();
    renderReview();

    await user.click(screen.getByRole("button", { name: "Comment on line 81" }));
    await user.keyboard("{Escape}");
    expect(
      screen.queryByRole("textbox", { name: "Comment on line 81" }),
    ).not.toBeInTheDocument();
  });
});

describe("DiffReview states", () => {
  it("says there is nothing to review rather than showing an empty pane", () => {
    renderReview({ files: [] });

    expect(screen.getByText("No changes to review")).toBeVisible();
  });

  it("reports a failure without claiming the worktree changed", () => {
    renderReview({ state: "failed", error: "not a git repository" });

    expect(screen.getByText("The diff could not be read")).toBeVisible();
    expect(screen.getByText("not a git repository")).toBeVisible();
  });
});
