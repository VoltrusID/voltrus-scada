//! REST API for the Voltrus SCADA phone app.
//!
//! A phone pairs in one of two ways and then holds a device token
//! (`Authorization: Bearer vdt_…`) that acts as the user who paired it:
//!
//! * **QR code.** A signed-in user opens "Connect phone" in the web HMI
//!   (`POST /api/v1/mobile/pairings`), the phone scans the code and trades it
//!   for a token (`POST /api/v1/mobile/pair`, no sign-in). Works through the
//!   push relay too, when the phone cannot reach the site directly.
//! * **Sign-in.** On the site's network the app signs in like the web HMI
//!   (`POST /api/v1/auth/login`, so lockouts and password rules apply) and
//!   then asks for a token with that session (`POST /api/v1/mobile/devices`).
//!
//! With the token the app uses the normal alarm routes (list, acknowledge),
//! plus `GET /api/v1/mobile/summary` for the site screen. See docs/mobile.md.

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::auth_middleware::{AuthCredential, MobileDeviceAuth};
use super::{AppError, AppState, AuthUser};
use crate::mobile::{MobileSettings, PRIORITIES};
use crate::storage::{MobileDevice, MobileDeviceUpdate, NewMobileDevice, PairError};

/// How long a pairing QR code works.
pub const PAIRING_TTL_SECS: i64 = 10 * 60;

/// How long a phone paired with a shared demo account works.
const SHARED_ACCOUNT_DEVICE_SECS: i64 = 7 * 24 * 3600;

/// Scheme of the link in a pairing QR code. The app registers it, so a phone
/// camera opens the app too.
pub const PAIR_LINK_PREFIX: &str = "voltrus-scada://pair?d=";

/// What a pairing QR code carries (as base64url JSON after [`PAIR_LINK_PREFIX`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct PairingPayload {
    /// Format version.
    pub v: u32,
    pub site: String,
    /// Addresses of the site the phone tries first, in order.
    pub urls: Vec<String>,
    /// The push relay, when the site is connected to one (Integrator and up).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay: Option<PairingRelay>,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct PairingRelay {
    pub url: String,
    pub site_id: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreatePairingRequest {
    /// The address the browser uses for the HMI (`window.location.origin`),
    /// put in the code unless it is a loopback address.
    #[serde(default)]
    pub origin: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PairingResponse {
    pub expires_at: i64,
    pub payload: PairingPayload,
    /// The text to show as a QR code.
    pub link: String,
    /// Push works for this site: licensed and a relay is set up.
    pub push_available: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DeviceInfo {
    /// The phone's name as the user sees it ("Pixel 8 Budi").
    #[serde(default)]
    pub name: String,
    /// `android` or `ios`.
    pub platform: String,
    #[serde(default)]
    pub push_token: Option<String>,
    /// `id` (default) or `en`.
    #[serde(default)]
    pub lang: Option<String>,
    /// `info`, `warning` (default) or `critical`.
    #[serde(default)]
    pub min_priority: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PairRequest {
    pub code: String,
    #[serde(flatten)]
    pub device: DeviceInfo,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PairedResponse {
    /// The device token. Shown once.
    pub token: String,
    pub device: MobileDevice,
    pub username: String,
    pub role: String,
    pub site: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateDeviceRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// A new push token; `null` turns pushes off. Absent leaves it.
    #[serde(default, deserialize_with = "deserialize_some")]
    #[schema(value_type = Option<String>)]
    pub push_token: Option<Option<String>>,
    #[serde(default)]
    pub min_priority: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
}

fn deserialize_some<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize)]
pub struct DeviceListQuery {
    /// Every user's phones (admins only).
    #[serde(default)]
    pub all: bool,
}

fn check_platform(platform: &str) -> Result<(), AppError> {
    if matches!(platform, "android" | "ios") {
        Ok(())
    } else {
        Err(AppError::bad_request("platform must be android or ios"))
    }
}

fn check_priority(priority: &str) -> Result<(), AppError> {
    if PRIORITIES.contains(&priority) {
        Ok(())
    } else {
        Err(AppError::bad_request(
            "min_priority must be info, warning or critical",
        ))
    }
}

fn check_lang(lang: &str) -> Result<(), AppError> {
    if matches!(lang, "id" | "en") {
        Ok(())
    } else {
        Err(AppError::bad_request("lang must be id or en"))
    }
}

/// Expo push tokens look like `ExponentPushToken[…]`; a raw FCM or APNs token
/// is a long opaque string. Only the length and characters are checked.
fn check_push_token(token: &str) -> Result<(), AppError> {
    let ok =
        !token.is_empty() && token.len() <= 4096 && token.chars().all(|c| c.is_ascii_graphic());
    if ok {
        Ok(())
    } else {
        Err(AppError::bad_request("push_token is not a push token"))
    }
}

fn clean_name(name: &str) -> String {
    name.trim().chars().take(80).collect()
}

impl DeviceInfo {
    fn checked(self, username: &str, expires_at: Option<i64>) -> Result<NewMobileDevice, AppError> {
        check_platform(&self.platform)?;
        if let Some(token) = &self.push_token {
            check_push_token(token)?;
        }
        let lang = self.lang.unwrap_or_else(|| "id".to_string());
        check_lang(&lang)?;
        let min_priority = self.min_priority.unwrap_or_else(|| "warning".to_string());
        check_priority(&min_priority)?;
        let name = clean_name(&self.name);
        Ok(NewMobileDevice {
            username: username.to_string(),
            name: if name.is_empty() {
                self.platform.clone()
            } else {
                name
            },
            platform: self.platform,
            push_token: self.push_token.filter(|t| !t.is_empty()),
            min_priority,
            lang,
            expires_at,
        })
    }
}

/// Only a person signed in at the HMI (session, or an add-on edition's sign-in
/// provider) may pair a phone: not an API key, and not another phone.
fn require_person(credential: Option<&AuthCredential>) -> Result<(), AppError> {
    match credential {
        Some(AuthCredential::Session | AuthCredential::Bearer) | None => Ok(()),
        _ => Err(AppError::forbidden(
            "Pair a phone from a signed-in session, not with an API key or another phone",
        )),
    }
}

/// The addresses put in a QR code: the admin's list, then the browser's
/// origin unless it is a loopback address (useless on a phone).
fn pairing_urls(settings: &MobileSettings, origin: Option<&str>) -> Vec<String> {
    let mut urls = settings.public_urls.clone();
    if let Some(origin) = origin.map(|o| o.trim().trim_end_matches('/'))
        && let Ok(parsed) = reqwest::Url::parse(origin)
        && matches!(parsed.scheme(), "http" | "https")
        && let Some(host) = parsed.host_str()
    {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !loopback && !urls.iter().any(|u| u == origin) {
            urls.push(origin.to_string());
        }
    }
    urls
}

fn encode_link(payload: &PairingPayload) -> String {
    use base64::Engine;
    let json = serde_json::to_vec(payload).unwrap_or_default();
    format!(
        "{PAIR_LINK_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    )
}

/// The relay to put in a QR code: only when the tier includes `mobile_push`.
fn licensed_relay(state: &AppState) -> Option<crate::edition::MobileRelay> {
    if !state.tier().mobile_push() {
        return None;
    }
    crate::edition::mobile_push_sender()?.relay()
}

async fn site_name(state: &AppState, settings: &MobileSettings) -> String {
    let license = state
        .license_status
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    crate::mobile::site_name(&state.storage, settings, &license).await
}

/// Phones of shared demo accounts expire on their own, so the device limit
/// does not fill up with visitors' phones.
fn device_expiry(state: &AppState, user: &AuthUser) -> Option<i64> {
    state
        .public_demo
        .is_shared(&user.username, &user.role)
        .then(|| crate::timeutil::now_secs() + SHARED_ACCOUNT_DEVICE_SECS)
}

async fn pair_device(
    state: &AppState,
    user: &AuthUser,
    info: DeviceInfo,
    how: &str,
) -> Result<axum::response::Response, AppError> {
    let new = info.checked(&user.username, device_expiry(state, user))?;
    let (device, token) = match state.storage.create_mobile_device(new).await? {
        Ok(paired) => paired,
        Err(PairError::LimitReached) => {
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!(
                        "This site already has {} paired phones. Remove one under Profile, Phones, first.",
                        crate::storage::MOBILE_DEVICE_LIMIT
                    ),
                    "code": "device_limit",
                })),
            )
                .into_response());
        }
    };
    state.audit_logger.log(crate::security::AuditEvent {
        action: "mobile_pair".to_string(),
        username: user.username.clone(),
        resource: format!("mobile_device:{}", device.id),
        details: Some(format!(
            "name={}, platform={}, via={how}",
            device.name, device.platform
        )),
    });
    let settings = MobileSettings::load(&state.storage).await?;
    Ok((
        StatusCode::CREATED,
        Json(PairedResponse {
            token,
            device,
            username: user.username.clone(),
            role: user.role.clone(),
            site: site_name(state, &settings).await,
        }),
    )
        .into_response())
}

/// POST /api/v1/mobile/pairings — a one-time QR code to pair a phone.
#[utoipa::path(
    post,
    path = "/api/v1/mobile/pairings",
    request_body = CreatePairingRequest,
    responses((status = 200, body = PairingResponse), (status = 403)),
    tag = "Mobile",
    security(("cookie_auth" = []))
)]
pub async fn create_pairing(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    credential: Option<Extension<AuthCredential>>,
    Json(body): Json<CreatePairingRequest>,
) -> Result<impl IntoResponse, AppError> {
    require_person(credential.as_ref().map(|Extension(c)| c))?;
    let settings = MobileSettings::load(&state.storage).await?;
    let code = state
        .storage
        .create_mobile_pairing(&user.username, PAIRING_TTL_SECS)
        .await?;
    let relay = licensed_relay(&state);
    let payload = PairingPayload {
        v: 1,
        site: site_name(&state, &settings).await,
        urls: pairing_urls(&settings, body.origin.as_deref()),
        relay: relay.as_ref().map(|r| PairingRelay {
            url: r.url.clone(),
            site_id: r.site_id.clone(),
        }),
        code,
    };
    Ok(Json(PairingResponse {
        expires_at: crate::timeutil::now_secs() + PAIRING_TTL_SECS,
        link: encode_link(&payload),
        payload,
        push_available: relay.is_some(),
    }))
}

/// POST /api/v1/mobile/pair — trade a QR code for a device token. No sign-in:
/// the code is the proof. A code works once and for ten minutes.
#[utoipa::path(
    post,
    path = "/api/v1/mobile/pair",
    request_body = PairRequest,
    responses(
        (status = 201, body = PairedResponse),
        (status = 401, description = "Unknown, used or expired code"),
        (status = 409, description = "The site has no room for another phone"),
    ),
    tag = "Mobile",
)]
pub async fn pair_with_code(
    State(state): State<AppState>,
    Extension(crate::security::ClientIp(client_ip)): Extension<crate::security::ClientIp>,
    Json(body): Json<PairRequest>,
) -> Result<axum::response::Response, AppError> {
    // Check the request before the code is used up.
    check_platform(&body.device.platform)?;
    let unauthorized = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "This pairing code is not valid any more. Show a new QR code in the HMI.",
                "code": "pairing_invalid",
            })),
        )
            .into_response()
    };
    let code = body.code.trim();
    if code.is_empty() || code.len() > 128 {
        return Ok(unauthorized());
    }
    let Some(username) = state.storage.take_mobile_pairing(code).await? else {
        state
            .intrusion_detector
            .record_401_with_storage(&client_ip, &state.storage)
            .await;
        return Ok(unauthorized());
    };
    let Some(user) = state.storage.get_user_by_username(&username).await? else {
        return Ok(unauthorized());
    };
    if user.disabled || super::auth_middleware::password_change_pending(&state, &user) {
        return Ok(unauthorized());
    }
    let user = AuthUser {
        username: user.username,
        role: user.role,
    };
    pair_device(&state, &user, body.device, "qr").await
}

/// POST /api/v1/mobile/devices — a device token for the phone signed in with
/// this session (the app's sign-in on the site's network).
#[utoipa::path(
    post,
    path = "/api/v1/mobile/devices",
    request_body = DeviceInfo,
    responses((status = 201, body = PairedResponse), (status = 409)),
    tag = "Mobile",
    security(("cookie_auth" = []))
)]
pub async fn create_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    credential: Option<Extension<AuthCredential>>,
    Json(body): Json<DeviceInfo>,
) -> Result<axum::response::Response, AppError> {
    require_person(credential.as_ref().map(|Extension(c)| c))?;
    pair_device(&state, &user, body, "sign-in").await
}

/// GET /api/v1/mobile/devices — the caller's paired phones (`?all=true`: every
/// user's, admins only).
#[utoipa::path(
    get,
    path = "/api/v1/mobile/devices",
    params(("all" = Option<bool>, Query, description = "Every user's phones (admins)")),
    responses((status = 200, body = Vec<MobileDevice>)),
    tag = "Mobile",
    security(("cookie_auth" = []))
)]
pub async fn list_devices(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<DeviceListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let devices = if query.all {
        if user.role != "admin" {
            return Err(AppError::forbidden("admin access required"));
        }
        state.storage.list_mobile_devices(None).await?
    } else {
        state
            .storage
            .list_mobile_devices(Some(&user.username))
            .await?
    };
    Ok(Json(devices))
}

/// DELETE /api/v1/mobile/devices/{id} — unpair a phone: one's own, or any (admins).
#[utoipa::path(
    delete,
    path = "/api/v1/mobile/devices/{id}",
    params(("id" = i64, Path, description = "Device id")),
    responses((status = 200), (status = 404)),
    tag = "Mobile",
    security(("cookie_auth" = []))
)]
pub async fn delete_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let Some(device) = state.storage.get_mobile_device(id).await? else {
        return Err(AppError::not_found("no such phone"));
    };
    if device.username != user.username && user.role != "admin" {
        // Do not tell other users' phones apart from missing ones.
        return Err(AppError::not_found("no such phone"));
    }
    unpair(&state, &user, &device).await?;
    Ok(Json(json!({"ok": true})))
}

async fn unpair(state: &AppState, user: &AuthUser, device: &MobileDevice) -> Result<(), AppError> {
    state.storage.delete_mobile_device(device.id).await?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "mobile_unpair".to_string(),
        username: user.username.clone(),
        resource: format!("mobile_device:{}", device.id),
        details: Some(format!("name={}, owner={}", device.name, device.username)),
    });
    Ok(())
}

fn this_device(device: Option<Extension<MobileDeviceAuth>>) -> Result<i64, AppError> {
    device
        .map(|Extension(d)| d.id)
        .ok_or_else(|| AppError::bad_request("this route is for a paired phone (device token)"))
}

/// GET /api/v1/mobile/device — the calling phone.
#[utoipa::path(
    get,
    path = "/api/v1/mobile/device",
    responses((status = 200, body = MobileDevice)),
    tag = "Mobile",
    security(("device_token" = []))
)]
pub async fn get_this_device(
    State(state): State<AppState>,
    device: Option<Extension<MobileDeviceAuth>>,
) -> Result<impl IntoResponse, AppError> {
    let id = this_device(device)?;
    let device = state
        .storage
        .get_mobile_device(id)
        .await?
        .ok_or_else(|| AppError::not_found("no such phone"))?;
    Ok(Json(device))
}

/// PATCH /api/v1/mobile/device — change the calling phone's push token,
/// lowest pushed priority, language or name.
#[utoipa::path(
    patch,
    path = "/api/v1/mobile/device",
    request_body = UpdateDeviceRequest,
    responses((status = 200, body = MobileDevice)),
    tag = "Mobile",
    security(("device_token" = []))
)]
pub async fn update_this_device(
    State(state): State<AppState>,
    device: Option<Extension<MobileDeviceAuth>>,
    Json(body): Json<UpdateDeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let id = this_device(device)?;
    if let Some(Some(token)) = &body.push_token {
        check_push_token(token)?;
    }
    if let Some(p) = &body.min_priority {
        check_priority(p)?;
    }
    if let Some(l) = &body.lang {
        check_lang(l)?;
    }
    let update = MobileDeviceUpdate {
        name: body
            .name
            .as_deref()
            .map(clean_name)
            .filter(|n| !n.is_empty()),
        push_token: body.push_token.map(|t| t.filter(|t| !t.is_empty())),
        min_priority: body.min_priority,
        lang: body.lang,
    };
    let device = state
        .storage
        .update_mobile_device(id, update)
        .await?
        .ok_or_else(|| AppError::not_found("no such phone"))?;
    Ok(Json(device))
}

/// DELETE /api/v1/mobile/device — the calling phone unpairs itself ("remove
/// site" in the app). Its token stops working at once.
#[utoipa::path(
    delete,
    path = "/api/v1/mobile/device",
    responses((status = 200)),
    tag = "Mobile",
    security(("device_token" = []))
)]
pub async fn delete_this_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    device: Option<Extension<MobileDeviceAuth>>,
) -> Result<impl IntoResponse, AppError> {
    let id = this_device(device)?;
    if let Some(device) = state.storage.get_mobile_device(id).await? {
        unpair(&state, &user, &device).await?;
    }
    Ok(Json(json!({"ok": true})))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PinnedTag {
    pub key: String,
    pub name: String,
    pub unit: String,
    /// `null` when there is no value (never read, or not a number).
    pub value: Option<f64>,
    #[schema(value_type = String)]
    pub quality: crate::quality::Quality,
    pub ts_ms: Option<i64>,
    /// `normal`, `warning` or `critical`, from the tag's alarm limits.
    pub status: String,
}

/// GET /api/v1/mobile/summary — what the phone's site screen shows: the site,
/// its branding, the caller's rights, alarm counts and the pinned tags.
#[utoipa::path(
    get,
    path = "/api/v1/mobile/summary",
    responses((status = 200, body = serde_json::Value)),
    tag = "Mobile",
    security(("device_token" = []), ("cookie_auth" = []))
)]
pub async fn summary(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let settings = MobileSettings::load(&state.storage).await?;
    let license = state
        .license_status
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let site = crate::mobile::site_name(&state.storage, &settings, &license).await;

    let branding = if license.white_label() {
        let rows = state
            .storage
            .get_settings(&["brand_title", "brand_logo", "brand_accent"])
            .await?;
        let get = |k: &str| {
            rows.iter()
                .find(|s| s.key == k)
                .map(|s| s.value.clone())
                .unwrap_or_default()
        };
        let accent = if license.tier().theme_full() {
            get("brand_accent")
        } else {
            String::new()
        };
        json!({"title": get("brand_title"), "logo": get("brand_logo"), "accent": accent})
    } else {
        json!({"title": "", "logo": "", "accent": ""})
    };

    let active = state.storage.list_alarms(Some("active"), 500).await?;
    let count = |p: &str| active.iter().filter(|a| a.priority == p).count();
    let unresolved = state.storage.count_active_alarms().await?;

    let sources = state.storage.list_data_sources(None).await?;
    let pinned_sources: Vec<_> = settings
        .pinned_tags
        .iter()
        .filter_map(|key| sources.iter().find(|s| &s.key == key).cloned())
        .collect();
    let cache = state.cache.get_all().await;
    let values = super::data::evaluate_data_sources(
        &pinned_sources,
        &cache,
        crate::timeutil::now_secs() as f64,
    );
    let pinned: Vec<PinnedTag> = pinned_sources
        .iter()
        .zip(values)
        .map(|(ds, v)| PinnedTag {
            key: ds.key.clone(),
            name: ds.name.clone(),
            unit: ds.unit.clone(),
            value: v.value.is_finite().then_some(v.value),
            quality: v.quality,
            ts_ms: v.ts_ms,
            status: v.status,
        })
        .collect();

    let relay = licensed_relay(&state);
    Ok(Json(json!({
        "site": site,
        "branding": branding,
        "user": {
            "username": user.username,
            "role": user.role,
            "can_acknowledge": super::role_allows_operation(&user.role),
        },
        "push": {
            "licensed": license.tier().mobile_push(),
            "relay_connected": relay.as_ref().is_some_and(|r| r.connected),
            "details": settings.push_details,
        },
        "alarms": {
            "unacknowledged": active.len(),
            "unresolved": unresolved,
            "critical": count("critical"),
            "warning": count("warning"),
            "info": count("info"),
        },
        "pinned": pinned,
        "server_time": crate::timeutil::now_secs(),
    })))
}

/// GET /api/v1/mobile/settings — the phone app settings of the site.
#[utoipa::path(
    get,
    path = "/api/v1/mobile/settings",
    responses((status = 200, body = MobileSettings)),
    tag = "Mobile",
    security(("cookie_auth" = []))
)]
pub async fn get_settings(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let settings = MobileSettings::load(&state.storage).await?;
    let relay = licensed_relay(&state);
    Ok(Json(json!({
        "site_name": settings.site_name,
        "push_details": settings.push_details,
        "pinned_tags": settings.pinned_tags,
        "public_urls": settings.public_urls,
        "effective_site_name": site_name(&state, &settings).await,
        "push_licensed": state.tier().mobile_push(),
        "relay": relay,
        "device_limit": crate::storage::MOBILE_DEVICE_LIMIT,
    })))
}

/// PUT /api/v1/mobile/settings — change them (configuration roles).
#[utoipa::path(
    put,
    path = "/api/v1/mobile/settings",
    request_body = MobileSettings,
    responses((status = 200, body = MobileSettings), (status = 400)),
    tag = "Mobile",
    security(("cookie_auth" = []))
)]
pub async fn put_settings(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<MobileSettings>,
) -> Result<impl IntoResponse, AppError> {
    let settings = body.validated().map_err(AppError::bad_request)?;
    settings.save(&state.storage).await?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "mobile_settings".to_string(),
        username: user.username.clone(),
        resource: "settings:mobile".to_string(),
        details: Some(format!(
            "site_name={:?}, push_details={}, pinned_tags={}, public_urls={}",
            settings.site_name,
            settings.push_details,
            settings.pinned_tags.len(),
            settings.public_urls.len()
        )),
    });
    Ok(Json(settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_origins_stay_out_of_the_qr_code() {
        let settings = MobileSettings {
            public_urls: vec!["https://scada.example.com".into()],
            ..Default::default()
        };
        assert_eq!(
            pairing_urls(&settings, Some("http://localhost:3000")),
            ["https://scada.example.com"]
        );
        assert_eq!(
            pairing_urls(&settings, Some("http://127.0.0.1:3000/")),
            ["https://scada.example.com"]
        );
        assert_eq!(
            pairing_urls(&settings, Some("http://[::1]:3000")),
            ["https://scada.example.com"]
        );
        assert_eq!(
            pairing_urls(&settings, Some("http://192.168.1.20:3000/")),
            ["https://scada.example.com", "http://192.168.1.20:3000"]
        );
        assert_eq!(
            pairing_urls(&settings, Some("https://scada.example.com")),
            ["https://scada.example.com"]
        );
        assert_eq!(
            pairing_urls(&settings, Some("javascript:alert(1)")).len(),
            1
        );
    }

    #[test]
    fn the_link_carries_the_payload_as_base64url_json() {
        use base64::Engine;
        let payload = PairingPayload {
            v: 1,
            site: "Pabrik A".into(),
            urls: vec!["http://10.0.0.5:3000".into()],
            relay: None,
            code: "abc".into(),
        };
        let link = encode_link(&payload);
        let data = link.strip_prefix(PAIR_LINK_PREFIX).unwrap();
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(data)
            .unwrap();
        let back: PairingPayload = serde_json::from_slice(&json).unwrap();
        assert_eq!(back, payload);
    }
}
