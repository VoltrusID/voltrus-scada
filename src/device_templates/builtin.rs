//! Read-only templates generated at startup from the simulator device profiles
//! in `sim_profiles.rs`. They are never stored: the library lists them next to
//! the stored ones and refuses to delete them.

use std::sync::LazyLock;

use super::format::{
    DEFAULT_BACNET_PORT, DEFAULT_COV_LIFETIME, DEFAULT_INTERVAL_MS, DEFAULT_LICENSE,
    DeviceTemplate, Protocol, SOURCE_BUILTIN, TEMPLATE_FORMAT, TemplateObject, TemplateRegister,
    slugify,
};
use crate::sim_profiles::{BacnetProfile, DeviceProfile};

const SIM_NOTE: &str =
    "Derived from the Voltrus simulator profile; not checked against a physical device.";

static BUILTINS: LazyLock<Vec<DeviceTemplate>> = LazyLock::new(|| {
    let mut out: Vec<DeviceTemplate> = [
        crate::sim_profiles::profile_sma_inverter(),
        crate::sim_profiles::profile_pm5560(),
        crate::sim_profiles::profile_eh_flowmeter(),
        crate::sim_profiles::fn_profile_abb_analyzer(),
        crate::sim_profiles::profile_daniel_flow(),
        crate::sim_profiles::profile_raritan_pdu(),
    ]
    .iter()
    .map(modbus_template)
    .collect();
    out.extend(
        [
            crate::sim_profiles::bacnet_profile_ahu_datacenter(),
            crate::sim_profiles::bacnet_profile_ahu_water_treatment(),
        ]
        .iter()
        .map(bacnet_template),
    );
    out
});

/// The built-in templates, built once.
pub fn builtins() -> &'static [DeviceTemplate] {
    &BUILTINS
}

fn base(
    manufacturer: &str,
    model: &str,
    protocol: Protocol,
    description: &str,
    quirks: Vec<String>,
) -> DeviceTemplate {
    DeviceTemplate {
        format: TEMPLATE_FORMAT.to_string(),
        template_id: slugify(&format!("{manufacturer} {model}")),
        manufacturer: manufacturer.to_string(),
        model: model.to_string(),
        protocol,
        description: description.to_string(),
        version: "1.0.0".to_string(),
        verified: false,
        verified_notes: SIM_NOTE.to_string(),
        quirks,
        source: SOURCE_BUILTIN.to_string(),
        license: DEFAULT_LICENSE.to_string(),
        port: None,
        interval_ms: Some(DEFAULT_INTERVAL_MS),
        unit_id: None,
        endianness: None,
        registers: Vec::new(),
        cov_enabled: None,
        cov_lifetime: None,
        objects: Vec::new(),
    }
}

/// The simulator lays the registers out back to back from holding offset 0:
/// a Float32 takes 2 registers, high word first; a 64-bit counter takes 4.
/// The poller has no unsigned 64-bit type, so counters are left out of the
/// register list and named in the quirks, their words still counted in the offsets.
fn modbus_template(p: &DeviceProfile) -> DeviceTemplate {
    let mut registers = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut offset: u32 = 0;
    for def in &p.registers {
        if def.is_counter {
            skipped.push(format!(
                "{} (offset {offset}, {})",
                def.name,
                if def.unit.is_empty() {
                    "no unit"
                } else {
                    def.unit
                }
            ));
            offset += 4;
            continue;
        }
        registers.push(TemplateRegister {
            name: def.name.to_string(),
            offset: offset as u16,
            register_type: "float32".to_string(),
            register_area: "holding".to_string(),
            scale: 1.0,
            unit: def.unit.to_string(),
            description: def.description.to_string(),
            writable: false,
        });
        offset += 2;
    }
    let mut quirks = vec![
        "Float32 values, 2 registers each, high word first (big endianness); holding registers from offset 0.".to_string(),
    ];
    if !skipped.is_empty() {
        quirks.push(format!(
            "Not mapped: unsigned 64-bit counters ({}) because the poller has no uint64 type; their 4 registers each are still counted in the offsets.",
            skipped.join(", ")
        ));
    }
    let mut t = base(
        p.manufacturer,
        p.model,
        Protocol::Modbus,
        p.description,
        quirks,
    );
    t.port = Some(502);
    t.unit_id = Some(1);
    t.endianness = Some("big".to_string());
    t.registers = registers;
    t
}

fn bacnet_type_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0 => "analogInput",
        1 => "analogOutput",
        2 => "analogValue",
        3 => "binaryInput",
        4 => "binaryOutput",
        5 => "binaryValue",
        13 => "multiStateInput",
        14 => "multiStateOutput",
        19 => "multiStateValue",
        _ => return None,
    })
}

fn bacnet_template(p: &BacnetProfile) -> DeviceTemplate {
    let mut t = base(
        p.vendor_name,
        p.model_name,
        Protocol::Bacnet,
        p.description,
        vec!["Present values only; the object list is the simulator's, a real controller's list depends on its application.".to_string()],
    );
    t.port = Some(DEFAULT_BACNET_PORT);
    t.cov_enabled = Some(false);
    t.cov_lifetime = Some(DEFAULT_COV_LIFETIME);
    t.objects = p
        .objects
        .iter()
        .filter_map(|o| {
            let object_type = bacnet_type_name(o.obj_type)?;
            Some(TemplateObject {
                object_type: object_type.to_string(),
                object_instance: o.instance,
                property: "presentValue".to_string(),
                name: o.name.to_string(),
                display_name: String::new(),
                cov_increment: 0.0,
                unit: String::new(),
                description: o.description.to_string(),
                writable: object_type.ends_with("Output"),
            })
        })
        .collect();
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtins_validate_and_are_unique() {
        let all = builtins();
        assert_eq!(all.len(), 8);
        let mut ids = std::collections::HashSet::new();
        for t in all {
            let mut c = t.clone();
            c.normalize_and_validate()
                .unwrap_or_else(|e| panic!("{}: {e:?}", t.template_id));
            assert!(!t.verified);
            assert!(t.is_builtin());
            assert!(
                ids.insert(t.template_id.clone()),
                "duplicate {}",
                t.template_id
            );
        }
    }

    #[test]
    fn no_bacnet_object_is_dropped() {
        for p in [
            crate::sim_profiles::bacnet_profile_ahu_datacenter(),
            crate::sim_profiles::bacnet_profile_ahu_water_treatment(),
        ] {
            let t = bacnet_template(&p);
            assert_eq!(t.objects.len(), p.objects.len(), "{}", t.template_id);
        }
    }

    #[test]
    fn pm5560_layout_matches_the_simulator() {
        let t = builtins()
            .iter()
            .find(|t| t.template_id == "schneider-electric-pm5560")
            .unwrap();
        // Float32 pairs from offset 0, counters take 4 words but are not mapped.
        let p = crate::sim_profiles::profile_pm5560();
        let mut offset = 0u16;
        let mut i = 0;
        for def in &p.registers {
            if def.is_counter {
                offset += 4;
                continue;
            }
            assert_eq!(t.registers[i].name, def.name);
            assert_eq!(t.registers[i].offset, offset);
            offset += 2;
            i += 1;
        }
        assert_eq!(i, t.registers.len());
        assert!(t.quirks.iter().any(|q| q.contains("Energy_Import")));
    }
}
