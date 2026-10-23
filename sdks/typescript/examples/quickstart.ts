/**
 * Voltrus quickstart: connect, verify the server, read live data.
 *
 * Environment:
 *   VOLTRUS_BASE_URL    server base URL          (default http://localhost:3000)
 *   VOLTRUS_API_KEY     API key credential       (preferred for scripts)
 *   VOLTRUS_USERNAME    local login alternative  (used when VOLTRUS_API_KEY is unset)
 *   VOLTRUS_PASSWORD    local login alternative
 *
 * Run: npx tsx examples/quickstart.ts
 */
import { VoltrusClient } from '../src';

function requiredEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    console.error(`Missing environment variable: ${name}`);
    process.exit(1);
  }
  return value;
}

async function credentials(client: VoltrusClient): Promise<void> {
  const apiKey = process.env.VOLTRUS_API_KEY;
  if (apiKey) {
    console.log('Auth: X-API-Key');
    return;
  }
  console.log('Auth: username/password (cookie session)');
  const session = await client.auth.login(requiredEnv('VOLTRUS_USERNAME'), requiredEnv('VOLTRUS_PASSWORD'));
  console.log(`Logged in as ${session.username} (role: ${session.role})`);
}

async function main(): Promise<void> {
  const client = new VoltrusClient({
    basePath: process.env.VOLTRUS_BASE_URL ?? 'http://localhost:3000',
    apiKey: process.env.VOLTRUS_API_KEY,
    timeout: 5000,
  });

  await credentials(client);

  const version = await client.system.getVersion();
  console.log(`Voltrus server v${version.version}`);

  const live = await client.system.getLive();
  console.log(`\nDevices (${live.devices.length}):`);
  for (const device of live.devices) {
    const metrics = Object.entries(device.metrics)
      .map(([name, value]) => `${name}=${value}`)
      .join('  ');
    console.log(`  [${device.status}] ${device.device_id}  ${metrics}`);
  }

  const readings = await client.data.getData();
  console.log(`\nData sources (${readings.length}):`);
  for (const reading of readings) {
    console.log(`  ${reading.key.padEnd(24)} ${String(reading.value).padStart(10)}  ${reading.status}`);
  }
}

main().catch((error) => {
  console.error('Quickstart failed:', error instanceof Error ? error.message : error);
  process.exit(1);
});
