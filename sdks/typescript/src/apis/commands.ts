import type { HttpClient } from '../http';
import type {
  Command,
  CommandAuditEntry,
  CommandListParams,
  CommandStatusValue,
  WritePolicy,
  WritePolicyUpdate,
  WriteQueuedResponse,
  WriteRequest,
} from '../types';

/**
 * Delay primitive. Uses the Promise executor form because
 * Promise.withResolvers() requires Node 22+ and this package supports Node 18.
 */
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export class CommandsApi {
  constructor(private readonly http: HttpClient) {}

  /**
   * Queue a write to a tag (data source). The server enforces the tag's write
   * policy (range, deadband, rate limit, reason requirement) and returns HTTP
   * 202 with a command_id to poll.
   */
  async writeTag(tagId: number, request: WriteRequest): Promise<WriteQueuedResponse> {
    return this.http.post(`/api/v1/tags/${tagId}/write`, request);
  }

  async getStatus(commandId: string): Promise<Command> {
    return this.http.get(`/api/v1/commands/${commandId}`);
  }

  /** Poll until the command reaches a terminal state (acked/failed/expired/cancelled). */
  async waitForCompletion(
    commandId: string,
    options: { intervalMs?: number; timeoutMs?: number } = {},
  ): Promise<Command> {
    const intervalMs = options.intervalMs ?? 500;
    const timeoutMs = options.timeoutMs ?? 30000;
    const deadline = Date.now() + timeoutMs;
    const terminal: CommandStatusValue[] = ['acked', 'failed', 'expired', 'cancelled'];

    for (;;) {
      const command = await this.getStatus(commandId);
      if (terminal.includes(command.status)) return command;
      if (Date.now() >= deadline) {
        throw new Error(`Command ${commandId} did not reach a terminal state within ${timeoutMs}ms`);
      }
      await sleep(intervalMs);
    }
  }

  async list(params?: CommandListParams): Promise<Command[]> {
    return this.http.get('/api/v1/commands', params);
  }

  async cancel(commandId: string): Promise<void> {
    await this.http.post(`/api/v1/commands/${commandId}/cancel`);
  }

  async getAudit(commandId: string): Promise<CommandAuditEntry[]> {
    return this.http.get(`/api/v1/commands/${commandId}/audit`);
  }

  async getWritePolicy(dataSourceId: number): Promise<WritePolicy> {
    return this.http.get(`/api/v1/data-sources/${dataSourceId}/write-policy`);
  }

  async updateWritePolicy(dataSourceId: number, update: WritePolicyUpdate): Promise<void> {
    await this.http.patch(`/api/v1/data-sources/${dataSourceId}/write-policy`, update);
  }
}
