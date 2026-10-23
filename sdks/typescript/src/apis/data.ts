import type { HttpClient } from '../http';
import type {
  DataQueryParams,
  DataSourceReading,
  ExportQueryParams,
  ExportResponse,
  HistoryResponse,
} from '../types';

export class DataApi {
  constructor(private readonly http: HttpClient) {}

  /** Current evaluated values for a screen (or all data sources). */
  async getData(params?: DataQueryParams): Promise<DataSourceReading[]> {
    return this.http.get('/api/v1/data', params);
  }

  /** Historical series in compact form: values[], start (unix ms), interval (ms). */
  async getHistory(params?: DataQueryParams): Promise<HistoryResponse> {
    return this.http.get('/api/v1/history', params);
  }

  /** Ad-hoc CSV export from history (`sensor` + `range`). */
  async exportCsv(params?: DataQueryParams): Promise<string> {
    return this.http.getRaw('/api/v1/export/csv', { params }).then((res) => res.data);
  }

  /** Paginated telemetry export (JSON envelope or CSV when format: "csv"). */
  async exportData(params?: ExportQueryParams): Promise<ExportResponse | string> {
    if (params?.format === 'csv') {
      return this.http.getRaw('/api/v1/data/export', { params }).then((res) => res.data);
    }
    return this.http.get('/api/v1/data/export', params);
  }
}
