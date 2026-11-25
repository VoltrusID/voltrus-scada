# Upgrade notes

Changes in behaviour that can need action when upgrading. The repository has
no changelog file: release notes are generated from pull requests, so this
page is where such changes are written down. Newest release first.

## Unreleased

### New tags have no alarm limits; alarms clear when their limit goes

On a fresh database the first start used to create the configured devices'
tags with alarm limits nobody set: high warning 80 and high critical 90 for
any register (and fixed limits for registers named `voltage`, `current`,
`power`, `frequency`, `power_factor` and `energy_total`, including a high
power-factor alarm at 0.98). Tags from vertical templates got the range's
minimum and maximum as high limits, and a template's "below" alarm became a
high limit. A flow reading 1178 m³/h was then in critical alarm on its first
poll. Now:

* Tags created at first start, from vertical templates and from FEED
  templates have no high or low limits until someone sets them. A vertical
  template's own alarms still become limits on its tags: "above" a high
  limit, "below" a low limit (it used to be stored as a high limit), and
  "equal" a low limit when it is the bottom of the tag's range (safety
  contacts such as `SAFE_DOOR`, `SAFE_ROPE` and `NAV_<light>` alarm at 0;
  they used to get no alarm at all) or a high limit otherwise. A tag with
  several template alarms keeps them all, and the template's deadband is
  written to the tag.
* An active limit alarm clears on the next evaluation (persistence interval)
  when its condition no longer exists because the tag changed: its limit was
  removed or disabled, its limit moved so the current value is inside it
  (with the tag's deadband), or the tag was deleted or no longer reads a
  device. Before, such alarms stayed active forever, since evaluation only
  cleared an alarm once the value came back inside the limit it was raised
  on. The alarm's audit trail gets an `auto_resolved` entry whose details
  say why: `limit_removed`, `limit_changed`, `tag_deleted` or `not_alarmed`,
  with a message such as "Raw Water Intake Flow FT-101: high critical limit
  removed (was 90 m³/h)". A removed limit clears its alarm even while the
  tag has no good value; a changed one waits for a good value inside the new
  limit (with the deadband stored on the alarm) and, on a tag with an off
  delay, for that value to stay inside for the off delay.
* Renaming a tag's key moves its active limit alarms to the new key, so they
  are not cleared and raised again.
* Alarms raised by a flow's alarm node are stored with `alarm_type`
  `event`, not `high`, so limit evaluation never clears them (it used to
  clear them when the tag of the same key read below 0). An event alarm has
  no condition to return to normal, so **acknowledging it resolves it** (the
  audit trail gets a `resolved` entry by the operator, reason
  `event_acknowledged`). It no longer holds back limit alarms on the same
  key, and its acknowledgement cooldown applies to it only. The upgrade
  retypes flow alarms stored before (`high`, value, limit and deadband 0,
  and a message not written by limit evaluation) as `event`. API clients
  and the SDKs see `alarm_type: "event"`; value and threshold are 0.

**Existing tags keep their limits.** A limit seeded by an earlier version
cannot be told apart from the same limit set on purpose, so the upgrade
changes no stored limit. At start the server logs a warning for each kind of
unconfigured limit it still finds on tags never edited since (no
`*_enabled` flag in their settings):

* first-start tags with the seeded limits exactly as seeded (name, colour,
  range and limits unchanged);
* vertical and FEED template tags (orange) whose high warning limit is still
  the range minimum or whose high critical limit is still the range maximum;
* vertical template tags whose "below" alarm was stored as a high limit
  (the warning names them; move the value to the low limit).

Review those tags and set real limits or clear them; once cleared, any alarm
raised on them clears on the next evaluation. Template "equal 0" safety
alarms are not added to existing tags: set a low limit of 0 on them, or
create the template again.

## v0.52.0

### Sign-in lockout is per address, not per account

Ten wrong passwords for an account used to lock the account for everyone for
15 minutes, so anyone who could reach the sign-in page could keep a user
(an admin, the operators of a plant) locked out by sending a few wrong
passwords every 15 minutes. Failed sign-ins are now counted in a new table,
`login_failures`, against the client address the server resolves (the
`trusted_proxy` rules: a proxy header is read only from the proxy's own
address; IPv6 addresses count per /64 network):

* 10 wrong passwords for one account from one address within 15 minutes
  lock that account **for that address** for 15 minutes (403, as before).
  The user still signs in from any other address.
* 30 wrong passwords (or unknown usernames) from one address across all
  accounts within 15 minutes lock that address out of sign-in for 15 minutes
  (429), against password spraying. A user who signed in from that address
  in the last 30 days is not locked out by it (someone else behind the same
  NAT can't shut the plant out). Failures from this machine (127.0.0.1,
  ::1) don't count here, and the in-memory limit of 10 failures a minute
  per address doesn't apply to it: the native console and a local reverse
  proxy connect from it.
* 50 wrong passwords for one account from all addresses within 15 minutes
  make sign-ins to it from addresses the user has not signed in from in the
  last 30 days wait a minute (429), renewed by every further failure in
  those 15 minutes. Addresses the user did sign in from are not slowed down.
  The trade-off: during a sustained attack from many addresses, the user
  cannot sign in from a new device until the attack stops or an admin
  resets the password.
* A successful sign-in clears the count of that account and address (not
  the address's count across accounts). Changing the password, or an admin
  resetting it (Users, or `voltrus --reset-admin-password` for admin),
  clears every count and lock of the account.
* The in-memory limit of 5 tries a minute is now per account and address,
  no longer per account.
* Wrong passwords typed inside a session (electronic signatures, and now
  also the current password when changing it, which was not limited
  before) are counted apart from sign-in: 10 within 15 minutes lock these
  checks for 15 minutes and end the user's sessions, but no longer lock
  sign-in. They do count towards the account's 50.
* Attempts are counted before the password is checked, so requests sent in
  parallel cannot get past a limit. Windows are fixed, starting at the first
  failure, so up to twice a limit can get through around the end of one.

Behind a reverse proxy on another machine, list it in `trusted_proxy`: if
every client shows up as the proxy's address, all users share one address,
and 30 failures from anyone lock out of sign-in every user who has not
signed in through it in the last 30 days.

The upgrade creates the table and lifts any lock of the old kind; if it
cannot create the table the server does not start. The `failed_attempts`
and `locked_until` columns of `users` are left in place but no longer used.

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
