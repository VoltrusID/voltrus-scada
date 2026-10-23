#![allow(dead_code)]
use bacnet_rs::object::{ObjectType, PropertyIdentifier};

pub(super) fn object_type_from_str(s: &str) -> Option<ObjectType> {
    match s.to_lowercase().as_str() {
        "analoginput" | "ai" => Some(ObjectType::AnalogInput),
        "analogoutput" | "ao" => Some(ObjectType::AnalogOutput),
        "analogvalue" | "av" => Some(ObjectType::AnalogValue),
        "binaryinput" | "bi" => Some(ObjectType::BinaryInput),
        "binaryoutput" | "bo" => Some(ObjectType::BinaryOutput),
        "binaryvalue" | "bv" => Some(ObjectType::BinaryValue),
        "multistateinput" | "mi" => Some(ObjectType::MultiStateInput),
        "multistateoutput" | "mo" => Some(ObjectType::MultiStateOutput),
        "multistatevalue" | "mv" => Some(ObjectType::MultiStateValue),
        _ => None,
    }
}

pub(super) fn object_type_to_string(ot: &ObjectType) -> String {
    match ot {
        ObjectType::AnalogInput => "analogInput".to_string(),
        ObjectType::AnalogOutput => "analogOutput".to_string(),
        ObjectType::AnalogValue => "analogValue".to_string(),
        ObjectType::BinaryInput => "binaryInput".to_string(),
        ObjectType::BinaryOutput => "binaryOutput".to_string(),
        ObjectType::BinaryValue => "binaryValue".to_string(),
        ObjectType::MultiStateInput => "multiStateInput".to_string(),
        ObjectType::MultiStateOutput => "multiStateOutput".to_string(),
        ObjectType::MultiStateValue => "multiStateValue".to_string(),
        ObjectType::Device => "device".to_string(),
        _ => format!("{:?}", ot),
    }
}

pub(super) fn property_id_from_str(s: &str) -> PropertyIdentifier {
    match s.to_lowercase().as_str() {
        "presentvalue" | "present_value" => PropertyIdentifier::PresentValue,
        "description" => PropertyIdentifier::Description,
        "objectname" | "object_name" => PropertyIdentifier::ObjectName,
        "statusflags" | "status_flags" => PropertyIdentifier::StatusFlags,
        "units" => PropertyIdentifier::Units,
        "objectlist" | "object_list" => PropertyIdentifier::ObjectList,
        "objectidentifier" | "object_identifier" => PropertyIdentifier::ObjectIdentifier,
        "objecttype" | "object_type" => PropertyIdentifier::ObjectType,
        "maxapdulengthaccepted" => PropertyIdentifier::MaxApduLengthAccepted,
        "segmentationsupported" => PropertyIdentifier::SegmentationSupported,
        "vendorname" | "vendor_name" => PropertyIdentifier::VendorName,
        "vendoridentifier" | "vendor_identifier" => PropertyIdentifier::VendorIdentifier,
        "modelname" | "model_name" => PropertyIdentifier::ModelName,
        "firmwarerevision" | "firmware_revision" => PropertyIdentifier::FirmwareRevision,
        _ => PropertyIdentifier::PresentValue,
    }
}
