import { AuthApi } from './apis/auth';
import { AlarmsApi } from './apis/alarms';
import { CommandsApi } from './apis/commands';
import { DataApi } from './apis/data';
import { DevicesApi } from './apis/devices';
import { OeeApi } from './apis/oee';
import { ReportsApi } from './apis/reports';
import { ScreensApi } from './apis/screens';
import { SystemApi } from './apis/system';
import { Configuration, ConfigurationParameters } from './configuration';
import { HttpClient } from './http';

export { Configuration } from './configuration';
export type { ConfigurationParameters } from './configuration';
export { HttpClient } from './http';
export { VoltrusApiError } from './errors';
export { AuthApi } from './apis/auth';
export { AlarmsApi } from './apis/alarms';
export { CommandsApi } from './apis/commands';
export { DataApi } from './apis/data';
export { DevicesApi } from './apis/devices';
export { OeeApi } from './apis/oee';
export { ReportsApi } from './apis/reports';
export { ScreensApi } from './apis/screens';
export { SystemApi } from './apis/system';
export * from './types';

/**
 * Official TypeScript client for the Voltrus SCADA server.
 *
 * One client holds a single HTTP session, so a cookie-based login through
 * `client.auth.login()` is shared by every API group. Construct the API
 * classes directly instead (they accept an `HttpClient`) only if you need to
 * split credentials across isolated sessions.
 */
export class VoltrusClient {
  readonly auth: AuthApi;
  readonly system: SystemApi;
  readonly data: DataApi;
  readonly alarms: AlarmsApi;
  readonly commands: CommandsApi;
  readonly devices: DevicesApi;
  readonly screens: ScreensApi;
  readonly oee: OeeApi;
  readonly reports: ReportsApi;

  constructor(configuration: Configuration | ConfigurationParameters = {}) {
    const config =
      configuration instanceof Configuration ? configuration : new Configuration(configuration);
    const http = new HttpClient(config);

    this.auth = new AuthApi(http);
    this.system = new SystemApi(http);
    this.data = new DataApi(http);
    this.alarms = new AlarmsApi(http);
    this.commands = new CommandsApi(http);
    this.devices = new DevicesApi(http);
    this.screens = new ScreensApi(http);
    this.oee = new OeeApi(http);
    this.reports = new ReportsApi(http);
  }
}

export default VoltrusClient;
