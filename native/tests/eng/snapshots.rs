//! Config snapshots against the live plant: list, create, open, diff and a
//! rollback to the snapshot just made (so the configuration ends as it
//! began). The server has no DELETE for snapshots: the one made here and
//! the `restore` snapshot the rollback saves first stay in the list.

use voltrus_scada_native::api::VoltrusApi;
use voltrus_scada_native::api::eng_snapshots::{ConfigSnapshot, SNAPSHOTS};

pub(super) async fn round_trip(api: &VoltrusApi) {
    let before = api.config_snapshots().await.expect("list snapshots");
    println!("snapshots: {} listed", before.len());

    // The raw list decodes as the mirror too.
    let raw: Vec<ConfigSnapshot> = api.get_json(SNAPSHOTS).await.expect("raw snapshot list");
    assert_eq!(raw.len(), before.len());

    let created = api
        .create_config_snapshot("native e2e")
        .await
        .expect("create a snapshot");
    assert_eq!(created.label.as_deref(), Some("native e2e"));
    assert_eq!(created.trigger, "manual");
    assert!(!created.unreadable, "the snapshot data is a JSON object");
    assert!(
        created
            .tables
            .iter()
            .any(|table| table.name == "data_sources" && table.rows > 0),
        "the plant's data sources are in the snapshot: {}",
        created.counts_line()
    );
    println!(
        "snapshots: created #{} ({}, {} bytes)",
        created.id,
        created.counts_line(),
        created.bytes
    );

    let listed = api.config_snapshots().await.expect("list again");
    assert!(listed.iter().any(|snapshot| snapshot.id == created.id));

    let detail = api
        .config_snapshot(created.id)
        .await
        .expect("open the snapshot");
    assert_eq!(detail.summary.id, created.id);
    let sources = detail
        .tables
        .iter()
        .find(|table| table.name == "data_sources")
        .expect("a data_sources table");
    assert!(!sources.rows.is_empty());
    println!(
        "snapshots: opened #{} ({} tables, e.g. {})",
        created.id,
        detail.tables.len(),
        sources.rows[0]
    );

    // A snapshot against itself differs nowhere.
    let same = api
        .diff_config_snapshots(created.id, created.id)
        .await
        .expect("diff a snapshot with itself");
    assert!(same.is_empty(), "no differences: {same:?}");
    if let Some(older) = before.first() {
        let diff = api
            .diff_config_snapshots(older.id, created.id)
            .await
            .expect("diff the newest older snapshot");
        println!(
            "snapshots: diff #{} → #{}: {} tables differ, totals {:?}",
            older.id,
            created.id,
            diff.tables.len(),
            diff.totals()
        );
    }

    // Roll back to what was just saved: the configuration stays the same.
    api.rollback_config_snapshot(created.id)
        .await
        .expect("roll back to the new snapshot");
    let after = api.config_snapshots().await.expect("list after rollback");
    let restore = after
        .iter()
        .find(|snapshot| snapshot.trigger == "restore" && snapshot.id > created.id)
        .expect("the server saved a restore snapshot before rolling back");
    let check = api
        .diff_config_snapshots(created.id, restore.id)
        .await
        .expect("diff the snapshot with the pre-rollback state");
    // `alarms` holds the live alarm events, which a running plant raises
    // and clears between the two snapshots; the configuration tables
    // must match.
    let changed: Vec<&str> = check
        .tables
        .iter()
        .map(|table| table.name.as_str())
        .filter(|name| *name != "alarms")
        .collect();
    assert!(
        changed.is_empty(),
        "the configuration did not change between the snapshot and the rollback: {check:?}"
    );
    println!(
        "snapshots: rolled back to #{}; restore snapshot #{} matches it (no DELETE route: #{} and #{} remain)",
        created.id, restore.id, created.id, restore.id
    );
}
