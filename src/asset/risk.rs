use crate::asset::models::Asset;
use crate::asset::storage::AssetStorage;

/// Compute a 0-100 risk score for an asset based on four weighted factors:
///
/// - Firmware outdated (+30)
/// - No maintenance in 12+ months (+20)
/// - Critical asset unmonitored (+25)
/// - Default credentials tag (+25)
pub async fn compute_risk_score(storage: &AssetStorage, asset: &Asset) -> i64 {
    let mut score: i64 = 0;

    // Factor 1: Firmware outdated (+30)
    if let (Some(vendor), Some(model), Some(firmware)) =
        (&asset.vendor, &asset.model, &asset.firmware_version)
        && let Ok(Some(latest)) = storage.get_latest_firmware(vendor, model).await
        && *firmware != latest
    {
        score += 30;
    }

    // Factor 2: No maintenance in 12+ months (+20)
    match &asset.last_maintenance {
        None => score += 20,
        Some(date_str) => {
            // Parse as ISO 8601 and check if >365 days ago
            if let Ok(parsed) = time::OffsetDateTime::parse(
                date_str,
                &time::format_description::well_known::Iso8601::DEFAULT,
            ) {
                let now = time::OffsetDateTime::now_utc();
                let days_since = (now - parsed).whole_days();
                if days_since > 365 {
                    score += 20;
                }
            } else {
                // Can't parse, treat as no maintenance
                score += 20;
            }
        }
    }

    // Factor 3: Critical asset unmonitored (+25)
    if asset.criticality == "critical" && asset.device_ref.is_none() {
        score += 25;
    }

    // Factor 4: Default credentials (+25) — detected via tag
    if asset.tags.iter().any(|t| t == "default-credentials") {
        score += 25;
    }

    score.min(100)
}

/// Recompute the risk score for a single asset and persist it.
pub async fn recompute_and_update(storage: &AssetStorage, asset_id: &str) -> anyhow::Result<()> {
    if let Some(asset) = storage.get_asset(asset_id).await? {
        let score = compute_risk_score(storage, &asset).await;
        storage.update_risk_score(asset_id, score).await?;
    }
    Ok(())
}
