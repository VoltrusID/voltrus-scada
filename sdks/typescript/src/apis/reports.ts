import type { HttpClient } from '../http';
import type { CreateReportRequest, Report, UpdateReportRequest } from '../types';

/** Scheduled PDF report management. Provided by the Enterprise add-on; a Core-only server returns 404. */
export class ReportsApi {
  constructor(private readonly http: HttpClient) {}

  async list(): Promise<Report[]> {
    return this.http.get('/api/v1/reports');
  }

  async create(input: CreateReportRequest): Promise<void> {
    await this.http.post('/api/v1/reports', input);
  }

  async update(id: number, input: UpdateReportRequest): Promise<void> {
    await this.http.put(`/api/v1/reports/${id}`, input);
  }

  async delete(id: number): Promise<void> {
    await this.http.delete(`/api/v1/reports/${id}`);
  }

  /** Generate a report immediately instead of waiting for its schedule. */
  async generate(id: number): Promise<void> {
    await this.http.post(`/api/v1/reports/${id}/generate`);
  }
}
