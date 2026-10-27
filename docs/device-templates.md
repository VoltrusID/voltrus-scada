# Device templates

A device template is a Modbus or BACnet device description you can reuse: the
connection defaults and the register map (Modbus) or object list (BACnet) of one
hardware model, plus notes about the hardware (firmware, word order, offset
quirks). Create a device from a template by giving it an address; save a device
you configured as a template for the next site; share templates through a public
catalog.

Templates come from three places:

| Source | `source` value | Editable | Where it comes from |
|---|---|:-:|---|
| Built-in | `builtin` | no | Generated at startup from the simulator device profiles (SMA STP 60, Schneider PM5560, E+H Promag 10W, ABB AZL402, Emerson Daniel 1000, Raritan PX3, Trane Tracer SC+, JCI FEC). Marked `verified: false`: the register maps come from the simulator, not from a field check. Never stored; cannot be deleted. |
| Local | `local` | delete | Saved from a device, or a template file you imported. |
| Catalog | `catalog:<index url>` | delete | Imported from a public catalog, see [Catalog](#catalog). |

## Using templates in the web UI

Open **Data > Modbus** or **Data > BACnet** and press **Templates**.

* **Library** lists the built-in, local and catalog templates for that protocol.
  Search by manufacturer or model, filter by source, expand a row to see the
  registers or objects, the quirks and the verification notes.
  * **Use** asks for a name and an address (IP, port, unit id or device
    instance, interval) and creates the device with all its registers or
    objects.
  * **Export** downloads the template as a JSON file.
  * **Del** removes a local or catalog template (devices already made from it are
    not touched).
  * **Import file** adds a template file you were given.
* **Catalog** lists the public catalog and imports a template into your library
  (admins only).
* **Tpl** on a device row saves that device as a template: manufacturer, model,
  version, a description, whether you verified it and how, and the quirks.

## File format

A template is a JSON file. It extends the device JSON that Modbus Poll,
BACnet Explorer and `POST /api/v1/modbus-devices` already use, so a register or an
object has the same field names as the device routes. The schema is versioned
by the `format` field: `voltrus-device-template/1`. A server refuses a format it
does not know.

```json
{
  "format": "voltrus-device-template/1",
  "template_id": "acme-pm9000",
  "manufacturer": "Acme Power",
  "model": "PM9000",
  "protocol": "modbus",
  "description": "3-phase power meter, Modbus TCP gateway on the front port",
  "version": "1.0.0",
  "verified": true,
  "verified_notes": "Firmware 2.1.4, checked against the front panel at a 400 V site, Jan 2026",
  "quirks": [
    "Float32 values are low word first: use endianness 'swapped'",
    "The manual lists 1-based register numbers; offsets here are 0-based"
  ],
  "license": "CC-BY-4.0",
  "port": 502,
  "unit_id": 1,
  "interval_ms": 1000,
  "endianness": "swapped",
  "registers": [
    { "name": "Voltage_L1N", "offset": 0, "register_type": "float32", "register_area": "holding", "scale": 1.0, "unit": "V", "description": "L1-N voltage" },
    { "name": "Current_L1", "offset": 2, "register_type": "float32", "register_area": "holding", "scale": 1.0, "unit": "A" },
    { "name": "Power_Total", "offset": 4, "register_type": "float32", "register_area": "holding", "scale": 0.001, "unit": "kW", "description": "Raw value is in W" },
    { "name": "Breaker_Open", "offset": 100, "register_type": "uint16", "register_area": "coil", "writable": true }
  ]
}
```

### Metadata

| Field | Required | Notes |
|---|:-:|---|
| `format` | yes | `voltrus-device-template/1`. |
| `template_id` | yes | A slug: lowercase letters, digits and single `-`, at most 64 characters. `catalog` and `from-device` are reserved. |
| `manufacturer`, `model` | yes | At most 120 characters each. |
| `protocol` | yes | `modbus` or `bacnet`. |
| `version` | yes | Semantic version, `MAJOR.MINOR.PATCH` with an optional `-prerelease`. `template_id` plus `version` identifies a file; raise the version when the register map changes. |
| `description` | no | Free text. |
| `verified` | no | `true` when the map was checked against a real device. Default `false`. |
| `verified_notes` | no | What was verified: firmware version, site, what was compared. A `verified: true` template without notes is not very useful to the next person. |
| `quirks` | no | List of free-text notes: word order, offset base, scaling oddities. |
| `license` | no | Default `CC-BY-4.0`. |
| `source` | no | `builtin`, `local` or `catalog:<url>`. Set by the server when a template is imported or saved; whatever the file says is ignored. |

### Modbus templates

Device defaults (all optional; defaults in brackets): `port` (502), `unit_id` (1),
`interval_ms` (1000), `endianness` (`big`, `little` or `swapped`; `big`).
At least one entry in `registers`.

| Register field | Required | Notes |
|---|:-:|---|
| `name` | yes | Unique within the template, at most 128 characters. |
| `offset` | yes | 0-based offset within `register_area`, 0-65535, as on the wire. Always an offset: the 5-digit references (40001) of the device file are not used in templates. |
| `register_type` | no | `int16`, `uint16`, `int32`, `float32` (default) or `float64`. |
| `register_area` | no | `holding` (default), `input`, `coil` or `discrete`. |
| `scale` | no | Multiplier, default 1. |
| `unit` | no | Engineering unit, informational. |
| `description` | no | Informational. |
| `writable` | no | The register accepts writes on the device. Informational. |

`unit`, `description` and `writable` are documentation: the device itself keeps
only name, offset, type, area and scale, so they are not copied onto a device
made from the template and a template saved from a device does not have them.

A value that does not fit before offset 65535 (a `float32` at 65535) is
refused.

### BACnet templates

Device defaults: `port` (47808), `interval_ms` (1000), `cov_enabled` (false),
`cov_lifetime` (3600). At least one entry in `objects`.

```json
{
  "format": "voltrus-device-template/1",
  "template_id": "acme-ahu-ctl",
  "manufacturer": "Acme Controls",
  "model": "AHU-CTL",
  "protocol": "bacnet",
  "version": "1.0.0",
  "quirks": ["Supply fan status is binaryInput 3, not 1 as in the data sheet"],
  "port": 47808,
  "cov_enabled": true,
  "objects": [
    { "object_type": "analogInput", "object_instance": 1, "name": "SupplyAirTemp", "unit": "degC" },
    { "object_type": "binaryOutput", "object_instance": 3, "name": "SupplyFan", "writable": true }
  ]
}
```

| Object field | Required | Notes |
|---|:-:|---|
| `object_type` | yes | `analogInput`, `analogOutput`, `analogValue`, `binaryInput`, `binaryOutput`, `binaryValue`, `multiStateInput`, `multiStateOutput` or `multiStateValue`. |
| `object_instance` | yes | 0-4194303. |
| `name` | yes | At most 128 characters. |
| `property` | no | Default `presentValue`. |
| `display_name`, `cov_increment` | no | As on `POST /api/v1/bacnet/devices/{id}/objects`. |
| `unit`, `description`, `writable` | no | Informational, as for Modbus. |

The device instance is not part of a template: it belongs to the device, you
give it when you create the device.

## REST API

All under `/api/v1/device-templates`. Any signed-in user can read. Writes follow
the device routes: admin, demo and integration-key roles; BACnet devices are
admin-only as on the BACnet routes. A catalog import is admin-only.

| Route | Purpose |
|---|---|
| `GET /device-templates?q=&manufacturer=&protocol=&source=` | List every version of every template. `q` is a case-insensitive substring of id, manufacturer, model or description; `source` is `builtin`, `local`, `catalog` or a full source. |
| `GET /device-templates/{id}?version=&source=` | The template and `read_only`. Without `version` the newest; with equal versions, local before catalog before built-in. |
| `GET /device-templates/{id}/export?version=&source=` | The template as a JSON download. |
| `POST /device-templates` | Import a template file (body = the file). Validated; stored as `local`. 400 with every problem found, 409 if that id and version is already stored as local. |
| `DELETE /device-templates/{id}?version=&source=` | Delete a local or catalog template. Built-in: 403. |
| `POST /device-templates/from-device/{device_id}` | Save a device as a local template. Body: `manufacturer`, `model` (required), `template_id`, `version` (default `1.0.0`), `description`, `verified`, `verified_notes`, `quirks`, `license`, `protocol` (`modbus` or `bacnet`; tried in that order when left out). |
| `POST /device-templates/{id}/instantiate` | Create the device. Body: `name`, `ip` (required), `port`, `interval_ms`, Modbus `id` (default: the name as a slug), `unit_id`, `endianness`; BACnet `device_instance`, `cov_enabled`, `cov_lifetime`, `enabled`; `version`, `source` to pick one when the id is ambiguous. Uses the normal device create path (asset inventory, poller start, audit log). 201 with the new device's `id`. |
| `GET /device-templates/catalog?url=` | Fetch the catalog index; each entry says whether it is already imported. `url` (admin only) replaces the configured catalog. |
| `POST /device-templates/catalog/import` | Body `{ "template_id", "version", "url" }`; `version` and `url` optional. Admin only. |

The OpenAPI document (`/api/v1/openapi.json`, with the `openapi` feature) lists
the same routes under the *Device Templates* tag.

## Catalog

A catalog is a static folder served over HTTP(S): an `index.json` and the
template files it lists. The default is a public git repository,

```
https://raw.githubusercontent.com/VoltrusID/voltrus-device-templates/main/index.json
```

so publishing a catalog is pushing files to a repository; there is no service
to run. Templates in the public catalog are licensed CC-BY-4.0.

```json
{
  "format": "voltrus-template-catalog/1",
  "templates": [
    {
      "template_id": "schneider-electric-pm5560",
      "manufacturer": "Schneider Electric",
      "model": "PM5560",
      "protocol": "modbus",
      "version": "1.0.0",
      "verified": false,
      "path": "schneider-electric-pm5560.json",
      "sha256": "9509dc62d1395c0bc5e0d9942b680a4810c798d3a76c33bee0c4a94bc5b7dd67"
    }
  ]
}
```

`path` is relative to the index URL (no `..`, no scheme, no leading `/`).
`sha256` is the lowercase hex SHA-256 of the file's bytes.

An import downloads the index, finds the entry (the newest version when none is
given), downloads the file and checks, in this order: at most 1 MiB, the
SHA-256 equals the index's, the file is a valid template, and its
`template_id`, `version` and `protocol` are the ones the index lists. A failed
check stores nothing and answers 422 (a catalog that cannot be reached or
answers an error status: 502). Requests time out after 5 s to connect and 20 s
in total; an index may be up to 4 MiB.

The SHA-256 protects against a file that no longer matches its index (a
corrupted or half-updated repository); it does not authenticate the catalog:
whoever controls the index controls the hashes. Import from catalogs you trust,
and read the template: it only declares registers, but a wrong scale or offset
gives wrong readings.

An imported template is stored with `source: catalog:<index url>` and is
independent of the catalog afterwards.

### Choosing the catalog

The index URL is the `device_templates.catalog_url` setting. In the config file:

```yaml
device_templates:
  catalog_url: "https://example.com/our-templates/index.json"
```

The value is written to the setting at every start. An admin can try another
catalog without changing the setting by typing its URL on the Catalog tab
(`?url=` / `"url"` in the API).

### Publishing a catalog

`docs/device-templates-catalog-seed/` holds a ready-to-publish catalog with a
template file per built-in template. To start the public repository, copy that
folder's contents to the repository root and push.

To add or change templates, put the template files in the folder (any depth,
`*.json`) and rebuild the index; it lists every template file with its hash:

```bash
voltrus template catalog path/to/catalog     # writes path/to/catalog/index.json
```

`voltrus template builtins <dir>` writes the built-in templates as files, which
is how the seed folder was made. Rebuild the index after any change to a file:
the hash is of the file's bytes.

Contributions to the public catalog are template files saved with **Tpl** (or
written by hand) and checked on a real device; put what you checked in
`verified_notes`.
