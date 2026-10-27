use super::Storage;
use crate::models::LicenseActivationRow;

fn row(site: bool) -> LicenseActivationRow {
    LicenseActivationRow {
        license_key: "VT-1".into(),
        fingerprint_hash: "h".into(),
        activation_blob: vec![1, 2, 3],
        fingerprint_hostname: "host".into(),
        fingerprint_cpu: "cpu".into(),
        fingerprint_disk_serial: "disk".into(),
        fingerprint_mac: "mac".into(),
        fingerprint_machine_id: "mid".into(),
        activated_at: "2026-01-01T00:00:00Z".into(),
        expires_at: None,
        tier: Some("integrator".into()),
        device_limit: Some(5),
        site_id: site.then(|| "site-1".to_string()),
        site_name: site.then(|| "Plant A".to_string()),
        support_until: site.then_some(1_900_000_000),
    }
}

#[tokio::test]
async fn licence_activation_round_trips_site_and_support() {
    let storage = Storage::new_in_memory().unwrap();
    assert!(storage.get_license_activation().await.unwrap().is_none());

    storage.store_license_activation(&row(true)).await.unwrap();
    let got = storage.get_license_activation().await.unwrap().unwrap();
    assert_eq!(got.site_id.as_deref(), Some("site-1"));
    assert_eq!(got.site_name.as_deref(), Some("Plant A"));
    assert_eq!(got.support_until, Some(1_900_000_000));
    assert_eq!(got.device_limit, Some(5));

    // A pre-per-site key stores and reads back with the new columns NULL.
    storage.store_license_activation(&row(false)).await.unwrap();
    let got = storage.get_license_activation().await.unwrap().unwrap();
    assert!(got.site_id.is_none() && got.site_name.is_none() && got.support_until.is_none());
}
