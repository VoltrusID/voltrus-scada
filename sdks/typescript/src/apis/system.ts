import type { HttpClient } from '../http';
import type { HealthResponse, LiveResponse, SystemInfo, VersionInfo } from '../types';

export class SystemApi {
  constructor(private readonly http: HttpClient) {}

  /** Unauthenticated health check with per-device online status. */
  async getHealth(): Promise<HealthResponse> {
    return this.http.get('/api/v1/health');
  }

  /** Latest live snapshot for every polled device. */
  async getLive(): Promise<LiveResponse> {
    return this.http.get('/api/v1/live');
  }

  async getSystemInfo(): Promise<SystemInfo> {
    return this.http.get('/api/v1/system');
  }

  async getVersion(): Promise<VersionInfo> {
    return this.http.get('/api/v1/version');
  }
}
