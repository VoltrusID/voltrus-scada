import axios, { AxiosError, AxiosInstance, AxiosRequestConfig, AxiosResponse } from 'axios';
import { Configuration } from './configuration';
import { VoltrusApiError } from './errors';

const SESSION_COOKIES: Record<string, true> = {
  voltrus_token: true,
  voltrus_refresh: true,
};
export class HttpClient {
  readonly config: Configuration;
  private readonly axios: AxiosInstance;
  private readonly cookies = new Map<string, string>();

  constructor(config: Configuration) {
    this.config = config;
    this.axios = axios.create({
      baseURL: config.basePath,
      timeout: config.timeout,
      // Browsers: the session cookies flow automatically. In Node we capture
      // and replay them ourselves (see captureSessionCookies).
      withCredentials: true,
      ...config.baseOptions,
    });

    this.axios.interceptors.request.use((req) => {
      if (config.apiKey) {
        // Server middleware treats an explicit API key as the sole auth method.
        req.headers.set('X-API-Key', config.apiKey);
      } else if (config.accessToken) {
        req.headers.set('Authorization', `Bearer ${config.accessToken}`);
      } else if (this.cookies.size > 0) {
        req.headers.set(
          'Cookie',
          [...this.cookies.entries()].map(([name, value]) => `${name}=${value}`).join('; '),
        );
      }
      return req;
    });
  }

  /**
   * Store voltrus session cookies from a login/refresh response. Browsers
   * ignore this (their HTTP stack keeps cookies); Node clients need it
   * because the local login delivers the token only as HttpOnly cookies.
   */
  captureSessionCookies(response: AxiosResponse): void {
    const setCookie = response.headers?.['set-cookie'];
    if (!Array.isArray(setCookie)) return;
    for (const line of setCookie) {
      const pair = line.split(';', 1)[0];
      const eq = pair.indexOf('=');
      if (eq <= 0) continue;
      const name = pair.slice(0, eq).trim();
      const value = pair.slice(eq + 1).trim();
      if (SESSION_COOKIES[name]) this.cookies.set(name, value);
    }
  }

  hasSessionCookies(): boolean {
    return this.cookies.size > 0;
  }

  clearSessionCookies(): void {
    this.cookies.clear();
  }

  private static toApiError(error: unknown): VoltrusApiError {
    if (error instanceof VoltrusApiError) return error;
    const axiosError = error as AxiosError;
    const response = axiosError?.response;
    if (response) {
      const data = response.data as { error?: string } | string | undefined;
      const message =
        (typeof data === 'object' && data?.error) ||
        (typeof data === 'string' && data) ||
        axiosError.message;
      return new VoltrusApiError(
        response.status,
        response.config?.method?.toUpperCase() ?? '',
        response.config?.url ?? '',
        message,
        typeof data === 'object' && data?.error ? undefined : data,
      );
    }
    return new VoltrusApiError(0, axiosError?.config?.method?.toUpperCase() ?? '', axiosError?.config?.url ?? '', axiosError?.message ?? String(error));
  }

  async get<T>(path: string, params?: object): Promise<T> {
    try {
      const res = await this.axios.get<T>(path, { params });
      return res.data;
    } catch (error) {
      throw HttpClient.toApiError(error);
    }
  }

  /** GET returning the raw response, for callers that need headers or raw text. */
  async getRaw(path: string, config?: AxiosRequestConfig): Promise<AxiosResponse> {
    try {
      return await this.axios.get(path, config);
    } catch (error) {
      throw HttpClient.toApiError(error);
    }
  }

  async post<T>(path: string, body?: unknown): Promise<T> {
    try {
      const res = await this.axios.post<T>(path, body);
      return res.data;
    } catch (error) {
      throw HttpClient.toApiError(error);
    }
  }

  /** POST returning the raw response, so login can capture Set-Cookie headers. */
  async postRaw(path: string, body?: unknown): Promise<AxiosResponse> {
    try {
      return await this.axios.post(path, body);
    } catch (error) {
      throw HttpClient.toApiError(error);
    }
  }

  async put<T>(path: string, body?: unknown): Promise<T> {
    try {
      const res = await this.axios.put<T>(path, body);
      return res.data;
    } catch (error) {
      throw HttpClient.toApiError(error);
    }
  }

  async patch<T>(path: string, body?: unknown): Promise<T> {
    try {
      const res = await this.axios.patch<T>(path, body);
      return res.data;
    } catch (error) {
      throw HttpClient.toApiError(error);
    }
  }

  async delete<T>(path: string): Promise<T> {
    try {
      const res = await this.axios.delete<T>(path);
      return res.data;
    } catch (error) {
      throw HttpClient.toApiError(error);
    }
  }
}
