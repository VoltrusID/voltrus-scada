//! Starter equipment types that ship with the server (`library.json`):
//! Pump, Valve, Control valve, Tank, Analog instrument, Dosing pump and
//! Filter, each with the faceplate it opens. Installing one copies it into
//! the database, where it is an ordinary, editable type.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use super::EquipmentType;
use crate::faceplate::Faceplate;

#[derive(Deserialize)]
struct Library {
    faceplates: Vec<Faceplate>,
    types: Vec<EquipmentType>,
}

static LIBRARY: LazyLock<Library> = LazyLock::new(|| {
    serde_json::from_str(include_str!("library.json")).expect("library.json is valid")
});

/// One starter type and the faceplate definition it opens.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct LibraryEntry {
    #[serde(rename = "type")]
    pub equipment_type: EquipmentType,
    pub faceplate: Option<Faceplate>,
}

pub fn entries() -> Vec<LibraryEntry> {
    LIBRARY
        .types
        .iter()
        .map(|t| LibraryEntry {
            equipment_type: t.clone(),
            faceplate: t
                .faceplate
                .as_ref()
                .and_then(|id| LIBRARY.faceplates.iter().find(|f| &f.id == id))
                .cloned(),
        })
        .collect()
}

pub fn get(id: &str) -> Option<LibraryEntry> {
    entries().into_iter().find(|e| e.equipment_type.id == id)
}
