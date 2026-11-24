# Upgrade notes

Changes in behaviour that can need action when upgrading. The repository has
no changelog file: release notes are generated from pull requests, so this
page is where such changes are written down. Newest release first.

## Next release (after v0.51.0)

### Sign-in lockout is per address, not per account

Ten wrong passwords for an account used to lock the account for everyone for
15 minutes, so anyone who could reach the sign-in page could keep a user
(an admin, the operators of a plant) locked out by sending a few wrong
passwords every 15 minutes. Failed sign-ins are now counted in a new table,
`login_failures`, against the client address the server resolves (the
`trusted_proxy` rules: a proxy header is read only from the proxy's own
address):

* 10 wrong passwords for one account from one address within 15 minutes
  lock that account **for that address** for 15 minutes (403, as before).
  The user still signs in from any other address.
* 30 wrong passwords (or unknown usernames) from one address across all
  accounts within 15 minutes lock that address out of sign-in for 15 minutes
  (429), against password spraying. Failures from this machine (127.0.0.1,
  ::1) don't count here: the native console and a local reverse proxy
  connect from it, and a proxy that does not pass the client address on
  would otherwise make one typo-prone shift lock everyone out.
* 50 wrong passwords for one account from all addresses within 15 minutes
  make sign-ins to it from addresses the user has not signed in from in the
  last 30 days wait a minute (429), renewed by every further failure in
  those 15 minutes. Addresses the user did sign in from are not slowed down.
  The trade-off: during a sustained attack from many addresses, the user
  cannot sign in from a new device until the attack stops or an admin
  resets the password.
* A successful sign-in clears its address's count. Changing the password,
  or an admin resetting it (Users, or `voltrus --reset-admin-password` for admin),
  clears every count and lock of the account.
* The in-memory limit of 5 tries a minute is now per account and address,
  no longer per account.
* Wrong electronic-signature passwords are counted for the account apart
  from sign-in: 10 lock signing for 15 minutes and end the user's sessions,
  but no longer lock sign-in.

Behind a reverse proxy, check that `trusted_proxy` matches it (the server
warns about some mismatches): if every client shows up as the proxy's
address, the per-address lock behaves like the old account-wide lock.

The upgrade creates the table and lifts any lock of the old kind. The
`failed_attempts` and `locked_until` columns of `users` are left in place
but no longer used.

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
