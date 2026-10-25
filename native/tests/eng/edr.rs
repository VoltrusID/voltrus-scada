//! EDR compatibility against the live plant: every route the section reads
//! (report, exclusions, I/O status, Group Policy guide, Defender script).
//! Nothing here changes the plant.

use voltrus_scada_native::api::VoltrusApi;
use voltrus_scada_native::api::eng_edr::{self, PS1};

pub(crate) async fn round_trip(api: &VoltrusApi) {
    let report = api.edr_report().await.expect("EDR report");
    assert!(
        !report.recommended_exclusions.is_empty(),
        "the server always recommends exclusions (at least its database)"
    );
    println!(
        "edr: {} — {}, {} exclusions, {} recommendations",
        if report.compatible {
            "compatible"
        } else {
            "issues"
        },
        report.banner(),
        report.recommended_exclusions.len(),
        report.recommendations.len()
    );

    let exclusions = api.edr_exclusions().await.expect("EDR exclusions");
    assert_eq!(exclusions.count, exclusions.exclusions.len());
    assert_eq!(exclusions.exclusions, report.recommended_exclusions);
    println!("edr: {} exclusions", exclusions.count);

    let io = api.edr_io_status().await.expect("I/O status");
    assert_eq!(io.batch_size, report.io_optimization.batch_size);
    println!(
        "edr: I/O WAL {} · checkpoint {} · batch {}",
        io.wal_mode_active, io.checkpoint_optimized, io.batch_size
    );

    let guide = api.edr_gpo_guide().await.expect("GPO guide");
    let lines = eng_edr::text_lines(&guide);
    assert!(!lines.is_empty(), "the guide has text");
    println!("edr: deploy guide, {} lines", lines.len());

    // The script (an operator route; the test signs in as admin), the
    // same text the section saves through `ops_kit::save_download`.
    let script = api.edr_defender_ps1().await.expect("Defender script");
    let raw = api.get_text(PS1).await.expect("Defender script as text");
    assert_eq!(script, raw);
    assert!(
        report
            .recommended_exclusions
            .iter()
            .filter(|exclusion| exclusion.exclusion_type == "directory")
            .all(|exclusion| script.contains(&exclusion.path)),
        "every directory exclusion is in the script"
    );
    println!(
        "edr: Defender script, {} lines",
        eng_edr::text_lines(&script).len()
    );
}
