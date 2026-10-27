//! Device template tools: find known hardware, create a device from it, save a
//! configured device as a template, and the public catalog. They call the
//! handlers of the `/api/v1/device-templates` routes, so validation, role rules
//! and audit entries are the REST ones.

use axum::extract::{Extension, Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolFuture, from_handler, parse_args};
use crate::api::device_templates::{
    CatalogImportRequest, CatalogQuery, FromDeviceRequest, InstantiateRequest, RefQuery,
};
use crate::device_templates::TemplateFilter;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_device_templates",
            title: "List device templates",
            description: "Search the device template library: ready-made register maps (Modbus) \
                and object lists (BACnet) for known hardware such as power meters and \
                inverters. Every filter is optional and they combine. Each row carries \
                template_id, version, manufacturer, model, protocol, source (builtin, local or \
                catalog:<url>), verified and item_count; templates are not tags or devices \
                until you call instantiate_device_template. get_device_template shows the \
                full register map.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "q": {"type": "string", "description": "Case-insensitive text in the template id, manufacturer, model or description."},
                        "manufacturer": {"type": "string", "description": "Manufacturer, case-insensitive."},
                        "protocol": {"type": "string", "enum": ["modbus", "bacnet"]},
                        "source": {"type": "string", "description": "builtin, local, catalog (any catalog) or a full source such as catalog:<url>."}
                    },
                    "additionalProperties": false
                })
            },
            call: list,
        },
        Tool {
            name: "get_device_template",
            title: "Get device template",
            description: "One device template in full: defaults (port, unit id, interval, \
                endianness) and every register or object with type, area, scale, unit and \
                description, plus verified status and quirks (word order, offset base). Without \
                a version the newest is returned.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "template_id": {"type": "string", "minLength": 1, "description": "Template id from list_device_templates."},
                        "version": {"type": "string", "description": "Version, e.g. 1.0.0. Default: the newest."},
                        "source": {"type": "string", "description": "Source, when the same id comes from several (builtin, local, catalog:<url>)."}
                    },
                    "required": ["template_id"],
                    "additionalProperties": false
                })
            },
            call: get,
        },
        Tool {
            name: "instantiate_device_template",
            title: "Create device from template",
            description: "Create a device with the register map (Modbus) or object list \
                (BACnet) of a template and start polling it. Give the device a `name` and the \
                `ip` it is reachable at; port, unit id, interval and endianness default to the \
                template's and can be overridden. A Modbus device id defaults to the name as a \
                slug and fails if it exists. Registers are not tags: follow up with create_tag \
                (source_type \"modbus\", device_id = the returned id, key = register name). \
                BACnet devices can only be created by an admin login, not an integration key.",
            access: Access::Write,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "template_id": {"type": "string", "minLength": 1, "description": "Template id from list_device_templates."},
                        "name": {"type": "string", "minLength": 1, "maxLength": 128, "description": "Display name of the new device."},
                        "ip": {"type": "string", "minLength": 1, "description": "Host or IP address of the device."},
                        "id": {"type": "string", "description": "Modbus device id. Default: the name as a slug. BACnet devices get a numeric id."},
                        "port": {"type": "integer", "minimum": 1, "maximum": 65535, "description": "Default: the template's."},
                        "unit_id": {"type": "integer", "minimum": 0, "maximum": 255, "description": "Modbus unit id. Default: the template's."},
                        "interval_ms": {"type": "integer", "minimum": 1, "description": "Polling interval. Default: the template's."},
                        "endianness": {"type": "string", "enum": ["big", "little", "swapped"], "description": "Modbus word order. Default: the template's."},
                        "device_instance": {"type": "integer", "minimum": 0, "maximum": 4194303, "description": "BACnet device instance. Default 0."},
                        "cov_enabled": {"type": "boolean", "description": "BACnet change-of-value subscriptions. Default: the template's."},
                        "cov_lifetime": {"type": "integer", "minimum": 1, "description": "BACnet COV subscription lifetime in seconds. Default: the template's."},
                        "enabled": {"type": "boolean", "description": "BACnet only. Default true."},
                        "version": {"type": "string", "description": "Template version. Default: the newest."},
                        "source": {"type": "string", "description": "Template source, when the id comes from several."}
                    },
                    "required": ["template_id", "name", "ip"],
                    "additionalProperties": false
                })
            },
            call: instantiate,
        },
        Tool {
            name: "save_device_as_template",
            title: "Save device as template",
            description: "Save an existing device, with its registers or objects, as a local \
                device template so it can be reused for the same hardware. `device_id` is a \
                Modbus device id (or a BACnet device id; BACnet needs an admin login). \
                manufacturer and model are required; template_id defaults to their slug and \
                version to 1.0.0. Fails when that template id and version already exist locally.",
            access: Access::Write,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "device_id": {"type": "string", "minLength": 1, "description": "Device id from list_devices."},
                        "manufacturer": {"type": "string", "minLength": 1, "maxLength": 120},
                        "model": {"type": "string", "minLength": 1, "maxLength": 120},
                        "template_id": {"type": "string", "description": "Slug. Default: manufacturer and model."},
                        "version": {"type": "string", "description": "MAJOR.MINOR.PATCH. Default 1.0.0."},
                        "description": {"type": "string"},
                        "verified": {"type": "boolean", "description": "Checked against a real deployment. Default false."},
                        "verified_notes": {"type": "string", "description": "Firmware, site, what was checked."},
                        "quirks": {"type": "array", "items": {"type": "string"}, "description": "Word order, offset base, scaling oddities."},
                        "license": {"type": "string", "description": "Default CC-BY-4.0."},
                        "protocol": {"type": "string", "enum": ["modbus", "bacnet"], "description": "Only to disambiguate a Modbus and a BACnet device with the same id."}
                    },
                    "required": ["device_id", "manufacturer", "model"],
                    "additionalProperties": false
                })
            },
            call: save_device,
        },
        Tool {
            name: "list_template_catalog",
            title: "List template catalog",
            description: "Fetch the index of the public device template catalog the server is \
                configured to use (the catalog URL cannot be chosen here) and list its \
                templates, each marked `installed` when it was already imported. This makes an \
                outbound request. Import one with import_catalog_template (admin login only).",
            access: Access::Read,
            destructive: false,
            input_schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
            call: catalog,
        },
        Tool {
            name: "import_catalog_template",
            title: "Import catalog template",
            description: "Download a template from the configured public catalog, verify its \
                sha256 against the index and store it as a catalog template. Limited to a \
                signed-in admin: API and integration keys are refused. Fails when that \
                template version is already imported.",
            access: Access::AdminWrite,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "template_id": {"type": "string", "minLength": 1, "description": "Template id from list_template_catalog."},
                        "version": {"type": "string", "description": "Default: the newest version in the catalog."}
                    },
                    "required": ["template_id"],
                    "additionalProperties": false
                })
            },
            call: import_catalog,
        },
    ]
}

fn list(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            q: Option<String>,
            manufacturer: Option<String>,
            protocol: Option<String>,
            source: Option<String>,
        }
        let a: Args = parse_args(args)?;
        let filter = TemplateFilter {
            q: a.q,
            manufacturer: a.manufacturer,
            protocol: a.protocol,
            source: a.source,
        };
        from_handler(
            crate::api::device_templates::list_templates(State(ctx.state.clone()), Query(filter))
                .await,
        )
        .await
    })
}

fn get(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            template_id: String,
            version: Option<String>,
            source: Option<String>,
        }
        let a: Args = parse_args(args)?;
        from_handler(
            crate::api::device_templates::get_template(
                State(ctx.state.clone()),
                Path(a.template_id),
                Query(RefQuery {
                    version: a.version,
                    source: a.source,
                }),
            )
            .await,
        )
        .await
    })
}

fn instantiate(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            template_id: String,
            name: String,
            ip: String,
            id: Option<String>,
            port: Option<u16>,
            unit_id: Option<u8>,
            device_instance: Option<u32>,
            interval_ms: Option<u64>,
            endianness: Option<String>,
            cov_enabled: Option<bool>,
            cov_lifetime: Option<u32>,
            enabled: Option<bool>,
            version: Option<String>,
            source: Option<String>,
        }
        let a: Args = parse_args(args)?;
        let req = InstantiateRequest {
            name: a.name,
            ip: a.ip,
            id: a.id,
            port: a.port,
            unit_id: a.unit_id,
            device_instance: a.device_instance,
            interval_ms: a.interval_ms,
            endianness: a.endianness,
            cov_enabled: a.cov_enabled,
            cov_lifetime: a.cov_lifetime,
            enabled: a.enabled,
            version: a.version,
            source: a.source,
        };
        from_handler(
            crate::api::device_templates::instantiate_template(
                State(ctx.state.clone()),
                Extension(ctx.user.clone()),
                Path(a.template_id),
                axum::Json(req),
            )
            .await,
        )
        .await
    })
}

fn save_device(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            device_id: String,
            manufacturer: String,
            model: String,
            template_id: Option<String>,
            version: Option<String>,
            #[serde(default)]
            description: String,
            #[serde(default)]
            verified: bool,
            #[serde(default)]
            verified_notes: String,
            #[serde(default)]
            quirks: Vec<String>,
            license: Option<String>,
            protocol: Option<String>,
        }
        let a: Args = parse_args(args)?;
        let req = FromDeviceRequest {
            manufacturer: a.manufacturer,
            model: a.model,
            template_id: a.template_id,
            version: a.version,
            description: a.description,
            verified: a.verified,
            verified_notes: a.verified_notes,
            quirks: a.quirks,
            license: a.license,
            protocol: a.protocol,
        };
        from_handler(
            crate::api::device_templates::template_from_device(
                State(ctx.state.clone()),
                Extension(ctx.user.clone()),
                Path(a.device_id),
                axum::Json(req),
            )
            .await,
        )
        .await
    })
}

fn catalog(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {}
        let Args {} = parse_args(args)?;
        // `url: None` is the configured catalog; the tool never takes a URL, so
        // the model cannot point the server at an address of its choosing.
        from_handler(
            crate::api::device_templates::get_catalog(
                State(ctx.state.clone()),
                Extension(ctx.user.clone()),
                Query(CatalogQuery { url: None }),
            )
            .await,
        )
        .await
    })
}

fn import_catalog(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            template_id: String,
            version: Option<String>,
        }
        let a: Args = parse_args(args)?;
        from_handler(
            crate::api::device_templates::import_from_catalog(
                State(ctx.state.clone()),
                Extension(ctx.user.clone()),
                axum::Json(CatalogImportRequest {
                    template_id: a.template_id,
                    version: a.version,
                    url: None,
                }),
            )
            .await,
        )
        .await
    })
}
