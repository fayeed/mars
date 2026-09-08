import type { CommandFailure, FileDocument, ProjectSnapshot } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { AlertCircleIcon, Code2Icon } from "lucide-react";
import { lazy, Suspense, useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BrainAnswer } from "@/features/brain/BrainAnswer";
import { MemoryApproval } from "@/features/brain/MemoryApproval";
import { BrainSearch } from "@/features/brain/BrainSearch";
import { FileExplorer } from "@/features/explorer/FileExplorer";
import { TaskWorkspace } from "@/features/tasks/TaskWorkspace";
import { TerminalPanel } from "@/features/terminal/TerminalPanel";

const CodeEditor = lazy(() =>
  import("@/features/editor/CodeEditor").then((module) => ({ default: module.CodeEditor })),
);

interface RepositoryWorkspaceProps {
  onOpenProject: (project: ProjectSnapshot) => void;
  project: ProjectSnapshot;
}

interface OpenFile {
  document: FileDocument;
  draft: string;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function RepositoryWorkspace({ onOpenProject, project }: RepositoryWorkspaceProps) {
  const [files, setFiles] = useState<OpenFile[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [revealLine, setRevealLine] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const activeFile = files.find((file) => file.document.relativePath === activePath) ?? null;

  async function openFile(relativePath: string, force = false, line: number | null = null) {
    setRevealLine(line);
    const existing = files.find((file) => file.document.relativePath === relativePath);
    if (existing && !force) {
      setActivePath(relativePath);
      return;
    }
    setError(null);
    try {
      const document = await invoke<FileDocument>("read_file", {
        repositoryId: project.repositoryId,
        relativePath,
      });
      setFiles((current) => [
        ...current.filter((file) => file.document.relativePath !== relativePath),
        { document, draft: document.content },
      ]);
      setActivePath(relativePath);
    } catch (caught) {
      setError(messageFor(caught));
    }
  }

  function updateActive(update: (file: OpenFile) => OpenFile) {
    if (!activePath) return;
    setFiles((current) =>
      current.map((file) => (file.document.relativePath === activePath ? update(file) : file)),
    );
  }

  return (
    <section className="col-span-2 min-h-0" aria-label={`${project.name} workspace`}>
      <ResizablePanelGroup orientation="horizontal">
        <ResizablePanel defaultSize="22%" minSize="15%" maxSize="38%">
          <Tabs className="h-full gap-0" defaultValue="tasks">
            <TabsList className="h-9 w-full rounded-none border-b bg-card" variant="line">
              <TabsTrigger value="tasks">Tasks</TabsTrigger>
              <TabsTrigger value="explorer">Explorer</TabsTrigger>
              <TabsTrigger value="brain">Brain</TabsTrigger>
              <TabsTrigger value="ask">Ask</TabsTrigger>
              <TabsTrigger value="memory">Memory</TabsTrigger>
            </TabsList>
            <TabsContent className="min-h-0" value="tasks">
              <TaskWorkspace
                onOpenWorktree={onOpenProject}
                repositoryId={project.repositoryId}
              />
            </TabsContent>
            <TabsContent className="min-h-0" value="explorer">
              <FileExplorer
                activePath={activePath}
                onOpen={(path) => void openFile(path)}
                repositoryId={project.repositoryId}
                repositoryName={project.name}
              />
            </TabsContent>
            <TabsContent className="min-h-0" value="brain">
              <BrainSearch
                onOpenSource={(path, line) => void openFile(path, false, line)}
                repositoryId={project.repositoryId}
              />
            </TabsContent>
            <TabsContent className="min-h-0" value="ask">
              <BrainAnswer
                onOpenSource={(path, line) => void openFile(path, false, line)}
                repositoryId={project.repositoryId}
              />
            </TabsContent>
            <TabsContent className="min-h-0" value="memory">
              <MemoryApproval projectId={project.projectId} />
            </TabsContent>
          </Tabs>
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel defaultSize="78%" minSize="45%">
          <ResizablePanelGroup orientation="vertical">
            <ResizablePanel defaultSize="70%" minSize="35%">
              <div className="flex h-full min-h-0 flex-col bg-background">
                {files.length > 0 ? (
                  <Tabs
                    className="min-h-0 flex-1 gap-0"
                    onValueChange={(value) => setActivePath(value)}
                    value={activePath ?? undefined}
                  >
                    <TabsList className="h-10 w-full justify-start overflow-x-auto rounded-none border-b bg-card px-2" variant="line">
                      {files.map((file) => (
                        <TabsTrigger className="max-w-52 px-3 text-xs" key={file.document.relativePath} value={file.document.relativePath}>
                          <span className="truncate">{file.document.relativePath.split("/").at(-1)}</span>
                          {file.draft !== file.document.content ? <span className="text-primary">●</span> : null}
                        </TabsTrigger>
                      ))}
                    </TabsList>
                    {activeFile ? (
                      <TabsContent className="min-h-0" value={activeFile.document.relativePath}>
                        <Suspense fallback={<div className="grid h-full place-items-center text-xs text-muted-foreground">Loading editor…</div>}>
                          <CodeEditor
                            key={activeFile.document.relativePath}
                            document={activeFile.document}
                            onChange={(draft) => updateActive((file) => ({ ...file, draft }))}
                            onReload={() => void openFile(activeFile.document.relativePath, true)}
                            onSaved={(document) =>
                              updateActive(() => ({ document, draft: document.content }))
                            }
                            repositoryId={project.repositoryId}
                            revealLine={revealLine}
                            value={activeFile.draft}
                          />
                        </Suspense>
                      </TabsContent>
                    ) : null}
                  </Tabs>
                ) : (
                  <div className="grid h-full place-items-center">
                    <div className="flex max-w-sm flex-col items-center gap-3 text-center text-muted-foreground">
                      <Code2Icon className="size-8 text-primary" />
                      <p className="text-sm text-foreground">Select a file from Explorer</p>
                      <p className="text-xs">Binary, oversized, ignored, and out-of-root files are blocked by the native core.</p>
                    </div>
                  </div>
                )}
                {error ? (
                  <Alert className="absolute bottom-4 right-4 z-20 max-w-md" variant="destructive">
                    <AlertCircleIcon />
                    <AlertTitle>File could not be opened</AlertTitle>
                    <AlertDescription>{error}</AlertDescription>
                  </Alert>
                ) : null}
              </div>
            </ResizablePanel>
            <ResizableHandle withHandle />
            <ResizablePanel defaultSize="30%" minSize="18%">
              <TerminalPanel project={project} />
            </ResizablePanel>
          </ResizablePanelGroup>
        </ResizablePanel>
      </ResizablePanelGroup>
    </section>
  );
}
