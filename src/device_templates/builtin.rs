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

/// The simulator lays the registers out back to back from holding offset 0,
/// unless a register has a fixed address (the real device's): a Float32 takes
/// 2 registers, high word first; a 64-bit counter takes 4, most significant
/// word first. Counters in Wh or VARh are scaled to kWh or kvarh.
fn modbus_template(p: &DeviceProfile) -> DeviceTemplate {
    let registers: Vec<TemplateRegister> = p
        .registers
        .iter()
        .zip(p.layout())
        .map(|(def, offset)| {
            let (register_type, unit, scale) = if def.is_counter {
                let (unit, scale) = match def.unit {
                    "Wh" => ("kWh", 0.001),
                    "VARh" => ("kvarh", 0.001),
                    unit => (unit, 1.0),
                };
                (if def.signed { "int64" } else { "uint64" }, unit, scale)
            } else {
                ("float32", def.unit, 1.0)
            };
            TemplateRegister {
                name: def.name.to_string(),
                offset: offset as u16,
                register_type: register_type.to_string(),
                register_area: "holding".to_string(),
                scale,
                unit: unit.to_string(),
                description: def.description.to_string(),
                writable: false,
            }
        })
        .collect();
    let mut quirks = vec![
        "Float32 values, 2 registers each, high word first (big endianness); holding registers from offset 0.".to_string(),
    ];
    if p.registers.iter().any(|d| d.is_counter) {
        quirks.push(
            "64-bit counters, 4 registers each, most significant word first. Values above 2^53 (9.0e15) lose precision.".to_string(),
        );
    }
    if let Some(note) = real_device_note(p) {
        quirks.push(note.to_string());
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

/// What a template says about which of its registers match the real device.
fn real_device_note(p: &DeviceProfile) -> Option<&'static str> {
    (p.model == "PM5560").then_some(
        "Only the four energy registers (offsets 3203, 3207, 3219, 3223: INT64 Wh/VARh, read as kWh/kvarh) \
         follow Schneider's register list (PM556x_PublicModbusRegisterList_10th_August-23.xlsx, \
         FAQ FA234017; its register numbers are 1-based, offset = register - 1). The float32 \
         registers from offset 0 are the simulator's layout, not the meter's: on a real PM5560 \
         remap them from that list (e.g. Current A is register 3000, Voltage A-N 3028).",
    )
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
        let p = crate::sim_profiles::profile_pm5560();
        assert_eq!(t.registers.len(), p.registers.len());
        // Float32 pairs from offset 0, then the energy counters at their fixed
        // addresses.
        let mut offset = 0u16;
        for (r, def) in t.registers.iter().zip(&p.registers) {
            assert_eq!(r.name, def.name);
            if def.is_counter {
                assert_eq!(r.register_type, "int64");
                continue;
            }
            assert_eq!(r.register_type, "float32");
            assert_eq!(r.offset, offset);
            offset += 2;
        }
    }

    /// The energy registers are where Schneider's PM556x register list puts
    /// them (1-based registers 3204, 3208, 3220, 3224: INT64 in Wh / VARh),
    /// read as kWh / kvarh.
    #[test]
    fn pm5560_energy_registers_follow_the_schneider_list() {
        let t = builtins()
            .iter()
            .find(|t| t.template_id == "schneider-electric-pm5560")
            .unwrap();
        let reg = |name: &str| t.registers.iter().find(|r| r.name == name).unwrap();
        for (name, register, unit) in [
            ("Active_Energy_Delivered", 3204, "kWh"),
            ("Active_Energy_Received", 3208, "kWh"),
            ("Reactive_Energy_Delivered", 3220, "kvarh"),
            ("Reactive_Energy_Received", 3224, "kvarh"),
        ] {
            let r = reg(name);
            assert_eq!(r.offset, register - 1, "{name}");
            assert_eq!(r.register_type, "int64", "{name}");
            assert_eq!(r.scale, 0.001, "{name}");
            assert_eq!(r.unit, unit, "{name}");
        }
        t.clone().normalize_and_validate().unwrap();
    }

    /// Counters of the other simulators are mapped as uint64 in their place.
    #[test]
    fn simulator_counters_are_mapped_as_uint64() {
        let t = builtins()
            .iter()
            .find(|t| t.template_id.starts_with("sma"))
            .unwrap();
        let p = crate::sim_profiles::profile_sma_inverter();
        let counters: Vec<_> = p.registers.iter().filter(|d| d.is_counter).collect();
        assert!(!counters.is_empty());
        for def in counters {
            let r = t.registers.iter().find(|r| r.name == def.name).unwrap();
            assert_eq!(r.register_type, "uint64", "{}", def.name);
        }
    }
}
