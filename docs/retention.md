# Retention of events, audit trails and database space

The historian tiers (raw, hourly, daily samples) are described in
[historian.md](historian.md). This page covers everything else that grows
while Voltrus runs, and how the database file gives space back.

All of it runs in the same worker as the historian tiers, every
`retention.run_interval_secs` (default 6 hours), on the active node only. A
passive node of a redundant pair deletes nothing and keeps what its peer
replicates.

## What is trimmed

| Data | Tables | Default keep | Key |
|---|---|---|---|
| Login sessions | `refresh_tokens` | expired and revoked rows are removed every cycle | none |
| Alarm history | cleared `alarms` and their `alarm_journal` rows | 2 years | `event_keep_secs` |
| Downtime history | ended `downtime_events` | 2 years | `event_keep_secs` |
| Security events | `security_log` | 2 years | `security_keep_secs` |
| Flow and script run history | `flow_runs`, `script_executions` | 90 days | `run_log_keep_secs` |
| Audit trails | `audit_log`, `alarm_audit_log`, finished `commands` with their `command_audit_log` | **forever** | `audit_keep_secs` |
| Electronic signatures (21 CFR Part 11) | `electronic_signatures` | **forever, not configurable** | none |

Every `*_keep_secs` value is a number of seconds. `0` or `null` keeps the
rows forever. A key that is missing from `config.yaml` has the default above,
so existing configurations need no change.

```yaml
retention:
  event_keep_secs: 63072000      # 2 years: cleared alarms, journal, downtime events
  security_keep_secs: 63072000   # 2 years: security event log
  run_log_keep_secs: 7776000     # 90 days: flow runs and script executions
  audit_keep_secs: null          # forever (default); set seconds to opt in
  purge_batch_rows: 5000         # rows per delete transaction
  vacuum_step_pages: 2000        # free pages returned to the OS per step; 0 = off
```

`retention.enabled: false` switches off every trim above and the historian
tiers. Expired refresh tokens are still removed, because nothing else would
ever delete them and they are not data anyone keeps.

### Why 2 years

Alarm and downtime history is what an annual review, an incident
investigation or a year-over-year comparison of alarm rates reads. Two years
covers two annual review cycles with the previous year still available. It is
a default, not a regulation: nothing in the core requires a period, so set what
your site policy says.

For scale: in the soak simulation a plant whose alarms chatter around their
thresholds (about 41 alarms an hour) added about 250 MB a year across the alarm,
alarm audit, journal and downtime tables. Most of that is the alarm audit
trail, which is kept by default (below); a site that does not need it forever
can bound it with `audit_keep_secs`.

### What is never deleted

* An alarm that is not cleared: active, acknowledged but not yet cleared, or
  shelved. Only an alarm whose `resolved_at` is older than the keep goes, and
  an alarm that was cleared without ever being acknowledged is treated like any
  other cleared alarm (the alarm summary does not count it as waiting).
* A downtime event that is still open.
* A command that has not reached a final state (`queued`, `pending`, `sent`).
* Electronic signature records, under any configuration.

An alarm's audit trail (`alarm_audit_log`) is **not** deleted with the alarm.
This release rebuilds that table once at startup to drop its
`ON DELETE CASCADE` foreign key (rows are kept), so the trail of an alarm
survives the alarm and follows `audit_keep_secs` only.

## Audit trails and compliance modes

The audit trails are what 21 CFR Part 11 and IEC 62443 audits ask for, so they
are kept forever unless you opt in with `audit_keep_secs`, and even then only
when no compliance mode is licensed.

The core does not run those modes; the Enterprise add-on does. What the core
can see is the entitlement the add-on resolves at startup
(`LicenseTier`). The worker reads it at every cycle:

| Licensed | `audit_keep_secs` | `security_keep_secs` |
|---|---|---|
| neither | honoured | honoured |
| 21 CFR Part 11 | ignored, kept forever | honoured |
| IEC 62443 | ignored, kept forever | ignored, kept forever |

Ignoring a configured value is logged once per cycle. The Storage tab and
`GET /api/v1/compression/config` show the values in force
(`audit_keep_secs: null`, `compliance_hold: true`), not the configured ones.

An add-on that implements a compliance mode without a licence entitlement
must keep its own tables out of this worker's reach; the core only deletes
the tables listed above.

## Batches

Every delete runs in transactions of at most `purge_batch_rows` rows (default
5000) with a 25 ms pause between them. The database has one shared connection,
so a first purge of years of rows would otherwise stall the pollers and the API
for as long as it takes. One failing table is logged and the others still run.

## Giving disk space back

Deleting rows frees pages inside the database file, but SQLite does not shrink
the file by itself.

* **New databases** are created with `auto_vacuum=INCREMENTAL`. After each
  cycle the worker returns the free pages in steps of `vacuum_step_pages`
  (default 2000, 8 MB with 4 KiB pages) until none are left. This also gives
  back the space that historian roll-ups free.
* **Existing databases** keep `auto_vacuum=NONE`: freed pages are reused by new
  rows, so the file stops growing once it has reached its working size, but it
  never gets smaller. To reclaim the space and switch the file to incremental
  mode, stop the server and run once:

  ```bash
  voltrus db vacuum [config.yaml]
  ```

  It rebuilds the file (`VACUUM`), needs free disk space equal to the database
  size, and holds the database for the whole rebuild, which is why the worker
  never runs it. `voltrus db stats` shows the free pages and which mode the
  file is in.
* **The WAL file** is truncated back to 64 MB after a checkpoint
  (`journal_size_limit`). Before, it kept the size of its largest burst
  forever.

## Not trimmed

These are bounded by what an operator does, not by time, and stay until
someone deletes them: configuration, configuration snapshots, annotations,
shift handovers, maintenance logs and the historian tiers (see
[historian.md](historian.md)). Camera snapshots follow the camera's own
retention, and user notifications are cleared after 30 days when a user lists
them.
