import type { HttpClient } from '../http';
import type {
  AddScreenSensorRequest,
  DataSource,
  Screen,
  UpdateScreenSensorRequest,
} from '../types';

export class ScreensApi {
  constructor(private readonly http: HttpClient) {}

  async list(): Promise<Screen[]> {
    return this.http.get('/api/v1/screens');
  }

  async create(name: string): Promise<{ ok: boolean; id: string }> {
    return this.http.post('/api/v1/screens', { name });
  }

  async rename(id: string, name: string): Promise<void> {
    await this.http.put(`/api/v1/screens/${id}`, { name });
  }

  async delete(id: string): Promise<void> {
    await this.http.delete(`/api/v1/screens/${id}`);
  }

  /** Data sources bound to a screen. */
  async listSensors(id: string): Promise<DataSource[]> {
    return this.http.get(`/api/v1/screens/${id}/data-sources`);
  }

  async addSensor(id: string, dataSourceId: number): Promise<void> {
    const body: AddScreenSensorRequest = { data_source_id: dataSourceId };
    await this.http.post(`/api/v1/screens/${id}/data-sources`, body);
  }

  async updateSensorOrder(id: string, dataSourceId: number, sortOrder: number): Promise<void> {
    const body: UpdateScreenSensorRequest = { sort_order: sortOrder };
    await this.http.put(`/api/v1/screens/${id}/data-sources/${dataSourceId}`, body);
  }

  async removeSensor(id: string, dataSourceId: number): Promise<void> {
    await this.http.delete(`/api/v1/screens/${id}/data-sources/${dataSourceId}`);
  }
}
