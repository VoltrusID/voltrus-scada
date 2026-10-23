import type { HttpClient } from '../http';
import type {
  CloseDowntimeRequest,
  CreateDowntimeRequest,
  CreateShiftRequest,
  DowntimeEvent,
  DowntimeListParams,
  OeeMetrics,
  OeeMetricsParams,
  ShiftConfig,
} from '../types';

/** OEE endpoints. */
export class OeeApi {
  constructor(private readonly http: HttpClient) {}

  /** Availability / performance / quality / OEE breakdown for one device. */
  async getMetrics(params: OeeMetricsParams): Promise<OeeMetrics> {
    return this.http.get('/api/v1/oee/metrics', params);
  }

  async listDowntime(params?: DowntimeListParams): Promise<DowntimeEvent[]> {
    return this.http.get('/api/v1/oee/downtime', params);
  }

  async createDowntime(input: CreateDowntimeRequest): Promise<void> {
    await this.http.post('/api/v1/oee/downtime', input);
  }

  async closeDowntime(id: number, reason?: string): Promise<void> {
    const body: CloseDowntimeRequest = { reason };
    await this.http.put(`/api/v1/oee/downtime/${id}/close`, body);
  }

  async listShifts(): Promise<ShiftConfig[]> {
    return this.http.get('/api/v1/oee/shifts');
  }

  async createShift(input: CreateShiftRequest): Promise<void> {
    await this.http.post('/api/v1/oee/shifts', input);
  }

  async deleteShift(id: number): Promise<void> {
    await this.http.delete(`/api/v1/oee/shifts/${id}`);
  }
}
