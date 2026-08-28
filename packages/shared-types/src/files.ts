export interface FileEntry {
  relativePath: string;
  size: number;
}

export interface FilePage {
  entries: FileEntry[];
  nextCursor: string | null;
}

export interface FileDocument {
  relativePath: string;
  content: string;
  version: string;
  size: number;
}
