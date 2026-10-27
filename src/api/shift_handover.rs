use axum::extract::{Extension, Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
use crate::models::*;

// ── Query params ──

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListShiftHandoversQuery {
    pub shift_date: Option<String>,
    pub shift_period: Option<String>,
}

// ── Helpers ──

/// The shift in progress at `now` (UTC) as (shift date, period).
/// Morning: 06:00-14:00, Afternoon: 14:00-22:00, Night: 22:00-06:00 the next
/// day. The night shift belongs to the date it started on, so between 00:00
/// and 06:00 it is yesterday's night shift that is in progress.
fn shift_at(now: time::OffsetDateTime) -> (String, String) {
    let hour = now.hour();
    let today = now.date();
    let (date, period) = if hour < 6 {
        (today.previous_day().unwrap_or(today), "night")
    } else if hour < 14 {
        (today, "morning")
    } else if hour < 22 {
        (today, "afternoon")
    } else {
        (today, "night")
    };
    (
        format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            date.month() as u8,
            date.day()
        ),
        period.to_string(),
    )
}

/// The current shift period and date on the server's UTC clock.
fn current_shift_period() -> (String, String) {
    shift_at(time::OffsetDateTime::now_utc())
}

/// Convert a shift date and period to its [start, end) range in unix seconds (UTC).
fn shift_time_range(shift_date: &str, shift_period: &str) -> (Option<i64>, Option<i64>) {
    use time::OffsetDateTime;

    // Parse shift_date (YYYY-MM-DD) into a rough unix range
    let parts: Vec<u32> = shift_date
        .split('-')
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() != 3 {
        return (None, None);
    }

    // Build a naive date; use time crate for proper conversion
    let year = parts[0] as i32;
    let month = time::Month::try_from(parts[1] as u8).ok();
    let day = parts[2] as u8;

    let (month, day) = match (month, day) {
        (Some(m), d) if (1..=31).contains(&d) => (m, d),
        _ => return (None, None),
    };

    let date = time::Date::from_calendar_date(year, month, day).ok();
    let date = match date {
        Some(d) => d,
        None => return (None, None),
    };

    let (start_hour, end_hour, next_day) = match shift_period {
        "morning" => (6, 14, false),
        "afternoon" => (14, 22, false),
        "night" => (22, 6, true), // ends 06:00 next day
        _ => return (None, None),
    };

    let start_time = date.with_hms(start_hour, 0, 0).ok();
    let end_date = if next_day {
        date.next_day().unwrap_or(date)
    } else {
        date
    };
    let end_time = end_date.with_hms(end_hour, 0, 0).ok();

    let from_ts = start_time.map(|t| {
        let dt: OffsetDateTime = t.assume_utc();
        dt.unix_timestamp()
    });
    let to_ts = end_time.map(|t| {
        let dt: OffsetDateTime = t.assume_utc();
        dt.unix_timestamp()
    });

    (from_ts, to_ts)
}

// ── Handlers ──

#[utoipa::path(
    post,
    path = "/api/v1/shift-handover",
    request_body = crate::models::CreateShiftHandoverRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 500),
    ),
    tag = "Shift Handover",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_shift_handover(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreateShiftHandoverRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Resolve author user ID from authenticated username
    let author = state
        .storage
        .get_user_by_username(&auth_user.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Authenticated user not found"))?;
    let author_id = author.id;

    // Create the shift handover log
    let handover_id = state.storage.create_shift_handover(&req, author_id).await?;

    // Parse @mentions from all three text fields and create notifications
    let fields: Vec<&str> = vec![
        req.what_happened.as_deref().unwrap_or(""),
        req.whats_pending.as_deref().unwrap_or(""),
        req.what_to_watch.as_deref().unwrap_or(""),
    ];
    let combined = fields.join(" ");
    let mentions = parse_mentions(&combined);
    for username in &mentions {
        if let Ok(Some(mentioned_user)) = state.storage.get_user_by_username(username).await {
            // Don't notify yourself
            if mentioned_user.id != author_id {
                let _ = state
                    .storage
                    .create_notification(&CreateNotificationRequest {
                        user_id: mentioned_user.id,
                        from_user_id: author_id,
                        notification_type: "shift_handover".to_string(),
                        message: format!(
                            "You were mentioned in a shift handover log for {} ({})",
                            req.shift_date, req.shift_period
                        ),
                        reference_id: Some(handover_id),
                    })
                    .await;
            }
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": handover_id, "ok": true })),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/shift-handover",
    params(ListShiftHandoversQuery),
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Shift Handover",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_shift_handovers(
    State(state): State<AppState>,
    Query(params): Query<ListShiftHandoversQuery>,
) -> Result<impl IntoResponse, AppError> {
    let handovers = state
        .storage
        .list_shift_handovers(params.shift_date.as_deref(), params.shift_period.as_deref())
        .await?;

    Ok(Json(json!({
        "handovers": handovers,
        "count": handovers.len(),
    })))
}

#[utoipa::path(
    get,
    path = "/api/v1/shift-handover/{id}",
    params(
        ("id" = i64, Path, description = "Shift handover ID")
    ),
    responses(
        (status = 200, body = crate::models::ShiftHandover),
        (status = 404),
    ),
    tag = "Shift Handover",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_shift_handover(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let handover = state
        .storage
        .get_shift_handover(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Shift handover not found"))?;

    Ok(Json(json!(handover)))
}

#[utoipa::path(
    get,
    path = "/api/v1/shift-handover/auto-report",
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Shift Handover",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn auto_shift_report(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let (shift_date, shift_period) = current_shift_period();
    let (from_ts, to_ts) = shift_time_range(&shift_date, &shift_period);

    // Alarms raised during the shift
    let alarms = match (from_ts, to_ts) {
        (Some(from), Some(to)) => state.storage.list_alarms_between(from, to).await?,
        _ => Vec::new(),
    };

    // Fetch annotations for this shift period
    let annotations = state.storage.list_annotations(None, from_ts, to_ts).await?;

    // Fetch existing handover logs for this shift
    let handovers = state
        .storage
        .list_shift_handovers(Some(&shift_date), Some(&shift_period))
        .await?;

    // Fetch security events for this shift period
    let security_events = state
        .storage
        .list_security_events(None, None, from_ts, to_ts, 100, 0)
        .await?;

    Ok(Json(json!({
        "shift_date": shift_date,
        "shift_period": shift_period,
        "period_start": from_ts,
        "period_end": to_ts,
        "alarms": alarms,
        "alarm_count": alarms.len(),
        "annotations": annotations,
        "annotation_count": annotations.len(),
        "handovers": handovers,
        "handover_count": handovers.len(),
        "security_events": security_events,
        "security_event_count": security_events.len(),
    })))
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_shift_period_format() {
        let (date, period) = current_shift_period();
        // Date should be YYYY-MM-DD
        assert_eq!(date.len(), 10);
        assert!(date.contains('-'));
        // Period should be one of the three
        assert!(
            period == "morning" || period == "afternoon" || period == "night",
            "Unexpected period: {}",
            period
        );
    }

    fn at(date: &str, hour: u8, min: u8) -> time::OffsetDateTime {
        let parts: Vec<u32> = date.split('-').map(|p| p.parse().unwrap()).collect();
        time::Date::from_calendar_date(
            parts[0] as i32,
            time::Month::try_from(parts[1] as u8).unwrap(),
            parts[2] as u8,
        )
        .unwrap()
        .with_hms(hour, min, 0)
        .unwrap()
        .assume_utc()
    }

    /// SCADA-16 S4: between 00:00 and 06:00 the report named today's night
    /// shift (22:00 tonight to 06:00 tomorrow), a window in the future.
    #[test]
    fn test_shift_at_after_midnight_is_yesterdays_night() {
        assert_eq!(
            shift_at(at("2026-05-04", 0, 0)),
            ("2026-05-03".to_string(), "night".to_string())
        );
        assert_eq!(
            shift_at(at("2026-05-04", 5, 59)),
            ("2026-05-03".to_string(), "night".to_string())
        );
        assert_eq!(
            shift_at(at("2026-05-04", 6, 0)),
            ("2026-05-04".to_string(), "morning".to_string())
        );
        assert_eq!(
            shift_at(at("2026-05-04", 14, 0)),
            ("2026-05-04".to_string(), "afternoon".to_string())
        );
        assert_eq!(
            shift_at(at("2026-05-04", 22, 0)),
            ("2026-05-04".to_string(), "night".to_string())
        );
        // A month boundary: 2026-06-01 03:00 belongs to May 31st's night.
        assert_eq!(
            shift_at(at("2026-06-01", 3, 0)),
            ("2026-05-31".to_string(), "night".to_string())
        );
    }

    /// The window of the shift in progress always contains the moment itself.
    #[test]
    fn test_shift_window_contains_now_at_every_hour() {
        for hour in 0..24 {
            for min in [0, 30, 59] {
                let now = at("2026-05-04", hour, min);
                let (date, period) = shift_at(now);
                let (from, to) = shift_time_range(&date, &period);
                let ts = now.unix_timestamp();
                assert!(
                    from.unwrap() <= ts && ts < to.unwrap(),
                    "{hour}:{min:02} in {date} {period}"
                );
            }
        }
    }

    #[test]
    fn test_shift_time_range_morning() {
        let (from, to) = shift_time_range("2026-05-03", "morning");
        assert!(from.is_some());
        assert!(to.is_some());
        // Morning is 06:00-14:00, so the range is 8 hours = 28800 seconds
        let diff = to.unwrap() - from.unwrap();
        assert_eq!(diff, 8 * 3600);
    }

    #[test]
    fn test_shift_time_range_afternoon() {
        let (from, to) = shift_time_range("2026-05-03", "afternoon");
        assert!(from.is_some());
        assert!(to.is_some());
        let diff = to.unwrap() - from.unwrap();
        assert_eq!(diff, 8 * 3600);
    }

    #[test]
    fn test_shift_time_range_night() {
        let (from, to) = shift_time_range("2026-05-03", "night");
        assert!(from.is_some());
        assert!(to.is_some());
        // Night: 22:00 to 06:00 next day = 8 hours
        // Since we use hour 30 % 24 = 6, the diff from 22 to 30 = 8 hours
        let diff = to.unwrap() - from.unwrap();
        assert_eq!(diff, 8 * 3600);
    }

    #[test]
    fn test_shift_time_range_invalid_date() {
        let (from, to) = shift_time_range("not-a-date", "morning");
        assert!(from.is_none());
        assert!(to.is_none());
    }

    #[test]
    fn test_shift_time_range_invalid_period() {
        let (from, to) = shift_time_range("2026-05-03", "invalid");
        assert!(from.is_none());
        assert!(to.is_none());
    }
}
