/** Error thrown for any non-2xx Voltrus API response. */
export class VoltrusApiError extends Error {
  readonly status: number;
  readonly method: string;
  readonly path: string;
  /** Raw response body when it was not a JSON `{ error }` envelope. */
  readonly detail?: unknown;

  constructor(status: number, method: string, path: string, message: string, detail?: unknown) {
    super(`Voltrus API ${method} ${path} failed (${status}): ${message}`);
    this.name = 'VoltrusApiError';
    this.status = status;
    this.method = method;
    this.path = path;
    this.detail = detail;
  }
}
