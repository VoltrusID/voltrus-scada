import type { HttpClient } from '../http';
import type {
  BacnetDevice,
  BacnetDeviceInput,
  BacnetObject,
  BacnetObjectInput,
  Dnp3Device,
  Dnp3DeviceInput,
  Dnp3Point,
  Dnp3PointInput,
  EnipDevice,
  EnipDeviceInput,
  EnipTag,
  EnipTagInput,
  MqttBrokerStatus,
  ModbusDevice,
  ModbusDeviceInput,
  OpcUaDevice,
  OpcUaDeviceInput,
  OpcUaNode,
  OpcUaNodeInput,
  S7Device,
  S7DeviceInput,
} from '../types';

/**
 * Protocol device management. Read endpoints are available to any
 * authenticated user; create/update/delete, start/stop and browse require the
 * admin role.
 */
export class DevicesApi {
  constructor(private readonly http: HttpClient) {}

  // ── Modbus ─────────────────────────────────────────────────

  async listModbus(): Promise<ModbusDevice[]> {
    return this.http.get('/api/v1/modbus-devices');
  }

  async getModbus(id: string): Promise<ModbusDevice> {
    return this.http.get(`/api/v1/modbus-devices/${id}`);
  }

  async createModbus(input: ModbusDeviceInput): Promise<{ ok: boolean; id: string }> {
    return this.http.post('/api/v1/modbus-devices', input);
  }

  async updateModbus(id: string, input: Partial<ModbusDeviceInput>): Promise<void> {
    await this.http.put(`/api/v1/modbus-devices/${id}`, input);
  }

  async deleteModbus(id: string): Promise<void> {
    await this.http.delete(`/api/v1/modbus-devices/${id}`);
  }

  /** Read device connectivity + per-register poller health. */
  async getModbusPollerStatuses(): Promise<Record<string, unknown>[]> {
    return this.http.get('/api/v1/modbus-devices/status');
  }

  async restartModbusPoller(id: string): Promise<void> {
    await this.http.post(`/api/v1/modbus-devices/${id}/restart`);
  }

  // ── OPC-UA ─────────────────────────────────────────────────

  async listOpcUa(): Promise<OpcUaDevice[]> {
    return this.http.get('/api/v1/opcua/devices');
  }

  async createOpcUa(input: OpcUaDeviceInput): Promise<{ ok: boolean; id: number }> {
    return this.http.post('/api/v1/opcua/devices', input);
  }

  async updateOpcUa(id: number, input: Partial<OpcUaDeviceInput>): Promise<void> {
    await this.http.put(`/api/v1/opcua/devices/${id}`, input);
  }

  async deleteOpcUa(id: number): Promise<void> {
    await this.http.delete(`/api/v1/opcua/devices/${id}`);
  }

  async listOpcUaNodes(id: number): Promise<OpcUaNode[]> {
    return this.http.get(`/api/v1/opcua/devices/${id}/nodes`);
  }

  async addOpcUaNode(id: number, input: OpcUaNodeInput): Promise<void> {
    await this.http.post(`/api/v1/opcua/devices/${id}/nodes`, input);
  }

  async removeOpcUaNode(id: number, nodeId: number): Promise<void> {
    await this.http.delete(`/api/v1/opcua/devices/${id}/nodes/${nodeId}`);
  }

  async startOpcUa(id: number): Promise<void> {
    await this.http.post(`/api/v1/opcua/devices/${id}/start`);
  }

  async stopOpcUa(id: number): Promise<void> {
    await this.http.post(`/api/v1/opcua/devices/${id}/stop`);
  }

  // ── S7 ─────────────────────────────────────────────────────

  async listS7(): Promise<S7Device[]> {
    return this.http.get('/api/v1/s7/devices');
  }

  async createS7(input: S7DeviceInput): Promise<{ ok: boolean; id: number }> {
    return this.http.post('/api/v1/s7/devices', input);
  }

  async updateS7(id: number, input: Partial<S7DeviceInput>): Promise<void> {
    await this.http.put(`/api/v1/s7/devices/${id}`, input);
  }

  async deleteS7(id: number): Promise<void> {
    await this.http.delete(`/api/v1/s7/devices/${id}`);
  }

  async startS7(id: number): Promise<void> {
    await this.http.post(`/api/v1/s7/devices/${id}/start`);
  }

  async stopS7(id: number): Promise<void> {
    await this.http.post(`/api/v1/s7/devices/${id}/stop`);
  }

  // ── EtherNet/IP ────────────────────────────────────────────

  async listEnip(): Promise<EnipDevice[]> {
    return this.http.get('/api/v1/enip/devices');
  }

  async createEnip(input: EnipDeviceInput): Promise<{ ok: boolean; id: number }> {
    return this.http.post('/api/v1/enip/devices', input);
  }

  async updateEnip(id: number, input: Partial<EnipDeviceInput>): Promise<void> {
    await this.http.put(`/api/v1/enip/devices/${id}`, input);
  }

  async deleteEnip(id: number): Promise<void> {
    await this.http.delete(`/api/v1/enip/devices/${id}`);
  }

  async listEnipTags(id: number): Promise<EnipTag[]> {
    return this.http.get(`/api/v1/enip/devices/${id}/tags`);
  }

  async addEnipTag(id: number, input: EnipTagInput): Promise<void> {
    await this.http.post(`/api/v1/enip/devices/${id}/tags`, input);
  }

  async removeEnipTag(id: number, tagId: number): Promise<void> {
    await this.http.delete(`/api/v1/enip/devices/${id}/tags/${tagId}`);
  }

  async startEnip(id: number): Promise<void> {
    await this.http.post(`/api/v1/enip/devices/${id}/start`);
  }

  async stopEnip(id: number): Promise<void> {
    await this.http.post(`/api/v1/enip/devices/${id}/stop`);
  }

  // ── DNP3 ───────────────────────────────────────────────────

  async listDnp3(): Promise<Dnp3Device[]> {
    return this.http.get('/api/v1/dnp3/devices');
  }

  async getDnp3(id: number): Promise<Dnp3Device> {
    return this.http.get(`/api/v1/dnp3/devices/${id}`);
  }

  async createDnp3(input: Dnp3DeviceInput): Promise<{ ok: boolean; id: number }> {
    return this.http.post('/api/v1/dnp3/devices', input);
  }

  async updateDnp3(id: number, input: Partial<Dnp3DeviceInput>): Promise<void> {
    await this.http.put(`/api/v1/dnp3/devices/${id}`, input);
  }

  async deleteDnp3(id: number): Promise<void> {
    await this.http.delete(`/api/v1/dnp3/devices/${id}`);
  }

  async addDnp3Point(id: number, input: Dnp3PointInput): Promise<void> {
    await this.http.post(`/api/v1/dnp3/devices/${id}/points`, input);
  }

  async removeDnp3Point(id: number, pointId: number): Promise<void> {
    await this.http.delete(`/api/v1/dnp3/devices/${id}/points/${pointId}`);
  }

  async scanDnp3Network(): Promise<unknown> {
    return this.http.post('/api/v1/dnp3/scan');
  }

  async startDnp3(id: number): Promise<void> {
    await this.http.post(`/api/v1/dnp3/devices/${id}/start`);
  }

  async stopDnp3(id: number): Promise<void> {
    await this.http.post(`/api/v1/dnp3/devices/${id}/stop`);
  }

  /** Trigger a full DNP3 integrity poll on a device. */
  async triggerDnp3Integrity(id: number): Promise<void> {
    await this.http.post(`/api/v1/dnp3/devices/${id}/integrity`);
  }

  // ── BACnet ─────────────────────────────────────────────────

  async listBacnet(): Promise<BacnetDevice[]> {
    return this.http.get('/api/v1/bacnet/devices');
  }

  async createBacnet(input: BacnetDeviceInput): Promise<{ ok: boolean; id: number }> {
    return this.http.post('/api/v1/bacnet/devices', input);
  }

  async updateBacnet(id: number, input: Partial<BacnetDeviceInput>): Promise<void> {
    await this.http.put(`/api/v1/bacnet/devices/${id}`, input);
  }

  async deleteBacnet(id: number): Promise<void> {
    await this.http.delete(`/api/v1/bacnet/devices/${id}`);
  }

  async listBacnetObjects(id: number): Promise<BacnetObject[]> {
    return this.http.get(`/api/v1/bacnet/devices/${id}/objects`);
  }

  async addBacnetObject(id: number, input: BacnetObjectInput): Promise<void> {
    await this.http.post(`/api/v1/bacnet/devices/${id}/objects`, input);
  }

  async removeBacnetObject(id: number, objectId: number): Promise<void> {
    await this.http.delete(`/api/v1/bacnet/devices/${id}/objects/${objectId}`);
  }

  async startBacnet(id: number): Promise<void> {
    await this.http.post(`/api/v1/bacnet/devices/${id}/start`);
  }

  async stopBacnet(id: number): Promise<void> {
    await this.http.post(`/api/v1/bacnet/devices/${id}/stop`);
  }

  // ── MQTT broker (embedded) ─────────────────────────────────

  async getMqttBrokerStatus(): Promise<MqttBrokerStatus> {
    return this.http.get('/api/v1/mqtt-broker/status');
  }

  async getMqttTopics(): Promise<Record<string, unknown>> {
    return this.http.get('/api/v1/mqtt-broker/topics');
  }

  async getMqttMessages(params?: { topic?: string; limit?: number }): Promise<unknown> {
    return this.http.get('/api/v1/mqtt-broker/messages', params);
  }
}
