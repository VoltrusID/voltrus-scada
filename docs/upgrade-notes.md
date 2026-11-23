# Upgrade notes

Changes in behaviour that can need action when upgrading. The repository has
no changelog file: release notes are generated from pull requests, so this
page is where such changes are written down. Newest release first.

## Next release (after v0.51.0)

### OEE: devices with several metrics need a state metric

The OEE monitor used to decide whether a machine runs from whichever metric
the cache happened to list first when no state metric was configured. It now
tracks such a device only if the device has a single metric. A device with
several metrics and no `metric` in its `oee_threshold_<device id>` setting
gets no downtime events until one is chosen:

```json
{ "metric": "spindle_speed", "running_above": 0 }
```

The server logs a warning once per device, and `GET /api/v1/oee/metrics`
answers with `"tracking": "not tracked: choose a state metric ..."` for it.
Readings that are not good (bad quality, stale, NaN) no longer change the
state either.
