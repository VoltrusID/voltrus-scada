//! Declarative description of every entity kind the project format covers.
//!
//! A [`Field`] list drives parsing (types, defaults, unknown-field rejection),
//! canonical output (field order, omitted defaults) and the SQL column mapping
//! of the "plain" kinds, so one engine handles most sections. Tags, screens and
//! the settings singleton need translation between database ids and natural
//! keys and have dedicated code in `export.rs` / `import.rs`.

/// Scalar and structured value types a field can hold.
#[derive(Clone, Copy)]
pub enum Ty {
    Text,
    Int,
    Real,
    Bool,
    /// Arbitrary JSON-compatible YAML (stored as JSON text in SQLite).
    Json,
    /// Nested mapping with its own field list (not a SQL column).
    Obj(&'static [Field]),
    /// Sequence of mappings with a shared field list (child rows or nested lists).
    List(&'static [Field]),
}

/// Default of a field that may be omitted from a hand-written file.
#[derive(Clone, Copy)]
pub enum Dv {
    S(&'static str),
    I(i64),
    R(f64),
    B(bool),
    /// Empty sequence (`[]`).
    Arr,
    /// Empty mapping (`{}`).
    Map,
    /// Raw JSON text, for defaults that are neither empty nor scalar.
    J(&'static str),
}

#[derive(Clone, Copy)]
pub enum Req {
    Required,
    /// May be absent; absent means SQL NULL.
    Optional,
    Default(Dv),
}

#[derive(Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    /// SQL column. Empty for fields that are not stored in a column of the
    /// section's own table.
    pub col: &'static str,
    pub ty: Ty,
    pub req: Req,
    /// Omit from output when equal to the default.
    pub lean: bool,
}

impl Field {
    pub const fn r(name: &'static str, ty: Ty) -> Self {
        Field {
            name,
            col: name,
            ty,
            req: Req::Required,
            lean: false,
        }
    }
    pub const fn o(name: &'static str, ty: Ty) -> Self {
        Field {
            name,
            col: name,
            ty,
            req: Req::Optional,
            lean: false,
        }
    }
    pub const fn d(name: &'static str, ty: Ty, dv: Dv) -> Self {
        Field {
            name,
            col: name,
            ty,
            req: Req::Default(dv),
            lean: false,
        }
    }
    pub const fn col(mut self, col: &'static str) -> Self {
        self.col = col;
        self
    }
    pub const fn lean(mut self) -> Self {
        self.lean = true;
        self
    }
    /// Field with no column of its own (nested data, references, input-only).
    pub const fn virt(mut self) -> Self {
        self.col = "";
        self
    }
    pub fn is_column(&self) -> bool {
        !self.col.is_empty() && !matches!(self.ty, Ty::Obj(_) | Ty::List(_))
    }
}

#[derive(Clone, Copy)]
pub enum Layout {
    /// One file per entity under a directory (or per group for tags).
    Dir,
    /// One file holding the whole sequence.
    File,
    /// One file holding a single mapping.
    Single,
}

#[derive(Clone, Copy)]
pub enum Pk {
    /// The `id` field is the TEXT primary key.
    Text,
    /// Hidden INTEGER AUTOINCREMENT `id`.
    Auto,
}

#[derive(Clone, Copy)]
pub enum Stamp {
    Secs,
    Iso,
}

pub struct Child {
    /// Name of the `Ty::List` field on the parent that carries these rows.
    pub yaml: &'static str,
    pub table: &'static str,
    /// Column holding the parent id.
    pub fk: &'static str,
    /// `ORDER BY` used when exporting; also the insertion order on import.
    pub order: &'static str,
    /// Natural key inside one parent (duplicates are rejected).
    pub key: &'static [&'static str],
    pub created: bool,
}

pub struct Plain {
    pub table: &'static str,
    pub pk: Pk,
    pub stamp: Stamp,
    pub created: Option<&'static str>,
    pub updated: Option<&'static str>,
    pub children: &'static [Child],
    pub hook: Option<Hook>,
}

#[derive(Clone, Copy)]
pub struct Hook {
    /// Translate database ids into natural keys after reading.
    pub export: fn(&mut serde_yaml_neo::Mapping, &mut super::export::Ctx),
    /// Translate natural keys back into database ids before writing.
    pub import: fn(&mut serde_yaml_neo::Mapping, &super::export::Ctx) -> anyhow::Result<()>,
}

pub enum Kind {
    Plain(&'static Plain),
    Tags,
    Screens,
    Settings,
}

pub struct Section {
    /// Top-level key in the single-file form.
    pub name: &'static str,
    pub label: &'static str,
    /// Directory (or file stem) in the directory form.
    pub path: &'static str,
    pub layout: Layout,
    pub fields: &'static [Field],
    /// Fields whose joined values identify an entity.
    pub key: &'static [&'static str],
    /// Duplicate keys are an error (the database enforces uniqueness).
    pub unique: bool,
    pub kind: Kind,
}

use Dv::*;
use Ty::*;

// ── Device protocols ──

const MODBUS_REGISTER: &[Field] = &[
    Field::r("name", Text),
    Field::r("address", Int),
    Field::d("register_type", Text, S("float32")),
    Field::d("register_area", Text, S("holding")),
    Field::d("scale", Real, R(1.0)),
    Field::d("sort_order", Int, I(0)).lean(),
    Field::d("zero_based", Bool, B(false)).lean(),
];

const MODBUS_DEVICE: &[Field] = &[
    Field::r("id", Text),
    Field::r("name", Text),
    Field::r("ip", Text),
    Field::d("port", Int, I(502)),
    Field::d("unit_id", Int, I(1)),
    Field::d("interval_ms", Int, I(1000)),
    Field::d("endianness", Text, S("big")),
    Field::d("enabled", Bool, B(true)),
    Field::d("registers", List(MODBUS_REGISTER), Arr)
        .virt()
        .lean(),
];

static MODBUS: Plain = Plain {
    table: "modbus_devices",
    pk: Pk::Text,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: Some("updated_at"),
    children: &[Child {
        yaml: "registers",
        table: "modbus_registers",
        fk: "device_id",
        order: "sort_order, id",
        key: &["name"],
        created: false,
    }],
    hook: None,
};

const OPCUA_NODE: &[Field] = &[
    Field::r("node_id", Text),
    Field::r("name", Text),
    Field::d("display_name", Text, S("")).lean(),
    Field::d("data_type", Text, S("")).lean(),
];

const OPCUA_DEVICE: &[Field] = &[
    Field::r("name", Text),
    Field::r("url", Text),
    Field::d("security_policy", Text, S("none")),
    Field::d("security_mode", Text, S("none")),
    Field::o("username", Text),
    Field::o("password", Text),
    Field::d("interval_ms", Int, I(1000)),
    Field::d("enabled", Bool, B(true)),
    Field::d("nodes", List(OPCUA_NODE), Arr).virt().lean(),
];

static OPCUA: Plain = Plain {
    table: "opcua_devices",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[Child {
        yaml: "nodes",
        table: "opcua_nodes",
        fk: "device_id",
        order: "created_at, id",
        key: &["node_id"],
        created: true,
    }],
    hook: None,
};

const S7_TAG: &[Field] = &[
    Field::r("tag_address", Text),
    Field::r("name", Text),
    Field::d("display_name", Text, S("")).lean(),
    Field::d("data_type", Text, S("")).lean(),
];

const S7_DEVICE: &[Field] = &[
    Field::r("name", Text),
    Field::r("url", Text),
    Field::d("rack", Int, I(0)),
    Field::d("slot", Int, I(1)),
    Field::d("interval_ms", Int, I(1000)),
    Field::d("enabled", Bool, B(true)),
    Field::d("tags", List(S7_TAG), Arr).virt().lean(),
];

static S7: Plain = Plain {
    table: "s7_devices",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[Child {
        yaml: "tags",
        table: "s7_tags",
        fk: "device_id",
        order: "created_at, id",
        key: &["name"],
        created: true,
    }],
    hook: None,
};

const ENIP_TAG: &[Field] = &[
    Field::r("tag_name", Text),
    Field::r("name", Text),
    Field::d("display_name", Text, S("")).lean(),
    Field::d("data_type", Text, S("")).lean(),
    Field::d("assembly_offset", Int, I(0)).lean(),
    Field::d("assembly_length", Int, I(0)).lean(),
];

const ENIP_DEVICE: &[Field] = &[
    Field::r("name", Text),
    Field::r("url", Text),
    Field::d("slot", Int, I(0)),
    Field::d("interval_ms", Int, I(1000)),
    Field::d("enabled", Bool, B(true)),
    Field::d("assembly_instance", Int, I(0)).lean(),
    Field::d("tags", List(ENIP_TAG), Arr).virt().lean(),
];

static ENIP: Plain = Plain {
    table: "enip_devices",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[Child {
        yaml: "tags",
        table: "enip_tags",
        fk: "device_id",
        order: "created_at, id",
        key: &["tag_name"],
        created: true,
    }],
    hook: None,
};

const DNP3_POINT: &[Field] = &[
    Field::r("group_num", Int),
    Field::r("variation", Int),
    Field::r("point_index", Int),
    Field::r("name", Text),
    Field::d("display_name", Text, S("")).lean(),
    Field::d("data_type", Text, S("float64")),
    Field::d("scale", Real, R(1.0)),
    Field::d("deadband", Real, R(0.0)).lean(),
];

const DNP3_DEVICE: &[Field] = &[
    Field::r("name", Text),
    Field::r("ip", Text),
    Field::d("port", Int, I(20000)),
    Field::d("master_address", Int, I(1)),
    Field::d("outstation_address", Int, I(10)),
    Field::d("enable_unsolicited", Bool, B(true)),
    Field::d("integrity_poll_interval_secs", Int, I(300)),
    Field::o("event_scan_interval_ms", Int),
    Field::d("enabled", Bool, B(true)),
    Field::d("points", List(DNP3_POINT), Arr).virt().lean(),
];

static DNP3: Plain = Plain {
    table: "dnp3_devices",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: Some("updated_at"),
    children: &[Child {
        yaml: "points",
        table: "dnp3_points",
        fk: "device_id",
        order: "created_at, id",
        key: &["name"],
        created: true,
    }],
    hook: None,
};

const BACNET_OBJECT: &[Field] = &[
    Field::r("object_type", Text),
    Field::r("object_instance", Int),
    Field::d("property", Text, S("presentValue")),
    Field::r("name", Text),
    Field::d("display_name", Text, S("")).lean(),
    Field::d("cov_increment", Real, R(0.0)).lean(),
];

const BACNET_DEVICE: &[Field] = &[
    Field::r("name", Text),
    Field::r("ip", Text),
    Field::d("port", Int, I(47808)),
    Field::d("device_instance", Int, I(0)),
    Field::d("interval_ms", Int, I(5000)),
    Field::d("cov_enabled", Bool, B(false)),
    Field::d("cov_lifetime", Int, I(3600)),
    Field::d("enabled", Bool, B(true)),
    Field::d("objects", List(BACNET_OBJECT), Arr).virt().lean(),
];

static BACNET: Plain = Plain {
    table: "bacnet_devices",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[Child {
        yaml: "objects",
        table: "bacnet_objects",
        fk: "device_id",
        order: "created_at, id",
        key: &["object_type", "object_instance"],
        created: true,
    }],
    hook: None,
};

const PLC_TAG: &[Field] = &[
    Field::r("variable_name", Text),
    Field::d("display_name", Text, S("")).lean(),
    Field::d("data_type", Text, S("")).lean(),
];

const PLC_DEVICE: &[Field] = &[
    Field::r("name", Text),
    Field::r("url", Text),
    Field::d("transport", Text, S("grpc")),
    Field::d("interval_ms", Int, I(1000)),
    Field::d("enabled", Bool, B(true)),
    Field::d("tags", List(PLC_TAG), Arr).virt().lean(),
];

static PLC: Plain = Plain {
    table: "voltrus_plc_devices",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[Child {
        yaml: "tags",
        table: "voltrus_plc_tags",
        fk: "device_id",
        order: "created_at, id",
        key: &["variable_name"],
        created: true,
    }],
    hook: None,
};

// ── Simulation, scripts, flows, recipes ──

const SIM_PROFILE: &[Field] = &[
    Field::r("name", Text),
    Field::r("data_type", Text),
    Field::d("config", Json, Map),
];

static SIM: Plain = Plain {
    table: "sim_profiles",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[],
    hook: None,
};

const SCRIPT: &[Field] = &[
    Field::r("name", Text),
    Field::d("enabled", Bool, B(true)),
    Field::d("cpu_limit_ms", Int, I(1000)),
    Field::d("memory_limit_bytes", Int, I(1_048_576)),
    Field::d("source", Text, S("")),
];

static SCRIPTS: Plain = Plain {
    table: "scripts",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: Some("updated_at"),
    children: &[],
    hook: None,
};

const FLOW: &[Field] = &[
    Field::r("id", Text),
    Field::r("name", Text),
    Field::d("enabled", Bool, B(true)),
    Field::r("trigger", Json).col("trigger_config"),
    Field::r("nodes", Json),
];

static FLOWS: Plain = Plain {
    table: "flows",
    pk: Pk::Text,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: Some("updated_at"),
    children: &[],
    hook: Some(Hook {
        export: super::export::flow_export,
        import: super::import::flow_import,
    }),
};

const RECIPE: &[Field] = &[
    Field::r("name", Text),
    Field::o("category", Text),
    Field::r("device_id", Text),
    Field::d("version", Int, I(1)),
    Field::o("description", Text),
    Field::r("values", Json),
];

static RECIPES: Plain = Plain {
    table: "recipes",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: Some("updated_at"),
    children: &[],
    hook: None,
};

// ── Alarm notifications, integrations, reporting ──

const NOTIFICATION: &[Field] = &[
    Field::r("source_key", Text),
    Field::r("channel_type", Text),
    Field::d("enabled", Bool, B(true)),
    Field::d("config", Json, Arr),
];

static NOTIFICATIONS: Plain = Plain {
    table: "notification_settings",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[],
    hook: None,
};

const WEBHOOK: &[Field] = &[
    Field::r("name", Text),
    Field::r("url", Text),
    Field::d("events", Json, J("[\"alarm\"]")),
    Field::d("device_filter", Json, J("\"all\"")),
    Field::d("headers", Json, Map).lean(),
    Field::d("enabled", Bool, B(true)),
    Field::d("retry_count", Int, I(3)),
    Field::d("timeout_ms", Int, I(10000)),
];

static WEBHOOKS: Plain = Plain {
    table: "outbound_webhooks",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[],
    hook: None,
};

const EGRESS: &[Field] = &[
    Field::r("name", Text),
    Field::d("enabled", Bool, B(true)),
    Field::d("schedule", Text, S("daily")),
    Field::d("format", Text, S("csv")),
    Field::r("destination", Text),
    Field::d("device_ids", Text, S("all")),
    Field::d("time_range", Text, S("24h")),
    Field::d("compression", Bool, B(false)),
];

static EGRESS_JOBS: Plain = Plain {
    table: "egress_jobs",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: Some("updated_at"),
    children: &[],
    hook: None,
};

const REPORT: &[Field] = &[
    Field::r("name", Text),
    Field::r("schedule", Text),
    Field::d("enabled", Bool, B(true)),
    Field::d("device_ids", Text, S("all")),
    Field::r("email_recipients", Text),
];

static REPORTS: Plain = Plain {
    table: "reports",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[],
    hook: None,
};

const CAMERA: &[Field] = &[
    Field::r("name", Text),
    Field::r("url", Text),
    Field::d("enabled", Bool, B(true)),
    Field::d("snapshot_on_alarm", Bool, B(true)),
    Field::d("retention_hours", Int, I(72)),
];

static CAMERAS: Plain = Plain {
    table: "cameras",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[],
    hook: None,
};

const HUB_SITE: &[Field] = &[
    Field::r("name", Text),
    Field::r("url", Text),
    Field::r("api_key", Text),
    Field::d("enabled", Bool, B(true)),
];

static HUB_SITES: Plain = Plain {
    table: "hub_sites",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[],
    hook: None,
};

// ── Device templates ──

const TEMPLATE_REGISTER: &[Field] = &[
    Field::r("name", Text),
    Field::r("offset", Int),
    Field::d("register_type", Text, S("float32")),
    Field::d("register_area", Text, S("holding")),
    Field::d("scale", Real, R(1.0)),
    Field::d("unit", Text, S("")).lean(),
    Field::d("description", Text, S("")).lean(),
    Field::d("writable", Bool, B(false)).lean(),
];

const TEMPLATE_OBJECT: &[Field] = &[
    Field::r("object_type", Text),
    Field::r("object_instance", Int),
    Field::d("property", Text, S("presentValue")),
    Field::r("name", Text),
    Field::d("display_name", Text, S("")).lean(),
    Field::d("cov_increment", Real, R(0.0)).lean(),
    Field::d("unit", Text, S("")).lean(),
    Field::d("description", Text, S("")).lean(),
    Field::d("writable", Bool, B(false)).lean(),
];

/// The stored `body` is not a field of a project: the template's own fields
/// stand in for it (see `templates.rs`).
const DEVICE_TEMPLATE: &[Field] = &[
    Field::r("template_id", Text),
    Field::r("version", Text),
    Field::r("source", Text),
    Field::r("manufacturer", Text),
    Field::r("model", Text),
    Field::r("protocol", Text),
    Field::d("description", Text, S("")).virt().lean(),
    Field::d("verified", Bool, B(false)).lean(),
    Field::d("verified_notes", Text, S("")).virt().lean(),
    Field::d("quirks", Json, Arr).virt().lean(),
    Field::d("license", Text, S("CC-BY-4.0")).virt().lean(),
    Field::o("port", Int).virt(),
    Field::o("interval_ms", Int).virt(),
    Field::o("unit_id", Int).virt(),
    Field::o("endianness", Text).virt(),
    Field::d("registers", List(TEMPLATE_REGISTER), Arr)
        .virt()
        .lean(),
    Field::o("cov_enabled", Bool).virt(),
    Field::o("cov_lifetime", Int).virt(),
    Field::d("objects", List(TEMPLATE_OBJECT), Arr)
        .virt()
        .lean(),
    Field::o("body", Json),
];

static DEVICE_TEMPLATES: Plain = Plain {
    table: "device_templates",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[],
    hook: Some(Hook {
        export: super::templates::export,
        import: super::templates::import,
    }),
};

// ── Equipment, shifts, metrics, assets ──

const EQUIPMENT_TEMPLATE: &[Field] = &[
    Field::r("name", Text),
    Field::o("industry", Text),
    Field::d("equipment_type", Text, S("")),
    Field::d("tag_template", Json, Arr),
    Field::d("alarm_template", Json, Arr),
    Field::d("dashboard_template", Json, Map),
    Field::d("flow_template", Json, Arr),
];

static EQUIPMENT_TEMPLATES: Plain = Plain {
    table: "equipment_templates",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: None,
    updated: None,
    children: &[],
    hook: None,
};

const SHIFT: &[Field] = &[
    Field::r("name", Text),
    Field::r("start_hour", Int),
    Field::r("start_minute", Int),
    Field::r("duration_hours", Int),
    Field::d("days_mask", Int, I(127)),
];

static SHIFTS: Plain = Plain {
    table: "shifts",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: None,
    children: &[],
    hook: None,
};

const CUSTOM_METRIC: &[Field] = &[
    Field::r("name", Text),
    Field::d("description", Text, S("")),
    Field::r("formula", Text),
    Field::o("unit", Text),
];

static CUSTOM_METRICS: Plain = Plain {
    table: "custom_metrics",
    pk: Pk::Auto,
    stamp: Stamp::Secs,
    created: Some("created_at"),
    updated: Some("updated_at"),
    children: &[],
    hook: None,
};

const ASSET: &[Field] = &[
    Field::r("id", Text),
    Field::o("device_ref", Text),
    Field::r("name", Text),
    Field::r("asset_type", Text),
    Field::o("vendor", Text),
    Field::o("model", Text),
    Field::o("firmware_version", Text),
    Field::o("serial_number", Text),
    Field::o("ip_address", Text),
    Field::o("mac_address", Text),
    Field::o("location", Text),
    Field::d("criticality", Text, S("medium")),
    Field::d("tags", Json, Arr).lean(),
    Field::o("last_maintenance", Text),
    Field::o("notes", Text),
];

static ASSETS: Plain = Plain {
    table: "assets",
    pk: Pk::Text,
    stamp: Stamp::Iso,
    created: Some("created_at"),
    updated: Some("updated_at"),
    children: &[],
    hook: None,
};

// ── Tags ──

pub const TAG_ALARM: &[Field] = &[
    Field::d("warn", Real, R(0.0)),
    Field::d("crit", Real, R(0.0)),
    Field::o("low_warn", Real),
    Field::o("low_crit", Real),
    Field::o("deadband", Real),
    Field::d("on_delay_secs", Int, I(0)).lean(),
    Field::d("off_delay_secs", Int, I(0)).lean(),
];

pub const TAG_WRITE: &[Field] = &[
    Field::d("writable", Bool, B(false)),
    Field::o("min", Real),
    Field::o("max", Real),
    Field::o("max_rate", Int),
    Field::d("requires_reason", Bool, B(false)).lean(),
    Field::o("deadband", Real),
];

pub const TAG_FIELDS: &[Field] = &[
    Field::r("key", Text),
    Field::d("device", Text, S("default")).col("device_id"),
    Field::o("ref", Text).virt(),
    Field::r("name", Text),
    Field::d("unit", Text, S("")).lean(),
    Field::d("color", Text, S("#f97316")).lean(),
    Field::d("decimals", Int, I(1)).lean(),
    Field::d("min", Real, R(0.0)).col("min_val"),
    Field::d("max", Real, R(0.0)).col("max_val"),
    Field::d("base", Real, R(0.0)).col("base_val"),
    Field::d("sort_order", Int, I(0)).lean(),
    Field::d("chart_type", Text, S("sparkline")).lean(),
    Field::d("source_type", Text, S("random_walk")),
    Field::d("source_config", Json, Map).lean(),
    Field::o("path", Text),
    Field::o("equipment_id", Text),
    Field::o("sim_value", Real),
    Field::d("alarm", Obj(TAG_ALARM), Map).virt().lean(),
    Field::d("write", Obj(TAG_WRITE), Map).virt().lean(),
    Field::o("id_hint", Int).virt(),
];

/// Input-only fields: accepted when parsing, never emitted, ignored when
/// comparing an entity with the database.
pub const INPUT_ONLY: &[&str] = &["id_hint"];

// ── Screens ──

pub const SCREEN_TAG: &[Field] = &[
    Field::r("ref", Text),
    Field::d("sort_order", Int, I(0)).lean(),
];

pub const SCREEN_CANVAS: &[Field] = &[
    Field::r("tag", Text),
    Field::d("x", Real, R(0.0)),
    Field::d("y", Real, R(0.0)),
    Field::d("width", Real, R(300.0)),
    Field::d("height", Real, R(200.0)),
];

pub const TWIN_LAYOUT: &[Field] = &[
    Field::r("equipment_id", Text),
    Field::r("equipment_type", Text),
    Field::d("equipment_name", Text, S("")).lean(),
    Field::d("position_x", Real, R(0.0)),
    Field::d("position_z", Real, R(0.0)),
    Field::d("rotation_y", Real, R(0.0)),
];

pub const TWIN_CONNECTION: &[Field] = &[Field::r("from_id", Text), Field::r("to_id", Text)];

pub const SCREEN_TWIN: &[Field] = &[
    Field::o("camera", Json),
    Field::d("layouts", List(TWIN_LAYOUT), Arr).lean(),
    Field::d("connections", List(TWIN_CONNECTION), Arr).lean(),
];

pub const SCADA_EQUIPMENT: &[Field] = &[
    Field::r("equipment_id", Text),
    Field::r("equipment_type", Text),
    Field::d("equipment_name", Text, S("")).lean(),
    Field::d("x", Real, R(0.0)),
    Field::d("y", Real, R(0.0)),
    Field::d("width", Real, R(100.0)),
    Field::d("height", Real, R(60.0)),
    Field::d("rotation", Real, R(0.0)).lean(),
    Field::d("sensors", Json, Arr).lean(),
    Field::d("config", Json, Map).lean(),
];

pub const SCADA_CONNECTION: &[Field] = &[
    Field::r("connection_id", Text),
    Field::r("from_equipment", Text),
    Field::r("to_equipment", Text),
    Field::d("connection_type", Text, S("power")),
    Field::d("waypoints", Json, Arr).lean(),
];

pub const SCREEN_SCADA: &[Field] = &[
    Field::d("equipment", List(SCADA_EQUIPMENT), Arr).lean(),
    Field::d("connections", List(SCADA_CONNECTION), Arr).lean(),
];

pub const SCREEN_FIELDS: &[Field] = &[
    Field::r("id", Text),
    // Absent for ids that only carry layouts (the overview screen).
    Field::o("name", Text),
    Field::d("type", Text, S("screen")).lean(),
    Field::d("sort_order", Int, I(0)).lean(),
    Field::d("simulation_mode", Bool, B(false)).lean(),
    Field::o("sim_profile", Text),
    Field::o("sim_config", Json),
    Field::d("tags", List(SCREEN_TAG), Arr).lean(),
    Field::d("layouts", Json, Map).lean(),
    Field::d("canvas", List(SCREEN_CANVAS), Arr).lean(),
    Field::d("twin", Obj(SCREEN_TWIN), Map).lean(),
    Field::d("scada", Obj(SCREEN_SCADA), Map).lean(),
];

// ── Settings singleton ──

pub const NAMING: &[Field] = &[Field::o("template", Text), Field::o("regex", Text)];

pub const SECURITY_THRESHOLDS: &[Field] = &[
    Field::d("failed_login_threshold", Int, I(5)),
    Field::d("failed_login_window_secs", Int, I(60)),
    Field::d("suspicious_401_threshold", Int, I(10)),
    Field::d("suspicious_401_window_secs", Int, I(60)),
    Field::d("rate_limit_threshold", Int, I(100)),
    Field::d("rate_limit_window_secs", Int, I(60)),
];

const SETTINGS: &[Field] = &[
    Field::o("alarm_cooldown_secs", Int),
    Field::o("naming_convention", Obj(NAMING)),
    Field::d("oee_thresholds", Json, Map).lean(),
    Field::d("security_thresholds", Obj(SECURITY_THRESHOLDS), Map).lean(),
    Field::o("compliance", Json),
];

/// Every section, in output order. Also the order in which import writes them,
/// so references always point at something already written (scripts and
/// simulation profiles before the screens and flows that name them, tags
/// before the screens that list them).
pub static SECTIONS: &[Section] = &[
    Section {
        name: "modbus_devices",
        label: "Modbus device",
        path: "devices/modbus",
        layout: Layout::Dir,
        fields: MODBUS_DEVICE,
        key: &["id"],
        unique: true,
        kind: Kind::Plain(&MODBUS),
    },
    Section {
        name: "opcua_devices",
        label: "OPC UA device",
        path: "devices/opcua",
        layout: Layout::Dir,
        fields: OPCUA_DEVICE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&OPCUA),
    },
    Section {
        name: "s7_devices",
        label: "S7 device",
        path: "devices/s7",
        layout: Layout::Dir,
        fields: S7_DEVICE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&S7),
    },
    Section {
        name: "enip_devices",
        label: "EtherNet/IP device",
        path: "devices/enip",
        layout: Layout::Dir,
        fields: ENIP_DEVICE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&ENIP),
    },
    Section {
        name: "dnp3_devices",
        label: "DNP3 device",
        path: "devices/dnp3",
        layout: Layout::Dir,
        fields: DNP3_DEVICE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&DNP3),
    },
    Section {
        name: "bacnet_devices",
        label: "BACnet device",
        path: "devices/bacnet",
        layout: Layout::Dir,
        fields: BACNET_DEVICE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&BACNET),
    },
    Section {
        name: "voltrus_plc_devices",
        label: "Voltrus PLC device",
        path: "devices/voltrus_plc",
        layout: Layout::Dir,
        fields: PLC_DEVICE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&PLC),
    },
    Section {
        name: "device_templates",
        label: "device template",
        path: "device_templates",
        layout: Layout::File,
        fields: DEVICE_TEMPLATE,
        key: &["template_id", "version", "source"],
        unique: true,
        kind: Kind::Plain(&DEVICE_TEMPLATES),
    },
    Section {
        name: "sim_profiles",
        label: "simulation profile",
        path: "sim_profiles",
        layout: Layout::File,
        fields: SIM_PROFILE,
        key: &["name"],
        unique: true,
        kind: Kind::Plain(&SIM),
    },
    Section {
        name: "scripts",
        label: "script",
        path: "scripts",
        layout: Layout::Dir,
        fields: SCRIPT,
        key: &["name"],
        unique: true,
        kind: Kind::Plain(&SCRIPTS),
    },
    Section {
        name: "tags",
        label: "tag",
        path: "tags",
        layout: Layout::Dir,
        fields: TAG_FIELDS,
        key: &["device", "key"],
        unique: true,
        kind: Kind::Tags,
    },
    Section {
        name: "screens",
        label: "screen",
        path: "screens",
        layout: Layout::Dir,
        fields: SCREEN_FIELDS,
        key: &["id"],
        unique: true,
        kind: Kind::Screens,
    },
    Section {
        name: "flows",
        label: "flow",
        path: "flows",
        layout: Layout::Dir,
        fields: FLOW,
        key: &["id"],
        unique: true,
        kind: Kind::Plain(&FLOWS),
    },
    Section {
        name: "recipes",
        label: "recipe",
        path: "recipes",
        layout: Layout::Dir,
        fields: RECIPE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&RECIPES),
    },
    Section {
        name: "notifications",
        label: "notification rule",
        path: "notifications",
        layout: Layout::File,
        fields: NOTIFICATION,
        key: &["source_key", "channel_type"],
        unique: false,
        kind: Kind::Plain(&NOTIFICATIONS),
    },
    Section {
        name: "webhooks",
        label: "outbound webhook",
        path: "webhooks",
        layout: Layout::File,
        fields: WEBHOOK,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&WEBHOOKS),
    },
    Section {
        name: "egress_jobs",
        label: "egress job",
        path: "egress_jobs",
        layout: Layout::File,
        fields: EGRESS,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&EGRESS_JOBS),
    },
    Section {
        name: "reports",
        label: "report",
        path: "reports",
        layout: Layout::File,
        fields: REPORT,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&REPORTS),
    },
    Section {
        name: "cameras",
        label: "camera",
        path: "cameras",
        layout: Layout::File,
        fields: CAMERA,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&CAMERAS),
    },
    Section {
        name: "hub_sites",
        label: "hub site",
        path: "hub_sites",
        layout: Layout::File,
        fields: HUB_SITE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&HUB_SITES),
    },
    Section {
        name: "equipment_templates",
        label: "equipment template",
        path: "equipment_templates",
        layout: Layout::File,
        fields: EQUIPMENT_TEMPLATE,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&EQUIPMENT_TEMPLATES),
    },
    Section {
        name: "shifts",
        label: "shift",
        path: "shifts",
        layout: Layout::File,
        fields: SHIFT,
        key: &["name"],
        unique: false,
        kind: Kind::Plain(&SHIFTS),
    },
    Section {
        name: "custom_metrics",
        label: "custom metric",
        path: "custom_metrics",
        layout: Layout::File,
        fields: CUSTOM_METRIC,
        key: &["name"],
        unique: true,
        kind: Kind::Plain(&CUSTOM_METRICS),
    },
    Section {
        name: "assets",
        label: "asset",
        path: "assets",
        layout: Layout::File,
        fields: ASSET,
        key: &["id"],
        unique: true,
        kind: Kind::Plain(&ASSETS),
    },
    Section {
        name: "settings",
        label: "settings",
        path: "settings",
        layout: Layout::Single,
        fields: SETTINGS,
        key: &[],
        unique: true,
        kind: Kind::Settings,
    },
];

pub fn section(name: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|s| s.name == name)
}
