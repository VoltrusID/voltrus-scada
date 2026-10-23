import type { HttpClient } from '../http';
import type {
  Alarm,
  AlarmAnalyticsParams,
  AlarmAuditEntry,
  AlarmAuditParams,
  AlarmListParams,
  AlarmSummary,
  ShelfAlarmRequest,
} from '../types';

export class AlarmsApi {
  constructor(private readonly http: HttpClient) {}

  async list(params?: AlarmListParams): Promise<Alarm[]> {
    return this.http.get('/api/v1/alarms', params);
  }

  async getSummary(): Promise<AlarmSummary> {
    return this.http.get('/api/v1/alarms/summary');
  }

  async getActiveCount(): Promise<number> {
    const res = await this.http.get<{ count: number }>('/api/v1/alarms/active/count');
    return res.count;
  }

  async acknowledge(alarmId: number): Promise<void> {
    await this.http.post(`/api/v1/alarms/${alarmId}/acknowledge`);
  }

  /** Acknowledge every currently active alarm (POST /api/v1/alarms). */
  async acknowledgeAll(): Promise<void> {
    await this.http.post('/api/v1/alarms');
  }

  /** Temporarily suppress an alarm. */
  async shelf(alarmId: number, durationSecs: number, reason: string): Promise<void> {
    const body: ShelfAlarmRequest = { duration_secs: durationSecs, reason };
    await this.http.post(`/api/v1/alarms/${alarmId}/shelf`, body);
  }

  async unshelf(alarmId: number): Promise<void> {
    await this.http.post(`/api/v1/alarms/${alarmId}/unshelf`);
  }

  async getAudit(params?: AlarmAuditParams): Promise<AlarmAuditEntry[]> {
    return this.http.get('/api/v1/alarms/audit', params);
  }

  async getAnalytics(params?: AlarmAnalyticsParams): Promise<Record<string, unknown>[]> {
    return this.http.get('/api/v1/alarms/analytics', params);
  }
}
