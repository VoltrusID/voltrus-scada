/**
 * Always-on device monitoring: poll live snapshots on a fixed interval,
 * forever, with simple offline detection and SIGINT-safe shutdown. This is
 * the pattern for 24/7 services that watch a Voltrus server.
 *
 * Environment:
 *   VOLTRUS_BASE_URL            server base URL   (default http://localhost:3000)
 *   VOLTRUS_API_KEY             API key credential
 *   VOLTRUS_USERNAME            local login alternative
 *   VOLTRUS_PASSWORD            local login alternative
 *   VOLTRUS_POLL_INTERVAL_MS    poll interval     (default 2000)
 *
 * Run: npx tsx examples/devices-24x7.ts
 */
import { VoltrusClient } from '../src';

const intervalMs = Number(process.env.VOLTRUS_POLL_INTERVAL_MS ?? 2000);
const MAX_BACKOFF_MS = 30000;

async function connect(): Promise<VoltrusClient> {
  const client = new VoltrusClient({
    basePath: process.env.VOLTRUS_BASE_URL ?? 'http://localhost:3000',
    apiKey: process.env.VOLTRUS_API_KEY,
    timeout: Math.min(intervalMs, 5000),
  });
  if (!process.env.VOLTRUS_API_KEY) {
    const username = process.env.VOLTRUS_USERNAME;
    const password = process.env.VOLTRUS_PASSWORD;
    if (!username || !password) {
      console.error('Set VOLTRUS_API_KEY or VOLTRUS_USERNAME + VOLTRUS_PASSWORD');
      process.exit(1);
    }
    await client.auth.login(username, password);
  }
  return client;
}

async function pollOnce(client: VoltrusClient, pollCount: number): Promise<void> {
  const live = await client.system.getLive();
  const online = live.devices.filter((d) => d.status === 'online');
  const stamp = new Date().toISOString();

  console.log(`\n[${stamp}] poll #${pollCount}: ${online.length}/${live.devices.length} devices online`);
  for (const device of live.devices) {
    const metrics = Object.entries(device.metrics)
      .slice(0, 4)
      .map(([name, value]) => `${name}=${value}`)
      .join('  ');
    const marker = device.status === 'online' ? '+' : 'x';
    console.log(`  [${marker}] ${device.device_id.padEnd(24)} ${metrics}`);
  }
}

async function main(): Promise<void> {
  const client = await connect();
  console.log(`Watching ${process.env.VOLTRUS_BASE_URL ?? 'http://localhost:3000'} every ${intervalMs}ms — Ctrl+C to stop.`);

  let consecutiveFailures = 0;
  let polls = 0;
  let running = true;

  process.on('SIGINT', () => {
    console.log(`\nShutting down after ${polls} polls.`);
    running = false;
  });

  while (running) {
    try {
      polls += 1;
      await pollOnce(client, polls);
      consecutiveFailures = 0;
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    } catch (error) {
      consecutiveFailures += 1;
      // Linear backoff: the server may be restarting for maintenance; keep
      // trying instead of crashing the watcher.
      const backoff = Math.min(intervalMs * consecutiveFailures, MAX_BACKOFF_MS);
      const message = error instanceof Error ? error.message : String(error);
      console.error(`Poll failed (${consecutiveFailures} in a row): ${message} — retrying in ${backoff}ms`);
      await new Promise((resolve) => setTimeout(resolve, backoff));
    }
  }
}

main().catch((error) => {
  console.error('Watcher failed:', error instanceof Error ? error.message : error);
  process.exit(1);
});
