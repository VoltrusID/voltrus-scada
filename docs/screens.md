# Values on screens

How the web client shows live values on dashboards (grid and canvas) and on 2D
SCADA screens: state colours, the trend popup and tag-swap faceplates.

## State colours

Every value bound to a tag shows its alarm state and its data quality, the same
way on every surface and with no setting per widget. One style map in the client
(`client/src/valueStyle.ts`) decides it:

| State | When | Colour (theme variable) | Marker | Label |
|---|---|---|:-:|---|
| Critical | the value is past a critical limit (high or low) | `--crit` | `!` | CRITICAL |
| Warning | the value is past a warning limit | `--warn` | `!` | WARNING |
| Bad | quality `bad`, `bad_comm_failure`, `bad_device_error`, `bad_config_error` | `--hmi-stale` | `✕` | BAD |
| Uncertain | quality `uncertain`, `uncertain_stale` | `--hmi-stale` | `?` | UNCERTAIN |
| Offline | the device is unreachable and there is no value | `--text-muted` | | OFFLINE |
| Normal | none of the above | the widget's own colour | | |

Quality wins over the alarm level, because the server evaluates alarms on good
values only (a stale value past a limit is shown as uncertain, not as an alarm).
The live stream spells the status `crit`/`warn` and the REST snapshot
`critical`/`warning`; both map to the same state.

Where it shows:

* **Dashboard cards:** the value text, the status dot and badge, and the card
  border take the state colour; a bad or uncertain value gets its marker.
* **2D SCADA screens:** at runtime every bound widget except areas, labels and
  pipes gets a frame in the state colour (dashed for uncertain and offline) and a
  round marker badge at its top-right corner, with the state in the tooltip. The
  big number, indicator and data table widgets also colour the value and show the
  label. Equipment symbols keep their own run/stop/fault colours inside the frame.
* **Faceplates** (below) colour analog values the same way.

Colour is never the only cue: each state has a marker and a label for screen
readers. Both themes define every colour, and a client test checks that each
reaches 4.5:1 contrast on the background and on cards (`tests/contrast.test.ts`).

Limits: add-on widgets drawn by an edition (not the core widgets) decide their own
look; the 3D twin does not use the map.
