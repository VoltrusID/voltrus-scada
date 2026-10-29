# Historian: storage, compression and retention

Voltrus keeps its history in the same SQLite database as everything else. This
page describes how samples are stored, how they are compressed, how long they
are kept and what that costs in disk space.

## Tiers

| Tier | What it holds | Where | Default keep |
|---|---|---|---|
| Recent | Every sample as a plain row, so inserts and the live trend stay cheap | `telemetry` | `pack_after_secs`, 1 hour |
| Packed full resolution | The same samples, compressed, one block per tag and hour | `telemetry_blocks` | 90 days (`raw` tier `keep`) |
| Hourly | min, mean, max and sample count per tag and hour | `telemetry_hourly` | 90 days past the raw window (`hourly` tier `keep`) |
| Daily | the same per day | `telemetry_daily` | forever (`daily` tier `keep: null`) |

Recent and packed samples together are the *full resolution* tier. Readers
(trends, `/api/v1/history`, exports, analytics, SPC, BI and egress, MCP
`read_history`) see one stream: they cannot tell whether a sample is still a row
or already inside a block. Results are identical either way, including for
duplicate timestamps and for `limit` and page offsets.

## Write path and deadband

The persistence loop reads the cache every `persist_interval_secs` (5 s by
default) and writes each device's metrics. Before that, every tag may filter its
own writes. Put these keys in the tag's `source_config` (in project YAML they sit
under `source_config:` next to the tag's other settings):

| Key | Meaning |
|---|---|
| `history_deadband` | Store a sample only when it differs from the last *stored* sample by more than this. `0` stores every change. Absent: no filtering. |
| `history_deadband_type` | `absolute` (tag units, default) or `percent` of the last stored value |
| `history_max_interval_secs` | Also store a sample at least this often, so a flat signal still has points and a gap in a trend always means "no data" |

`deadband` (without the `history_` prefix) is the alarm hysteresis and is a
different setting. `GET /api/v1/compression/deadband` lists the tags that filter
their history.

In the web client: edit a tag (Edit panel, General tab, or the `cfg` popover in
Data Manager, Sources) and fill in *History storage*. To set it on many tags,
tick them in Data Manager, Sources and use *History deadband...* in the bar that
appears; *Remove deadband* puts them back to storing every sample. The values are
checked before saving (band 0 or more, percent at most 100, heartbeat a whole
number of seconds). System, Storage shows the stored sizes, the running
retention settings (read-only, they come from `config.yaml`) and the tags that
filter.

Tags without `history_deadband` store every sample, with one exception that
applies to all tags: a sample that repeats the last stored one exactly (same
timestamp and value, which is what the loop sees when a device has stopped
answering and its cached reading is old) is dropped.

## Compression

Every complete hour older than `pack_after_secs` is packed by a background worker
that runs every `pack_interval_secs` (10 minutes). Per tag and hour it:

1. reads the raw rows (and any block already stored for that hour, when late
   samples arrived, for instance from a CSV import),
2. encodes them into a block and decodes it again to check the result is
   bit-identical,
3. writes the block and deletes the raw rows, in one SQLite transaction.

A crash between steps leaves either the rows or the block, never neither, and the
next pass simply does the hour again. The worker releases the database lock
between tag-hours, so the pollers' inserts are never starved, and it works
through a large backlog in batches. A database from before packing existed is
migrated by this worker; nothing else has to be done.

### Block format

Each block holds up to 4 Mi samples of one tag (an hour at up to 1 kHz) and is
self describing (`src/compression/block.rs`):

* **Timestamps**: delta-of-delta (Gorilla). Steady polling costs a fraction of a
  bit per sample, because a run of 23 or more identical intervals is one 22 bit
  token. Any `i64` sequence round-trips, including duplicates and out of order.
* **Analog values**: XOR of consecutive IEEE-754 values, storing only the
  changed bits (Gorilla). NaN, infinities, -0.0 and subnormals keep their bits.
* **Digital values**: a tag that only takes 0.0 and 1.0 in the hour is stored as
  run lengths (run-length encoding), one byte for most runs.
* **Constant values**: a tag that does not change in the hour stores its value
  once.

### Size

Measured with the real insert and compaction code on one hour of 1 000 synthetic
tags (see `src/storage/tests/historian_bench.rs`; run it with
`cargo test --lib storage::tests::historian_bench -- --ignored --nocapture`).
The mix: 450 slow process values (250 as scaled 16 bit registers, 200 as float32),
150 setpoints, 200 digitals, 50 pulse counters, 50 totalisers, 100 noisy float32
signals (flow, vibration). Sizes are database file sizes after `VACUUM`.

| Scenario | Samples per tag-hour | Raw rows | Packed payload | Packed in DB file |
|---|---|---|---|---|
| Every sample, 1 s polling | 3 600 | 64.2 bytes/sample | 2.31 bytes/sample | 2.53 bytes/sample |
| Deadband 0.5 % of span, 300 s heartbeat | 444 | 65.3 | 3.15 | 3.83 |
| Deadband tuned to the signals | 25 | 60.5 | 4.01 | 7.35 |

Extrapolated to 10 000 tags for one year of full resolution (packed, in the
database file):

| Scenario | Samples per year | Size |
|---|---|---|
| Every sample, 1 s polling | 3.2e11 | about 740 GiB |
| Deadband 0.5 % of span, 300 s heartbeat | 3.9e10 | about 140 GiB |
| Deadband tuned to the signals | 2.1e9 | about 15 GiB |

Reading these numbers:

* Compression of the stored samples is about 25 to 28 times against plain rows
  at 1 s, less for sparse data. How well a signal compresses depends on the
  signal: digitals and setpoints cost almost nothing, scaled registers and
  float32 values with noise cost 2.4 to 5 bytes, totalisers 5.5 bytes, and a
  signal that is mostly noise is close to incompressible.
* Deadband is what makes the year-scale figures small, not the block format.
  Unfiltered 1 s data cannot get near 15 GB for 10 000 tags: that would be
  0.4 bits per sample.
* Sparse series pay a fixed cost per block (key, header, page overhead, about
  80 bytes), which is why bytes per sample rise as samples per hour fall.
* The hourly tier costs about 110 bytes per tag-hour (measured with its two
  indexes), roughly 9 GiB per year for 10 000 tags if a full year of it is kept;
  the daily tier is about 0.4 GiB per year.
* The tag mix is an assumption. Measure your own plant: `voltrus db stats`
  prints the packed bytes per sample of the running database.

## Retention

```yaml
retention:
  enabled: true
  run_interval_secs: 21600     # roll expired tiers up every 6 hours
  pack_enabled: true
  pack_after_secs: 3600        # complete hours older than this are packed
  pack_interval_secs: 600      # how often the packer runs
  archive_dir: ./archive
  tiers:
    - name: raw                # full resolution: recent rows plus packed blocks
      keep: 7776000            # 90 days (seconds); null keeps it forever
      downsample_to: hourly
      archive: false           # true: export what expires to CSV first
    - name: hourly
      keep: 7776000            # 90 days past the raw window
      downsample_to: daily
    - name: daily
      keep: null               # past the hourly window; null is forever
```

Every key is optional and the keys that existed before keep their meaning.
The defaults above apply when `retention` is missing.

Once per `run_interval_secs`:

* full-resolution hours older than the `raw` window are written as hourly
  aggregates and removed (blocks of the same hour go together, one transaction
  per tag-hour). With `archive: true` they are first appended to
  `<archive_dir>/<YYYY-MM-DD>.csv.gz` (columns `device_id,metric,timestamp,value`);
  if the archive cannot be written the hour stays where it is and the next cycle
  retries. A file is a sequence of gzip members, one per hour, which `zcat` and
  any gzip library read as a single stream. Archives are opt-in because they are
  large: they hold every sample that leaves full resolution as text,
* hourly rows older than the `hourly` window are folded into daily rows (the
  mean weighs each hour by its sample count) and removed,
* daily rows older than the `daily` window, if one is set, are removed.

The same worker also trims the event and audit tables (cleared alarms,
downtime, security log, run history, refresh tokens) and returns freed database
pages to the filesystem. Those keys, their defaults, the compliance rules for
audit trails and `voltrus db vacuum` are in [retention.md](retention.md).

Packing can be switched off with `pack_enabled: false`; samples then stay plain
rows until they expire. Expiry always goes through blocks, because the hourly
aggregate of an hour is computed from its block.

## Operations

* `GET /api/v1/compression/stats`: row counts per tier, packed blocks, samples,
  payload bytes and bytes per sample, database size.
* `GET /api/v1/compression/config`: the running retention and packing settings,
  including the event and audit keeps in force (`null` is forever).
* `voltrus db vacuum`: rebuild the database file to reclaim free space (server
  stopped); see [retention.md](retention.md).
* `GET /api/v1/compression/deadband`: tags that filter their history writes.
* `voltrus db stats [CONFIG]`: the same statistics from the command line.
* `DELETE /api/v1/telemetry` removes samples from rows and blocks alike;
  blocks that straddle the range are rewritten without the deleted samples.
* Backup and restore copy the database file, blocks included.

## Limits

* Blocks are per tag and hour. A trend over a long range reads the blocks it
  covers one tag at a time and releases the database between tags. Each block
  stores its lowest, highest, sum and newest value, so a bucket that holds
  whole blocks (the 30 day trend uses 4 h buckets) is answered without
  decoding them; narrower buckets decode the blocks they cut through. Blocks
  packed by an older version have no summary and are always decoded.

## Reading history without losing the end of the window

No reader cuts a window off: an answer always runs from the start of the window
to its newest sample, and when the window holds more samples than the answer
carries, it is thinned evenly over its whole length.

| Reader | What it returns |
|---|---|
| `GET /api/v1/history` (trends, MCP `read_history`), `agg=sampling` (default) | every sample up to 2 000 per tag, otherwise the newest sample of each of 2 000 equal slices of the window; such a series carries `"downsampled": true` |
| the same with `detail=full` | the same rule with 10 000 points per tag |
| the same with `agg=avg` or `agg=minmax` | one bucket per `interval` of the range (10 s for 30 min, 8 min for 24 h, ...), widened if that would exceed 10 000 points; `minmax` adds the lowest and highest sample of each bucket |
| tags defined by an `expression` | the expression evaluated on the bucketed values of its inputs (their means, newest samples, or lows and highs by `agg`), so the band of a non-monotonic expression is approximate |
| `GET /api/v1/analytics/spc`, `histogram`, `run-chart` | up to 10 000 samples; a longer window gives every k-th sample plus the last |
| `Storage::query_history`, `query_history_multi` (reports) | up to 10 000 / 5 000 samples per tag; a longer window keeps the lowest and highest sample of every stretch, so a minimum or maximum taken from the points is the real one |
| BI and Grafana queries, scheduled egress | every sample of the range, or an error naming the limit (1 000 000 samples per device for BI, 3 000 000 per file for egress); narrow the range or split the job |
| `GET /api/v1/data/export` | paged with `limit` and `offset` (newest first), counts included |

Ranges older than the full-resolution window are answered from the hourly and
daily rows (count-weighted when buckets are merged); each tier is used only for
the time before the next one starts, so no moment is counted twice.

* Samples are `f64` and timestamps whole seconds, as in the `telemetry` table.
  SQLite cannot store NaN in that table, so NaN never reaches a block through
  the normal write path.
