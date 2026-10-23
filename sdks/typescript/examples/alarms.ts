/**
 * Alarm monitoring: list alarms, show the summary, acknowledge the oldest
 * active critical alarm, shelf the rest of the hour.
 *
 * Environment:
 *   VOLTRUS_BASE_URL    server base URL          (default http://localhost:3000)
 *   VOLTRUS_API_KEY     API key credential
 *   VOLTRUS_USERNAME    local login alternative
 *   VOLTRUS_PASSWORD    local login alternative
 *
 * Run: npx tsx examples/alarms.ts
 */
import { VoltrusClient } from '../src';

async function main(): Promise<void> {
  const client = new VoltrusClient({
    basePath: process.env.VOLTRUS_BASE_URL ?? 'http://localhost:3000',
    apiKey: process.env.VOLTRUS_API_KEY,
    timeout: 5000,
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

  const summary = await client.alarms.getSummary();
  console.log('Alarm summary:');
  console.log(`  active critical/warning/info: ${summary.active_critical}/${summary.active_warning}/${summary.active_info}`);
  console.log(`  shelved: ${summary.shelved}   last 24h: ${summary.recent_24h}   avg ack: ${summary.avg_ack_time_secs}s`);

  const alarms = await client.alarms.list({ status: 'active', limit: 50 });
  console.log(`\nActive alarms (${alarms.length}):`);
  for (const alarm of alarms) {
    console.log(
      `  #${alarm.id} [${alarm.severity}] ${alarm.source_name}: ${alarm.message} ` +
        `(value ${alarm.value} vs threshold ${alarm.threshold}, since ${new Date(alarm.triggered_at * 1000).toISOString()})`,
    );
  }

  const critical = alarms.filter((a) => a.severity === 'critical');
  if (critical.length > 0) {
    const target = critical[0];
    await client.alarms.acknowledge(target.id);
    console.log(`\nAcknowledged alarm #${target.id} (${target.source_name})`);
    console.log(`Remaining active critical alarms: ${critical.length - 1}`);
  } else {
    console.log('\nNo active critical alarms — nothing to acknowledge.');
  }

  const watchlist = alarms.filter((a) => a.severity === 'warning');
  if (watchlist.length > 0) {
    await client.alarms.shelf(watchlist[0].id, 3600, 'SDK example: shelving for one hour');
    console.log(`Shelved alarm #${watchlist[0].id} for one hour.`);
  }
}

main().catch((error) => {
  console.error('Alarm example failed:', error instanceof Error ? error.message : error);
  process.exit(1);
});
