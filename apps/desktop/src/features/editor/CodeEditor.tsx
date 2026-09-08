import Editor, { type OnMount } from "@monaco-editor/react";
import type { CommandFailure, FileDocument } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { AlertCircleIcon, RefreshCwIcon, SaveIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

import "./monaco";

interface CodeEditorProps {
  repositoryId: string;
  /**
   * The task this edit belongs to, when there is one.
   *
   * A save made with a task in view is recorded in that task's activity log, so
   * an edit made by hand shows up in the transcript the same way an agent's
   * does. Without a task there is no record to write to, and the save is just
   * a save.
   */
  taskId?: string | null;
  document: FileDocument;
  value: string;
  onChange: (value: string) => void;
  onReload: () => void;
  onSaved: (document: FileDocument) => void;
  revealLine?: number | null;
}

function languageFor(path: string) {
  const extension = path.split(".").pop()?.toLowerCase();
  return (
    {
      css: "css",
      html: "html",
      js: "javascript",
      json: "json",
      jsx: "javascript",
      md: "markdown",
      mjs: "javascript",
      ts: "typescript",
      tsx: "typescript",
      yaml: "yaml",
      yml: "yaml",
    }[extension ?? ""] ?? "plaintext"
  );
}

function errorDetails(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    const failure = error as CommandFailure;
    return { code: failure.code, message: String(failure.message) };
  }
  return {
    code: "file.unknown",
    message: error instanceof Error ? error.message : String(error),
  };
}

export function CodeEditor({
  repositoryId,
  document,
  value,
  onChange,
  onReload,
  onSaved,
  revealLine,
  taskId,
}: CodeEditorProps) {
  const editorRef = useRef<Parameters<OnMount>[0] | null>(null);
  const [error, setError] = useState<{ code: string; message: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const dirty = value !== document.content;

  const save = useCallback(async () => {
    if (!dirty || saving) return;
    setSaving(true);
    setError(null);
    try {
      const saved = await invoke<FileDocument>("write_file", {
        repositoryId,
        relativePath: document.relativePath,
        content: value,
        expectedVersion: document.version,
        taskId: taskId ?? null,
      });
      onSaved(saved);
    } catch (caught) {
      setError(errorDetails(caught));
    } finally {
      setSaving(false);
    }
  }, [
    dirty,
    document.relativePath,
    document.version,
    onSaved,
    repositoryId,
    saving,
    taskId,
    value,
  ]);

  useEffect(() => {
    function handleSave(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void save();
      }
    }
    window.addEventListener("keydown", handleSave);
    return () => window.removeEventListener("keydown", handleSave);
  }, [save]);

  useEffect(() => {
    if (revealLine && editorRef.current) {
      editorRef.current.revealLineInCenter(revealLine);
      editorRef.current.setPosition({ column: 1, lineNumber: revealLine });
      editorRef.current.focus();
    }
  }, [revealLine]);

  const mountEditor: OnMount = (editor) => {
    editorRef.current = editor;
    if (revealLine) {
      editor.revealLineInCenter(revealLine);
      editor.setPosition({ column: 1, lineNumber: revealLine });
    }
  };

  return (
    <section className="flex h-full min-h-0 flex-col" aria-label={`Editor: ${document.relativePath}`}>
      <header className="flex h-11 shrink-0 items-center gap-3 border-b px-3">
        <p className="min-w-0 flex-1 truncate font-mono text-xs">{document.relativePath}</p>
        {dirty ? <Badge variant="outline">Unsaved</Badge> : <Badge variant="secondary">Saved</Badge>}
        <Button disabled={!dirty || saving} onClick={() => void save()} size="sm" type="button">
          <SaveIcon data-icon="inline-start" />
          {saving ? "Saving…" : "Save"}
        </Button>
      </header>

      {error ? (
        <Alert className="m-3 w-auto" variant="destructive">
          <AlertCircleIcon />
          <AlertTitle>{error.code === "file.write_conflict" ? "File changed on disk" : "Save failed"}</AlertTitle>
          <AlertDescription className="flex items-center justify-between gap-4">
            <span>{error.message}</span>
            {error.code === "file.write_conflict" ? (
              <Button onClick={onReload} size="sm" type="button" variant="outline">
                <RefreshCwIcon data-icon="inline-start" /> Reload from disk
              </Button>
            ) : null}
          </AlertDescription>
        </Alert>
      ) : null}

      <div className="min-h-0 flex-1">
        <Editor
          language={languageFor(document.relativePath)}
          onMount={mountEditor}
          onChange={(next) => onChange(next ?? "")}
          options={{
            automaticLayout: true,
            fontFamily: "Geist Mono, ui-monospace, monospace",
            fontSize: 13,
            minimap: { enabled: false },
            padding: { top: 16 },
            scrollBeyondLastLine: false,
          }}
          path={`file:///${document.relativePath}`}
          saveViewState
          theme="vs-dark"
          value={value}
        />
      </div>
    </section>
  );
}
