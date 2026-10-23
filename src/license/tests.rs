use super::*;

#[test]
fn community_has_no_addon_features() {
    let t = LicenseTier::Community;
    assert!(!t.white_label());
    assert!(!t.theme_full());
    assert!(!t.pdf_reporting());
    assert!(!t.sms_notifications());
    assert!(!t.multi_site());
    assert!(!t.sso());
    assert!(!t.sso_saml());
    assert!(!t.cfr_part_11());
    assert!(!t.iec_62443());
    assert!(!t.ha());
    assert!(!t.rtsp_video());
}

#[test]
fn integrator_unlocks_branding_reports_and_hub_only() {
    let t = LicenseTier::Integrator;
    assert!(t.white_label());
    assert!(t.theme_full());
    assert!(t.pdf_reporting());
    assert!(t.sms_notifications());
    assert!(t.multi_site());
    assert!(!t.sso());
    assert!(!t.sso_saml());
    assert!(!t.cfr_part_11());
    assert!(!t.iec_62443());
    assert!(!t.ha());
    assert!(!t.rtsp_video());
}

#[test]
fn enterprise_unlocks_everything() {
    let t = LicenseTier::Enterprise;
    assert!(t.white_label() && t.theme_full() && t.pdf_reporting());
    assert!(t.sms_notifications() && t.multi_site());
    assert!(t.sso() && t.sso_saml() && t.cfr_part_11());
    assert!(t.iec_62443() && t.ha() && t.rtsp_video());
}

#[test]
fn legacy_tier_strings_map_per_the_split() {
    assert_eq!(
        LicenseTier::from_tier_str(Some("integrator")),
        LicenseTier::Integrator
    );
    assert_eq!(
        LicenseTier::from_tier_str(Some("Professional")),
        LicenseTier::Integrator
    );
    assert_eq!(
        LicenseTier::from_tier_str(Some("pro")),
        LicenseTier::Integrator
    );
    assert_eq!(
        LicenseTier::from_tier_str(Some("enterprise")),
        LicenseTier::Enterprise
    );
    assert_eq!(
        LicenseTier::from_tier_str(Some("starter")),
        LicenseTier::Community
    );
    assert_eq!(
        LicenseTier::from_tier_str(Some("edge")),
        LicenseTier::Community
    );
    assert_eq!(
        LicenseTier::from_tier_str(Some("engineering")),
        LicenseTier::Community
    );
    assert_eq!(
        LicenseTier::from_tier_str(Some("nonsense")),
        LicenseTier::Community
    );
    assert_eq!(LicenseTier::from_tier_str(None), LicenseTier::Community);
}

#[test]
fn feature_flags_have_exactly_the_documented_keys() {
    let flags = LicenseTier::Enterprise.to_feature_flags();
    let mut keys: Vec<_> = flags.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "cfr_part_11",
            "ha",
            "iec_62443",
            "multi_site",
            "pdf_reporting",
            "rtsp_video",
            "sms_notifications",
            "sso",
            "sso_saml",
            "theme_full",
            "white_label",
        ]
    );
}

#[test]
fn only_a_valid_licence_unlocks_a_tier() {
    let valid = LicenseStatus::Valid {
        license_key: "k".into(),
        expires_at: None,
        days_remaining: None,
        tier: Some("enterprise".into()),
        grace: false,
    };
    assert_eq!(valid.tier(), LicenseTier::Enterprise);
    assert_eq!(LicenseStatus::Community.tier(), LicenseTier::Community);
    let expired = LicenseStatus::Expired {
        expires_at: "2020-01-01T00:00:00Z".into(),
        message: "x".into(),
    };
    assert_eq!(expired.tier(), LicenseTier::Community);
    assert_eq!(expired.feature_flags()["sso"], false);
}

#[test]
fn status_json_is_tagged_by_status() {
    let v = serde_json::to_value(LicenseStatus::Community).unwrap();
    assert_eq!(v["status"], "Community");
}
