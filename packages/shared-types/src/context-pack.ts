import type { AdapterKind } from "./tasks";

export type ContextPackSection =
  | "instructions"
  | "context"
  | "code"
  | "history"
  | "skills";

export interface ContextPackItem {
  section: ContextPackSection;
  entityRef: string;
  title: string;
  content: string;
  provenance: string;
  freshness: string;
  sourceLocation: string;
  reason: string;
  score: number;
  byteSize: number;
}

export interface ContextPackExclusion {
  entityRef: string;
  reason: string;
}

export interface ContextPackSectionUsage {
  section: ContextPackSection;
  maxBytes: number;
  usedBytes: number;
}

export interface ContextPack {
  taskId: string;
  adapter: AdapterKind;
  maxBytes: number;
  totalBytes: number;
  sectionUsage: ContextPackSectionUsage[];
  items: ContextPackItem[];
  exclusions: ContextPackExclusion[];
}
