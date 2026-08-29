export type ContextKind = "text" | "image" | "file";

export interface ContextItem {
  id: string;
  repositoryId: string;
  kind: ContextKind;
  title: string;
  bodyText: string | null;
  originalUri: string | null;
  mimeType: string;
  contentHash: string;
  classification: string | null;
  quarantineReason: string | null;
  artifactPath: string | null;
  originalName: string | null;
  capturedAt: string;
}

export interface ContextArtifactContent {
  bytes: number[];
  mimeType: string;
  originalName: string | null;
}
