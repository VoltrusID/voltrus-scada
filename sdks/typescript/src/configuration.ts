import type { AxiosRequestConfig } from 'axios';

export interface ConfigurationParameters {
  /** Base URL of the Voltrus server, e.g. "http://plant-01.local:3000". */
  basePath?: string;
  /** API key sent as the X-API-Key header. Takes precedence over other credentials. */
  apiKey?: string;
  /** Bearer token (OIDC) sent as the Authorization header. */
  accessToken?: string;
  /** Username for cookie-based login via AuthApi.login(). */
  username?: string;
  /** Password for cookie-based login via AuthApi.login(). */
  password?: string;
  /** Request timeout in milliseconds. Default 10000. */
  timeout?: number;
  /** Extra axios options merged into every request. */
  baseOptions?: AxiosRequestConfig;
}

export class Configuration {
  basePath: string;
  apiKey?: string;
  accessToken?: string;
  username?: string;
  password?: string;
  timeout: number;
  baseOptions: AxiosRequestConfig;

  constructor(configuration: ConfigurationParameters = {}) {
    this.basePath = (configuration.basePath ?? 'http://localhost:3000').replace(/\/+$/, '');
    this.apiKey = configuration.apiKey;
    this.accessToken = configuration.accessToken;
    this.username = configuration.username;
    this.password = configuration.password;
    this.timeout = configuration.timeout ?? 10000;
    this.baseOptions = configuration.baseOptions ?? {};
  }
}
