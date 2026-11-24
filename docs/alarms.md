# Alarms: chatter and on-delay suggestions

A tag's alarm limits (high warning and critical, `low_warn` and `low_crit` in
`source_config`), its alarm `deadband` and its ISA-18.2 `on_delay_secs` and
`off_delay_secs` are set in the tag editor (**Edit** on a tag, section
"ISA-18.2 alarm delays"), over `PUT /api/v1/data-sources/{id}`, or in a project
file (`tags[].alarm`, see [project-format.md](project-format.md)). This page
covers what the server does against alarms that chatter.

## Why

An analog value hovering at a limit, with noise larger than the deadband, raises
and clears its alarm again and again. In our comparison lab a turbidity signal
with ±0.3 NTU noise and a 0.5 NTU deadband raised its alarm 5 times in 9 seconds
on a SCADA system without an on-delay. Voltrus has had on and off delays; it now
also proposes one and counts how often each alarm is raised.

## Chatter count

`GET /api/v1/alarms/analytics?from=&to=` returns, per tag, `trigger_count`,
`max_per_hour` (the most alarms raised in one clock hour within the range) and
`chattering` (`max_per_hour` is 5 or more). The Alarms panel's **Analytics** tab
(last 24 hours) marks such tags with `CHATTER n/h` and a hint to raise the
delay or deadband.

## Suggested on-delay

`GET /api/v1/data-sources/{id}/alarm-suggestion` looks at the tag's history of
the last 30 minutes and its alarms of the last 24 hours and answers:

```json
{"on_delay_secs": 5, "deadband": 1.3,
 "reason": "The value moves by about 0.42 between samples, more than a third of the deadband (0.50).",
 "analog": true, "noise": 0.42, "samples": 360,
 "alarms_last_hour": 2, "max_alarms_per_hour": 4, "chattering": false}
```

| Case | `on_delay_secs` |
|---|---|
| Digital tag (range 0..1 without decimals, or only 0 and 1 seen) | 0 |
| Alarm raised 5 or more times in one hour in the last day | 10 |
| Analog, noise more than a third of the alarm deadband | 5 |
| Any other analog tag, also without history | 2 |

Noise is the standard deviation of the differences between successive samples
divided by √2, so a slow ramp does not count as noise. With noise measured,
`deadband` is three times the noise, rounded up to the tag's decimals.

The tag editor shows the suggestion under the delay fields with **Use** buttons
for the delay and the deadband. Neither is filled in for you: opening the editor
and saving the tag keeps its stored delay until you press **Use**. The server
never changes a delay on its own.

## Limits

* Alarms are evaluated in the persistence loop (`persist_interval_secs`,
  default 5 s), so an on-delay is effectively rounded up to whole loop periods:
  a 2 s delay means the condition must still hold on the next pass.
* The noise estimate needs at least 10 samples in the last 30 minutes; the
  historian's per-tag deadband thins history and can hide noise.
* Chatter is counted from raised alarms per clock hour, not per sliding minute.
* Alarms on digital tags (state alarms) are not part of the core yet.
