import { invoke } from "@tauri-apps/api/core";

export interface StartupError {
  message: string;
  log_dir: string;
}

export interface ClaudeStatus {
  program: string | null;
  version: string | null;
  tested_version: string;
  mismatch: boolean;
}

export interface Status {
  port: number;
  server_error: string | null;
  claude: ClaudeStatus;
  schedules_paused: boolean;
  restart_required: boolean;
  data_dir: string;
}

export const commands = {
  startupError: () => invoke<StartupError | null>("startup_error"),
  status: () => invoke<Status>("status"),
};
