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

### Modbus: `float64` in `little` and `swapped` order

`little` and `swapped` used to apply only inside each half of a 64-bit
value: the high 32 bits always came first. They now reverse the whole value,
as the 32-bit orders do (`little`: `GH EF CD AB`, `swapped`: `HG FE DC BA`;
see docs/device-templates.md). 32-bit values and `big` are unchanged.

A device with `float64` registers whose readings were correct with `little`
or `swapped` before the upgrade keeps the old layout under the new names
`little_dword` and `swapped_dword`: change the device's `endianness` (device
API, project YAML or template). Readings of such a device that suddenly look
like tiny or huge numbers after the upgrade are the sign it needs this.

### Flow schedules: day fields written with `*`

Schedules now read the two day fields as Vixie cron does: a field that
starts with `*` counts as unrestricted, and then a day must match both
fields. Up to v0.51 a starred step such as `*/2` counted as restricted, so a
day matching either field fired.

Only schedules with a `*`-prefixed step in one day field and a restricted
other day field change, for example `0 0 */2 * mon`: it used to fire on
every odd day and every Monday, and now fires on Mondays that fall on an odd
day. To keep the old behaviour, write the step without the star:
`0 0 1-31/2 * mon`. Plain `*` and schedules with only one day field
restricted behave as before.

Schedules with a fixed minute and hour also handle daylight saving time
differently: they fire once when the clock goes back, and at the change
when the clock skips their time (docs/flows.md).
