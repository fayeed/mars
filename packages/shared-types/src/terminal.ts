export interface TerminalSessionInfo {
  sessionId: string;
  cwd: string;
}

export interface TerminalOutput {
  sessionId: string;
  data: number[];
}
