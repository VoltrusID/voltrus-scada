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
        legacy_white_label: false,
        site: None,
        support_until: None,
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

#[test]
fn logo_only_white_label_grant_unlocks_only_white_label() {
    let granted = LicenseStatus::Valid {
        license_key: "k".into(),
        expires_at: None,
        days_remaining: None,
        tier: Some("starter".into()),
        grace: false,
        legacy_white_label: true,
        site: None,
        support_until: None,
    };
    assert_eq!(granted.tier(), LicenseTier::Community);
    assert!(granted.white_label());
    let flags = granted.feature_flags();
    for (key, value) in flags.as_object().unwrap() {
        assert_eq!(value, &serde_json::json!(key == "white_label"), "{key}");
    }
    // The grant only counts on a Valid status.
    assert!(!LicenseStatus::Community.white_label());
}

fn valid_with(site: Option<LicenseSite>, support_until: Option<i64>) -> LicenseStatus {
    LicenseStatus::Valid {
        license_key: "k".into(),
        expires_at: None,
        days_remaining: None,
        tier: Some("integrator".into()),
        grace: false,
        legacy_white_label: false,
        site,
        support_until,
    }
}

#[test]
fn support_state_flips_exactly_at_support_until() {
    assert_eq!(SupportState::at(None, 100), SupportState::None);
    assert_eq!(SupportState::at(Some(100), 99), SupportState::Active);
    assert_eq!(SupportState::at(Some(100), 100), SupportState::Lapsed);
    assert_eq!(SupportState::at(Some(100), 101), SupportState::Lapsed);
    assert_eq!(
        valid_with(None, Some(100)).support_at(50),
        SupportState::Active
    );
    assert_eq!(
        valid_with(None, Some(100)).support_at(150),
        SupportState::Lapsed
    );
    assert_eq!(valid_with(None, None).support_at(150), SupportState::None);
    assert_eq!(LicenseStatus::Community.support_at(0), SupportState::None);
}

#[test]
fn old_serialized_valid_status_deserializes_without_site_fields() {
    let old = r#"{"status":"Valid","license_key":"k","expires_at":null,"days_remaining":null,"tier":"enterprise","grace":false}"#;
    let status: LicenseStatus = serde_json::from_str(old).unwrap();
    match status {
        LicenseStatus::Valid {
            site,
            support_until,
            legacy_white_label,
            ..
        } => {
            assert!(site.is_none());
            assert!(support_until.is_none());
            assert!(!legacy_white_label);
        }
        other => panic!("expected Valid, got {other:?}"),
    }
}

#[test]
fn site_and_support_round_trip_through_json() {
    let site = LicenseSite {
        id: "site-1".into(),
        name: "Plant A".into(),
    };
    let json = serde_json::to_value(valid_with(Some(site.clone()), Some(1_900_000_000))).unwrap();
    assert_eq!(json["site"]["name"], "Plant A");
    assert_eq!(json["support_until"], 1_900_000_000);
    match serde_json::from_value::<LicenseStatus>(json).unwrap() {
        LicenseStatus::Valid {
            site: s,
            support_until,
            ..
        } => {
            assert_eq!(s, Some(site));
            assert_eq!(support_until, Some(1_900_000_000));
        }
        other => panic!("expected Valid, got {other:?}"),
    }
    let bare = serde_json::to_value(valid_with(None, None)).unwrap();
    assert!(bare.get("site").is_none() && bare.get("support_until").is_none());
}
