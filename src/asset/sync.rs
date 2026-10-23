use crate::asset::storage::AssetStorage;

pub async fn on_device_created(
    storage: &AssetStorage,
    protocol: &str,
    device_id: &str,
    name: &str,
    ip: Option<&str>,
    asset_type: &str,
) {
    let device_ref = format!("{}:{}", protocol, device_id);
    if let Err(e) = storage.link_device(&device_ref, name, ip, asset_type).await {
        eprintln!("Failed to link asset for device {}: {}", device_ref, e);
    }
}

pub async fn on_device_deleted(storage: &AssetStorage, protocol: &str, device_id: &str) {
    let device_ref = format!("{}:{}", protocol, device_id);
    if let Err(e) = storage.unlink_device(&device_ref).await {
        eprintln!("Failed to unlink asset for device {}: {}", device_ref, e);
    }
}
