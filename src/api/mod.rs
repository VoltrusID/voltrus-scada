mod ai_copilot;
mod alarms;
mod analytics;
mod annotations;
mod api_keys;
mod asset_import;
mod assets;
mod audit_log;
pub mod auth;
mod auth_middleware;
#[cfg(feature = "bacnet")]
mod bacnet_devices;
mod commands;
mod compression;
mod config_snapshots;
mod csv_import;
pub(crate) mod data;
mod data_sources;
#[cfg(feature = "dnp3")]
mod dnp3_devices;
mod edr_compat;
#[cfg(feature = "ethernet-ip")]
mod enip_devices;
mod excel_import;
mod firmware_catalog;
mod flows;
mod generators;
pub(crate) mod hardware_cert;
mod import_nodered;
mod integration;
mod layouts;
mod license;
mod maintenance;
#[cfg(feature = "modbus")]
mod modbus_devices;
#[cfg(feature = "mqtt-broker")]
mod mqtt_broker;
mod naming_convention;
mod nis2_export;
mod notifications;
mod oee;
#[cfg(feature = "opcua")]
mod opcua_devices;
#[cfg(feature = "openapi")]
pub(crate) mod openapi;
mod rbac;
mod recipes;
#[cfg(feature = "s7")]
mod s7_devices;
mod screens;
#[cfg(feature = "scripts")]
mod scripts;
mod settings;
mod shift_handover;
mod shortcuts;
mod simulation;
mod smtp_settings;
#[cfg(feature = "snmp_scan")]
mod snmp_scan;
pub(crate) mod stream;
mod system;
mod system_docs;
mod tag_tree;
mod user_notifications;
mod vertical_templates;
mod voltrus_plc_devices;
mod websocket;
mod wizard;

use axum::extract::DefaultBodyLimit;
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{
        HeaderValue, Method, StatusCode,
        header::{self, CONTENT_TYPE, HeaderName},
    },
    middleware::{self, Next},
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
};
use serde_json::json;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::cache::DeviceCache;
use crate::poller_manager::PollerManager;
use crate::storage::Storage;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub cache: DeviceCache,
    pub storage: Storage,
    pub poller_manager: PollerManager,
    pub started_at: std::time::Instant,
    pub auth_secret: String,
    pub cors_origin: Option<String>,
    pub cookie_secure: bool,
    pub retention_config: crate::models::RetentionConfig,
    pub flow_engine: Option<Arc<crate::flow_engine::engine::FlowEngine>>,
    pub login_rate_limiter: crate::security::LoginRateLimiter,
    pub global_rate_limiter: crate::security::GlobalRateLimiter,
    pub audit_logger: crate::security::AuditLogger,
    pub intrusion_detector: crate::security::IntrusionDetector,
    pub license_status: Arc<std::sync::RwLock<crate::license::LicenseStatus>>,
    /// Add-on edition hooks. [`crate::edition::Community`] in the open core.
    pub ext: Arc<dyn crate::edition::Extension>,
    pub smtp_config: Arc<tokio::sync::RwLock<Option<crate::models::SmtpConfig>>>,
    pub config_path: String,
    pub ws_manager: Arc<crate::ws_manager::WsManager>,
    #[cfg(feature = "mqtt-broker")]
    pub mqtt_broker_state: Option<crate::mqtt_broker::MqttBrokerState>,
    #[cfg(not(feature = "mqtt-broker"))]
    pub mqtt_broker_state: Option<()>,
    pub network_manager: crate::network::NetworkManager,
    pub health_monitor: Arc<crate::health::watchdog::HealthMonitor>,
    pub widget_registry: crate::widget_sdk::WidgetRegistry,
    #[cfg(feature = "modbus")]
    pub modbus_pool: std::sync::Arc<crate::modbus::ModbusPool>,
}

#[derive(Debug)]
pub struct AppError(pub anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("{:#}", self.0)})),
        )
            .into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self {
        AppError(err.into())
    }
}

impl AppState {
    /// The tier currently unlocked by the licence, read live so an activation
    /// or renewal takes effect without a restart. Always Community in the core.
    pub fn tier(&self) -> crate::license::LicenseTier {
        self.license_status
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .tier()
    }

    /// Whether white-label branding is unlocked (tier, or an add-on edition's
    /// logo-only grant). Read live like [`AppState::tier`].
    pub fn white_label(&self) -> bool {
        self.license_status
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .white_label()
    }
}

/// Reports the active tier in an `x-license-tier` response header.
async fn edition_header_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> impl IntoResponse {
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(
        HeaderName::from_static("x-license-tier"),
        HeaderValue::from_static(state.tier().as_str()),
    );
    resp
}

// --- Protocol route helper functions (cfg-gated) ---

#[cfg(feature = "modbus")]
fn modbus_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/modbus-devices",
            get(modbus_devices::list_modbus_devices),
        )
        .route(
            "/api/v1/modbus-devices/status",
            get(modbus_devices::get_poller_statuses),
        )
        .route(
            "/api/v1/modbus-devices/{id}",
            get(modbus_devices::get_modbus_device),
        )
}

#[cfg(feature = "modbus")]
fn modbus_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/modbus-devices",
            post(modbus_devices::create_modbus_device),
        )
        .route(
            "/api/v1/modbus-devices/scan",
            post(modbus_devices::scan_modbus_registers),
        )
        .route(
            "/api/v1/modbus-devices/scan-network",
            post(modbus_devices::scan_network),
        )
        .route(
            "/api/v1/modbus-devices/{id}",
            put(modbus_devices::update_modbus_device).delete(modbus_devices::delete_modbus_device),
        )
        .route(
            "/api/v1/modbus-devices/{id}/registers",
            put(modbus_devices::replace_registers),
        )
        .route(
            "/api/v1/modbus-devices/{id}/restart",
            post(modbus_devices::restart_poller),
        )
}

#[cfg(feature = "opcua")]
fn opcua_admin_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/opcua/devices", post(opcua_devices::create_device))
        .route(
            "/api/v1/opcua/devices/{id}",
            put(opcua_devices::update_device).delete(opcua_devices::delete_device),
        )
        .route(
            "/api/v1/opcua/devices/{id}/nodes",
            get(opcua_devices::list_nodes).post(opcua_devices::add_node),
        )
        .route(
            "/api/v1/opcua/devices/{id}/nodes/{node_id}",
            delete(opcua_devices::remove_node),
        )
        .route(
            "/api/v1/opcua/devices/{id}/browse",
            post(opcua_devices::browse_device),
        )
        .route(
            "/api/v1/opcua/devices/{id}/start",
            post(opcua_devices::start_device),
        )
        .route(
            "/api/v1/opcua/devices/{id}/stop",
            post(opcua_devices::stop_device),
        )
}

#[cfg(feature = "s7")]
fn s7_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/s7/devices",
            get(s7_devices::list_devices).post(s7_devices::create_device),
        )
        .route(
            "/api/v1/s7/devices/{id}",
            put(s7_devices::update_device).delete(s7_devices::delete_device),
        )
        .route(
            "/api/v1/s7/devices/{id}/tags",
            get(s7_devices::list_tags).post(s7_devices::add_tag),
        )
        .route(
            "/api/v1/s7/devices/{id}/tags/{tag_id}",
            delete(s7_devices::remove_tag),
        )
        .route(
            "/api/v1/s7/devices/{id}/browse",
            post(s7_devices::browse_device),
        )
        .route(
            "/api/v1/s7/devices/{id}/start",
            post(s7_devices::start_device),
        )
        .route(
            "/api/v1/s7/devices/{id}/stop",
            post(s7_devices::stop_device),
        )
}

#[cfg(feature = "ethernet-ip")]
fn enip_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/enip/devices",
            get(enip_devices::list_devices).post(enip_devices::create_device),
        )
        .route(
            "/api/v1/enip/devices/{id}",
            put(enip_devices::update_device).delete(enip_devices::delete_device),
        )
        .route(
            "/api/v1/enip/devices/{id}/tags",
            get(enip_devices::list_tags).post(enip_devices::add_tag),
        )
        .route(
            "/api/v1/enip/devices/{id}/tags/{tag_id}",
            delete(enip_devices::remove_tag),
        )
        .route(
            "/api/v1/enip/devices/{id}/browse",
            post(enip_devices::browse_device),
        )
        .route(
            "/api/v1/enip/devices/{id}/start",
            post(enip_devices::start_device),
        )
        .route(
            "/api/v1/enip/devices/{id}/stop",
            post(enip_devices::stop_device),
        )
}

fn voltrus_plc_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/voltrus-plc/devices",
            get(voltrus_plc_devices::list_devices),
        )
        .route(
            "/api/v1/voltrus-plc/devices/{id}",
            get(voltrus_plc_devices::get_device),
        )
}

fn voltrus_plc_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/voltrus-plc/devices",
            post(voltrus_plc_devices::create_device),
        )
        .route(
            "/api/v1/voltrus-plc/devices/{id}",
            put(voltrus_plc_devices::update_device).delete(voltrus_plc_devices::delete_device),
        )
        .route(
            "/api/v1/voltrus-plc/devices/{id}/tags",
            get(voltrus_plc_devices::list_tags).post(voltrus_plc_devices::create_tag),
        )
        .route(
            "/api/v1/voltrus-plc/devices/{id}/tags/{tag_id}",
            delete(voltrus_plc_devices::delete_tag),
        )
        .route(
            "/api/v1/voltrus-plc/devices/{id}/start",
            post(voltrus_plc_devices::start_device),
        )
        .route(
            "/api/v1/voltrus-plc/devices/{id}/stop",
            post(voltrus_plc_devices::stop_device),
        )
        .route(
            "/api/v1/voltrus-plc/devices/{id}/restart",
            post(voltrus_plc_devices::restart_device),
        )
        .route(
            "/api/v1/voltrus-plc/devices/{id}/write",
            post(voltrus_plc_devices::write_variable),
        )
}

#[cfg(feature = "dnp3")]
fn dnp3_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/dnp3/devices",
            get(dnp3_devices::list_devices).post(dnp3_devices::create_device),
        )
        .route(
            "/api/v1/dnp3/devices/{id}",
            get(dnp3_devices::get_device)
                .put(dnp3_devices::update_device)
                .delete(dnp3_devices::delete_device),
        )
        .route(
            "/api/v1/dnp3/devices/{id}/points",
            post(dnp3_devices::add_point),
        )
        .route(
            "/api/v1/dnp3/devices/{id}/points/{point_id}",
            delete(dnp3_devices::remove_point),
        )
        .route("/api/v1/dnp3/scan", post(dnp3_devices::scan_network))
        .route(
            "/api/v1/dnp3/devices/{id}/start",
            post(dnp3_devices::start_device),
        )
        .route(
            "/api/v1/dnp3/devices/{id}/stop",
            post(dnp3_devices::stop_device),
        )
        .route(
            "/api/v1/dnp3/devices/{id}/integrity",
            post(dnp3_devices::trigger_integrity),
        )
        .route(
            "/api/v1/dnp3/devices/{id}/freeze",
            post(dnp3_devices::trigger_freeze),
        )
        .route(
            "/api/v1/dnp3/group-variations",
            get(dnp3_devices::list_group_variations),
        )
}

#[cfg(feature = "bacnet")]
fn bacnet_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/bacnet/devices",
            get(bacnet_devices::list_devices).post(bacnet_devices::create_device),
        )
        .route(
            "/api/v1/bacnet/devices/{id}",
            put(bacnet_devices::update_device).delete(bacnet_devices::delete_device),
        )
        .route(
            "/api/v1/bacnet/devices/{id}/objects",
            get(bacnet_devices::list_objects).post(bacnet_devices::add_object),
        )
        .route(
            "/api/v1/bacnet/devices/{id}/objects/{object_id}",
            delete(bacnet_devices::remove_object),
        )
        .route(
            "/api/v1/bacnet/devices/{id}/browse",
            post(bacnet_devices::browse_device),
        )
        .route(
            "/api/v1/bacnet/devices/{id}/start",
            post(bacnet_devices::start_device),
        )
        .route(
            "/api/v1/bacnet/devices/{id}/stop",
            post(bacnet_devices::stop_device),
        )
        .route(
            "/api/v1/bacnet/discover",
            post(bacnet_devices::discover_network),
        )
}

#[cfg(feature = "mqtt-broker")]
fn mqtt_broker_routes() -> Router<AppState> {
    Router::new().route("/api/v1/mqtt-broker/status", get(mqtt_broker::get_status))
}

#[cfg(feature = "mqtt-broker")]
fn mqtt_broker_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/mqtt-broker/config",
            put(mqtt_broker::update_config),
        )
        .route("/api/v1/mqtt-broker/start", post(mqtt_broker::start_broker))
        .route("/api/v1/mqtt-broker/stop", post(mqtt_broker::stop_broker))
        .route("/api/v1/mqtt-broker/topics", get(mqtt_broker::get_topics))
        .route(
            "/api/v1/mqtt-broker/topics/{topic}/messages",
            get(mqtt_broker::get_topic_messages),
        )
        .route(
            "/api/v1/mqtt-broker/messages",
            get(mqtt_broker::get_messages),
        )
        .route(
            "/api/v1/mqtt-broker/publish",
            post(mqtt_broker::publish_message),
        )
}

#[cfg(feature = "snmp_scan")]
fn snmp_scan_admin_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/snmp/scan", post(snmp_scan::start_scan))
        .route("/api/v1/snmp/scan/status", get(snmp_scan::get_scan_status))
        .route(
            "/api/v1/snmp/scan/results",
            get(snmp_scan::list_scan_results),
        )
        .route(
            "/api/v1/snmp/scan/{ip}/import",
            post(snmp_scan::import_scanned_device),
        )
}

#[cfg(feature = "scripts")]
fn scripts_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/scripts",
            get(scripts::list_scripts).post(scripts::create_script),
        )
        .route(
            "/api/v1/scripts/{id}",
            put(scripts::update_script).delete(scripts::delete_script),
        )
        .route("/api/v1/scripts/{id}/test", post(scripts::test_script))
        .route(
            "/api/v1/scripts/{id}/executions",
            get(scripts::list_executions),
        )
}

pub fn create_router(state: AppState) -> Router {
    // Build CORS layer only for API routes — static assets should not have CORS headers
    let cors = match &state.cors_origin {
        Some(origin) => {
            let origins: Vec<HeaderValue> = origin
                .split(',')
                .map(|o| o.trim().parse().expect("Invalid cors_origin value"))
                .collect();
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                ])
                .allow_headers([
                    CONTENT_TYPE,
                    header::ACCEPT,
                    header::AUTHORIZATION,
                    HeaderName::from_static("x-api-key"),
                ])
                .allow_credentials(true)
        }
        None => {
            // In production, cors_origin MUST be configured. This default only allows
            // local development origins and does NOT allow credentials.
            CorsLayer::new()
                .allow_origin([
                    "http://localhost:5173".parse().unwrap(),
                    "http://localhost:5174".parse().unwrap(),
                    "http://127.0.0.1:5173".parse().unwrap(),
                    "http://127.0.0.1:5174".parse().unwrap(),
                ])
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                ])
                .allow_headers([
                    CONTENT_TYPE,
                    header::ACCEPT,
                    header::AUTHORIZATION,
                    HeaderName::from_static("x-api-key"),
                ])
            // NOTE: allow_credentials is intentionally FALSE when no explicit origin is set
            // to prevent credential leakage to arbitrary origins.
        }
    };

    // Routes contributed by the add-on edition (none in the open core).
    let ext_routes = state.ext.routes(&state);

    // Public routes (no auth required)
    let public = Router::new()
        .route("/api/v1/health", get(system::get_health))
        // Prometheus metrics endpoint (no auth, tier-gated internally)
        .route("/metrics", get(integration::prometheus_metrics))
        .route(
            "/api/v1/networks",
            get(crate::network::handle_list_networks),
        )
        .route("/api/v1/health/detailed", get(crate::health::health_check))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/refresh", post(auth::refresh_token))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/me", get(auth::auth_me))
        .route("/api/v1/auth/csrf-token", get(auth::get_csrf_token))
        .route("/api/v1/auth/providers", get(auth::list_auth_providers))
        // SAML ACS and LDAP login removed from public router until signature
        // validation is properly implemented. Re-enable on the protected router
        // when SAML XML-DSIG verification is complete.
        .route(
            "/api/v1/settings/alarm-cooldown",
            get(settings::get_alarm_cooldown),
        )
        .route(
            "/api/v1/settings/naming-convention",
            get(naming_convention::get_naming_convention),
        )
        .route("/api/license/status", get(license::get_license_status))
        .route("/api/v1/system/features", get(system::get_active_features))
        .route("/api/v1/ws", get(websocket::websocket_handler));
    let public = public.merge(ext_routes.public);

    // Protected routes (any authenticated user — read-only for non-admins)
    let protected = Router::new()
        .route("/api/v1/live", get(system::get_live))
        .route(
            "/api/v1/widgets",
            get(crate::widget_sdk::widget_api::list_widgets),
        )
        .route(
            "/api/v1/widgets/{id}",
            get(crate::widget_sdk::widget_api::get_widget),
        )
        .route(
            "/api/v1/widgets/{id}/asset/{filename}",
            get(crate::widget_sdk::widget_api::get_widget_asset),
        )
        .route(
            "/api/v1/widgets/{id}/install",
            post(crate::widget_sdk::widget_api::install_widget),
        )
        .route(
            "/api/v1/widgets/{id}/uninstall",
            delete(crate::widget_sdk::widget_api::delete_widget),
        )
        .route("/api/v1/system", get(system::get_system))
        .route("/api/v1/system/memory", get(compression::get_system_memory))
        .route(
            "/api/v1/compression/stats",
            get(compression::get_compression_stats),
        )
        .route(
            "/api/v1/compression/config",
            get(compression::get_compression_config),
        )
        .route("/api/v1/version", get(system::get_version))
        .route("/api/v1/docs/topology", get(system_docs::get_topology))
        .route("/api/v1/docs/tag-xref", get(system_docs::get_tag_xref))
        .route(
            "/api/v1/docs/automation",
            get(system_docs::get_automation_docs),
        )
        .route("/api/v1/docs/export", get(system_docs::export_docs))
        .route(
            "/api/v1/docs/config-diff",
            get(system_docs::get_config_diff),
        )
        .route("/api/v1/auth/change-password", post(auth::change_password))
        .route("/api/v1/data", get(data::get_data))
        .route("/api/v1/data/export", get(data::export_data))
        .route("/api/v1/history", get(data::get_history))
        .route("/api/v1/export/csv", get(data::export_csv))
        .route("/api/v1/screens", get(screens::list_screens))
        .route(
            "/api/v1/screens/{id}/data-sources",
            get(screens::list_screen_sensors),
        )
        .route("/api/v1/screens/{id}/layouts", get(layouts::get_layouts))
        .route(
            "/api/v1/screens/{id}/twin-layouts",
            get(layouts::get_twin_layouts),
        )
        .route(
            "/api/v1/screens/{id}/twin-connections",
            get(layouts::get_twin_connections),
        )
        .route(
            "/api/v1/screens/{id}/canvas-layouts",
            get(layouts::get_canvas_layouts),
        )
        .route(
            "/api/v1/screens/{id}/scada-equipment",
            get(layouts::get_scada_equipment),
        )
        .route(
            "/api/v1/screens/{id}/scada-connections",
            get(layouts::get_scada_connections),
        )
        .route("/api/v1/data-sources", get(data_sources::list_data_sources))
        .route("/api/v1/tags/tree", get(tag_tree::get_tag_tree))
        .route(
            "/api/v1/shortcuts",
            get(shortcuts::get_shortcuts).put(shortcuts::save_shortcuts),
        )
        .route(
            "/api/v1/nav-shortcuts",
            get(shortcuts::get_nav_shortcuts).put(shortcuts::save_nav_shortcuts),
        )
        .route(
            "/api/v1/alarms",
            get(alarms::list_alarms).post(alarms::acknowledge_all),
        )
        .route(
            "/api/v1/alarms/active/count",
            get(alarms::active_alarm_count),
        )
        .route(
            "/api/v1/alarms/{id}/acknowledge",
            post(alarms::acknowledge_alarm),
        )
        .route(
            "/api/v1/notifications/settings",
            get(notifications::list_notification_settings),
        )
        // User notifications (any authenticated user sees their own)
        .route(
            "/api/v1/user-notifications",
            get(user_notifications::list_notifications),
        )
        .route(
            "/api/v1/user-notifications/{id}/read",
            put(user_notifications::mark_read),
        )
        .route(
            "/api/v1/user-notifications/{id}",
            delete(user_notifications::dismiss_notification),
        )
        // Command read endpoints (authenticated users)
        .route("/api/v1/commands", get(commands::list_commands))
        .route(
            "/api/v1/commands/{command_id}",
            get(commands::get_command_status),
        )
        .route(
            "/api/v1/commands/{command_id}/audit",
            get(commands::get_command_audit),
        )
        .route("/api/v1/oee/metrics", get(oee::get_oee_metrics))
        .route("/api/v1/oee/downtime", get(oee::list_downtime))
        .route("/api/v1/oee/shifts", get(oee::list_shifts))
        .route("/api/v1/analytics/aggregate", get(analytics::get_aggregate))
        .route("/api/v1/analytics/compare", get(analytics::get_compare))
        .route("/api/v1/analytics/spc", get(analytics::get_spc))
        .route("/api/v1/analytics/histogram", get(analytics::get_histogram))
        .route("/api/v1/analytics/run-chart", get(analytics::get_run_chart))
        .route(
            "/api/v1/analytics/custom-metrics",
            get(analytics::list_custom_metrics).post(analytics::create_custom_metric),
        )
        .route(
            "/api/v1/analytics/custom-metrics/evaluate",
            get(analytics::evaluate_all_custom_metrics),
        )
        .route(
            "/api/v1/analytics/custom-metrics/{id}",
            put(analytics::update_custom_metric).delete(analytics::delete_custom_metric),
        )
        .route(
            "/api/v1/settings/ui-theme",
            get(settings::get_ui_theme).put(settings::update_ui_theme),
        )
        .route(
            "/api/v1/settings/ui-lang",
            get(settings::get_ui_lang).put(settings::update_ui_lang),
        )
        // EDR compatibility routes (any authenticated user can view)
        .route("/api/v1/edr/report", get(edr_compat::get_edr_report))
        .route(
            "/api/v1/edr/exclusions",
            get(edr_compat::get_edr_exclusions),
        )
        .route("/api/v1/edr/io-status", get(edr_compat::get_io_status))
        .route("/api/v1/edr/gpo-guide", get(edr_compat::get_gpo_guide))
        // Integration routes (authenticated users)
        .route("/api/v1/integration/bi/query", get(integration::bi_query))
        .route(
            "/api/v1/integration/grafana/query",
            get(integration::grafana_query),
        )
        .route(
            "/api/v1/integration/grafana/search",
            get(integration::grafana_search),
        )
        .route(
            "/api/v1/integration/connection-info",
            get(integration::connection_info),
        )
        .route(
            "/api/v1/integration/webhooks/outbound",
            get(integration::list_outbound_webhooks),
        )
        // Hardware certification routes (any authenticated user can view)
        .route("/api/v1/hw/report", get(hardware_cert::get_hw_report))
        .route(
            "/api/v1/hw/compatibility-matrix",
            get(hardware_cert::get_compatibility_matrix),
        )
        .route(
            "/api/v1/hw/edge-deploy",
            get(hardware_cert::get_edge_deploy_guide),
        )
        .route(
            "/api/v1/hw/edge-deploy/rpi-quickstart",
            get(hardware_cert::get_rpi_quickstart),
        )
        // AI Copilot routes (any authenticated user)
        .route("/api/v1/ai/settings", get(ai_copilot::get_ai_settings))
        .route(
            "/api/v1/ai/settings/test",
            post(ai_copilot::test_ai_settings),
        )
        .route("/api/v1/ai/proposals", get(ai_copilot::list_proposals))
        .route("/api/v1/ai/proposals/{id}", get(ai_copilot::get_proposal))
        .route(
            "/api/v1/ai/generate/register-map",
            post(ai_copilot::generate_register_map),
        )
        .route(
            "/api/v1/ai/generate/dashboard-layout",
            post(ai_copilot::generate_dashboard_layout),
        )
        .route(
            "/api/v1/ai/generate/alarm-thresholds",
            post(ai_copilot::generate_alarm_thresholds),
        )
        .route(
            "/api/v1/ai/generate/tag-rename",
            post(ai_copilot::generate_tag_rename),
        )
        .route(
            "/api/v1/ai/generate/flow-rule",
            post(ai_copilot::generate_flow_rule),
        )
        .route("/api/v1/ai/templates", get(ai_copilot::list_templates))
        .route(
            "/api/v1/ai/generate/from-template",
            post(ai_copilot::generate_from_template),
        )
        .route("/api/v1/templates", get(vertical_templates::list_templates))
        .route(
            "/api/v1/templates/preview",
            post(vertical_templates::preview_template),
        )
        .route("/api/v1/wizard/create", post(wizard::wizard_create))
        .route(
            "/api/v1/wizard/health-check",
            get(wizard::wizard_health_check),
        )
        .route(
            "/api/v1/wizard/feed-templates",
            get(wizard::list_feed_templates),
        )
        .route(
            "/api/v1/wizard/feed-templates/{id}",
            get(wizard::get_feed_template_handler),
        );
    // Merge protocol-specific protected routes
    #[cfg(feature = "modbus")]
    let protected = protected.merge(modbus_routes());
    #[cfg(feature = "mqtt-broker")]
    let protected = protected.merge(mqtt_broker_routes());
    let protected = protected.merge(voltrus_plc_routes());
    let protected = protected.merge(ext_routes.protected);

    let protected = protected.layer(middleware::from_fn_with_state(
        state.clone(),
        cookie_auth_middleware,
    ));

    // Demo-allowed routes (authenticated + admin or demo role required)
    // These are operational routes that demo users should be able to use.
    let demo_protected = Router::new()
        .route(
            "/api/v1/settings/smtp",
            get(smtp_settings::get_smtp_config).put(smtp_settings::update_smtp_config),
        )
        .route(
            "/api/v1/settings/smtp/test",
            post(smtp_settings::test_smtp_connection),
        )
        .route(
            "/api/v1/settings/alarm-cooldown",
            put(settings::update_alarm_cooldown),
        )
        .route(
            "/api/v1/settings/naming-convention",
            put(naming_convention::update_naming_convention),
        )
        // Alarm management: shelving, audit trail, analytics, summary
        .route("/api/v1/alarms/{id}/shelf", post(alarms::shelf_alarm))
        .route("/api/v1/alarms/{id}/unshelf", post(alarms::unshelf_alarm))
        .route("/api/v1/alarms/audit", get(alarms::list_alarm_audit))
        .route(
            "/api/v1/alarms/{id}/journal",
            get(alarms::get_alarm_journal),
        )
        .route(
            "/api/v1/alarms/analytics",
            get(alarms::list_alarm_analytics),
        )
        .route("/api/v1/alarms/summary", get(alarms::get_alarm_summary))
        .route("/api/v1/screens", post(screens::create_screen))
        .route(
            "/api/v1/screens/{id}",
            put(screens::update_screen).delete(screens::delete_screen),
        )
        .route(
            "/api/v1/screens/{id}/data-sources",
            post(screens::add_screen_sensor),
        )
        .route(
            "/api/v1/screens/{id}/data-sources/{data_source_id}",
            put(screens::update_screen_sensor).delete(screens::remove_screen_sensor),
        )
        .route("/api/v1/screens/{id}/layouts", put(layouts::save_layouts))
        .route(
            "/api/v1/screens/{id}/twin-layouts",
            put(layouts::save_twin_layouts),
        )
        .route(
            "/api/v1/screens/{id}/twin-connections",
            put(layouts::save_twin_connections),
        )
        .route(
            "/api/v1/screens/{id}/twin-camera",
            put(layouts::save_twin_camera),
        )
        .route(
            "/api/v1/screens/{id}/canvas-layouts",
            put(layouts::save_canvas_layouts),
        )
        .route(
            "/api/v1/screens/{id}/scada-equipment",
            put(layouts::save_scada_equipment),
        )
        .route(
            "/api/v1/screens/{id}/scada-connections",
            put(layouts::save_scada_connections),
        )
        .route(
            "/api/v1/data-sources",
            post(data_sources::create_data_source),
        )
        .route(
            "/api/v1/data-sources/reorder",
            patch(data_sources::reorder_data_sources),
        )
        .route(
            "/api/v1/data-sources/{id}",
            put(data_sources::update_data_source).delete(data_sources::delete_data_source),
        )
        .route(
            "/api/v1/flows",
            get(flows::list_flows).post(flows::create_flow),
        )
        .route(
            "/api/v1/flows/{id}",
            get(flows::get_flow)
                .put(flows::update_flow)
                .delete(flows::delete_flow),
        )
        .route("/api/v1/flows/{id}/start", post(flows::start_flow))
        .route("/api/v1/flows/{id}/stop", post(flows::stop_flow))
        .route("/api/v1/flows/{id}/test", post(flows::test_flow))
        .route("/api/v1/flows/{id}/runs", get(flows::list_flow_runs))
        .route(
            "/api/v1/notifications/settings",
            post(notifications::create_notification_setting),
        )
        .route(
            "/api/v1/notifications/settings/{id}",
            put(notifications::update_notification_setting)
                .delete(notifications::delete_notification_setting),
        )
        .route(
            "/api/v1/notifications/test",
            post(notifications::test_notification),
        )
        .route(
            "/api/v1/edr/exclusions/ps1",
            get(edr_compat::get_defender_ps1),
        )
        // Hardware certification admin routes (benchmark is admin-only, writes to DB)
        .route(
            "/api/v1/hw/benchmark",
            post(hardware_cert::run_hw_benchmark),
        )
        // RTSP Video / Camera routes
        .route("/api/v1/oee/downtime", post(oee::create_downtime))
        .route("/api/v1/oee/downtime/{id}/close", put(oee::close_downtime))
        .route("/api/v1/oee/shifts", post(oee::create_shift))
        .route("/api/v1/oee/shifts/{id}", delete(oee::delete_shift))
        .route(
            "/api/v1/recipes",
            get(recipes::list_recipes).post(recipes::create_recipe),
        )
        .route(
            "/api/v1/recipes/{id}",
            get(recipes::get_recipe)
                .put(recipes::update_recipe)
                .delete(recipes::delete_recipe),
        )
        .route("/api/v1/recipes/{id}/load", post(recipes::load_recipe))
        // OT Asset Inventory routes
        .route(
            "/api/v1/assets",
            get(assets::list_assets).post(assets::create_asset),
        )
        .route("/api/v1/assets/ip-conflicts", get(assets::get_ip_conflicts))
        .route(
            "/api/v1/assets/{id}",
            get(assets::get_asset)
                .patch(assets::update_asset)
                .delete(assets::delete_asset),
        )
        .route(
            "/api/v1/assets/{id}/maintenance",
            get(maintenance::list_maintenance).post(maintenance::create_maintenance),
        )
        .route("/api/v1/assets/import", post(asset_import::upload_import))
        .route(
            "/api/v1/assets/import/confirm",
            post(asset_import::confirm_import),
        )
        .route("/api/v1/assets/nis2-export", get(nis2_export::nis2_export))
        .route(
            "/api/v1/firmware-catalog",
            get(firmware_catalog::list_firmware_catalog)
                .post(firmware_catalog::create_firmware_catalog),
        )
        .route(
            "/api/v1/firmware-catalog/{id}",
            patch(firmware_catalog::update_firmware_catalog)
                .delete(firmware_catalog::delete_firmware_catalog),
        )
        // Audit log routes
        .route("/api/v1/audit", get(audit_log::list_audit_logs))
        .route("/api/v1/audit/export", get(audit_log::export_audit_csv))
        // Annotation routes
        .route(
            "/api/v1/annotations",
            get(annotations::list_annotations).post(annotations::create_annotation),
        )
        .route(
            "/api/v1/annotations/{id}",
            get(annotations::get_annotation)
                .put(annotations::update_annotation)
                .delete(annotations::delete_annotation),
        )
        // Shift Handover routes
        .route(
            "/api/v1/shift-handover",
            get(shift_handover::list_shift_handovers).post(shift_handover::create_shift_handover),
        )
        .route(
            "/api/v1/shift-handover/auto-report",
            get(shift_handover::auto_shift_report),
        )
        .route(
            "/api/v1/shift-handover/{id}",
            get(shift_handover::get_shift_handover),
        )
        // Command write/cancel routes
        .route("/api/v1/tags/{id}/write", post(commands::write_tag))
        .route("/api/v1/tags/{id}/path", put(tag_tree::update_tag_path))
        .route(
            "/api/v1/commands/{command_id}/cancel",
            post(commands::cancel_command),
        )
        .route(
            "/api/v1/data-sources/{id}/write-policy",
            patch(commands::update_write_policy),
        )
        // Simulation routes
        .route(
            "/api/v1/sim-profiles",
            get(simulation::list_sim_profiles).post(simulation::create_sim_profile),
        )
        .route(
            "/api/v1/sim-profiles/{id}",
            put(simulation::update_sim_profile).delete(simulation::delete_sim_profile),
        )
        .route(
            "/api/v1/devices/{id}/simulate",
            post(simulation::toggle_simulation),
        )
        .route("/api/v1/devices/{id}/go-live", post(simulation::go_live))
        // Node-RED import
        .route(
            "/api/v1/import/node-red",
            post(import_nodered::import_node_red),
        )
        // Config snapshot routes
        .route(
            "/api/v1/config/snapshots",
            get(config_snapshots::list_snapshots).post(config_snapshots::create_snapshot),
        )
        .route(
            "/api/v1/config/snapshots/{id}",
            get(config_snapshots::get_snapshot),
        )
        .route(
            "/api/v1/config/snapshots/{from_id}/diff/{to_id}",
            get(config_snapshots::diff_snapshots),
        )
        .route(
            "/api/v1/config/snapshots/{id}/rollback",
            post(config_snapshots::rollback),
        )
        // AI Copilot admin routes
        .route("/api/v1/ai/settings", put(ai_copilot::update_ai_settings))
        // Integration admin routes
        .route(
            "/api/v1/egress/jobs",
            get(integration::list_egress_jobs).post(integration::create_egress_job),
        )
        .route(
            "/api/v1/egress/jobs/{id}",
            get(integration::get_egress_job)
                .put(integration::update_egress_job)
                .delete(integration::delete_egress_job),
        )
        .route(
            "/api/v1/egress/jobs/{id}/run",
            post(integration::run_egress_job),
        )
        .route(
            "/api/v1/integration/webhooks/outbound",
            post(integration::create_outbound_webhook),
        )
        .route(
            "/api/v1/integration/webhooks/outbound/{id}",
            delete(integration::delete_outbound_webhook),
        )
        .route(
            "/api/v1/integration/webhooks/outbound/{id}/test",
            post(integration::test_outbound_webhook),
        )
        .route(
            "/api/v1/ai/proposals/{id}/approve",
            post(ai_copilot::approve_proposal),
        )
        .route(
            "/api/v1/ai/proposals/{id}/reject",
            post(ai_copilot::reject_proposal),
        )
        .route(
            "/api/v1/ai/proposals/{id}/apply",
            post(ai_copilot::apply_proposal),
        )
        .route(
            "/api/v1/templates/{id}",
            get(vertical_templates::get_template),
        )
        .route(
            "/api/v1/templates/{id}/instantiate",
            post(vertical_templates::instantiate_template),
        );
    // Merge protocol-specific demo-allowed routes
    #[cfg(feature = "modbus")]
    let demo_protected = demo_protected.merge(modbus_admin_routes());
    #[cfg(feature = "opcua")]
    let demo_protected = demo_protected.merge(opcua_admin_routes());
    #[cfg(feature = "s7")]
    let demo_protected = demo_protected.merge(s7_admin_routes());
    #[cfg(feature = "ethernet-ip")]
    let demo_protected = demo_protected.merge(enip_admin_routes());
    #[cfg(feature = "dnp3")]
    let demo_protected = demo_protected.merge(dnp3_admin_routes());
    #[cfg(feature = "bacnet")]
    let demo_protected = demo_protected.merge(bacnet_admin_routes());
    #[cfg(feature = "mqtt-broker")]
    let demo_protected = demo_protected.merge(mqtt_broker_admin_routes());
    #[cfg(feature = "snmp_scan")]
    let demo_protected = demo_protected.merge(snmp_scan_admin_routes());
    #[cfg(feature = "scripts")]
    let demo_protected = demo_protected.merge(scripts_admin_routes());
    let demo_protected = demo_protected.merge(voltrus_plc_admin_routes());
    let demo_protected = demo_protected.merge(ext_routes.operator);

    let demo_protected = demo_protected
        .layer(middleware::from_fn(demo_allowed_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cookie_auth_middleware,
        ));

    // Strictly admin-only routes (users, roles, API keys, security, compliance)
    // Demo users CANNOT access these.
    let admin_protected = Router::new()
        .route("/api/v1/seed", post(auth::seed))
        .route(
            "/api/v1/users",
            get(auth::list_users).post(auth::create_user_handler),
        )
        .route(
            "/api/v1/users/{id}",
            put(auth::update_user_handler).delete(auth::delete_user_handler),
        )
        // RBAC routes (admin-only, license-gated)
        .route(
            "/api/v1/roles",
            get(rbac::list_roles).post(rbac::create_role),
        )
        .route(
            "/api/v1/roles/{id}",
            put(rbac::update_role).delete(rbac::delete_role),
        )
        .route("/api/v1/users/{id}/role", put(rbac::assign_user_role))
        // API keys (admin-only)
        .route(
            "/api/v1/api-keys",
            post(api_keys::create_api_key).get(api_keys::list_api_keys),
        )
        .route("/api/v1/api-keys/{id}", delete(api_keys::revoke_api_key));

    let admin_protected = admin_protected.merge(ext_routes.admin);

    let admin_protected = admin_protected
        .layer(middleware::from_fn(admin_only_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cookie_auth_middleware,
        ));

    async fn api_not_found() -> impl IntoResponse {
        (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
    }

    // CSV/Excel import router with increased body limit (admin-only)
    let import_router = Router::new()
        .route("/api/v1/import/csv", post(csv_import::import_csv))
        .route("/api/v1/import/excel", post(excel_import::import_excel))
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024)) // 50MB for file upload
        .layer(middleware::from_fn(admin_only_middleware))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cookie_auth_middleware,
        ));

    // API router with CORS, rate limiting, security headers, body limit
    let api_router = Router::new()
        .merge(public)
        .merge(protected)
        .merge(demo_protected)
        .merge(admin_protected)
        .merge(import_router)
        .route("/api/v1/{*path}", get(api_not_found))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            edition_header_middleware,
        ))
        .layer(cors)
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1MB body limit
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::security::global_rate_limit_middleware,
        ))
        .layer(middleware::from_fn(crate::security::request_id_middleware))
        .layer(middleware::from_fn(
            crate::security::security_headers_middleware,
        ))
        .layer(middleware::from_fn(
            crate::security::strip_forwarded_host_middleware,
        ));

    // Static file serving WITHOUT CORS headers
    let static_router = Router::new()
        .fallback_service(ServeDir::new("static").fallback(ServeFile::new("static/index.html")))
        .layer(middleware::from_fn(
            crate::security::block_source_maps_middleware,
        ))
        .layer(middleware::from_fn(
            crate::security::limit_range_requests_middleware,
        ))
        .layer(middleware::from_fn(
            crate::security::strip_accept_ranges_middleware,
        ))
        .layer(middleware::from_fn(
            crate::security::security_headers_middleware,
        ))
        .layer(middleware::from_fn(
            crate::security::strip_forwarded_host_middleware,
        ));

    // `mut` is consumed by the cfg(openapi) docs-route attachment below.
    #[allow(unused_mut)]
    let mut router = Router::new().merge(api_router).merge(static_router);

    #[cfg(feature = "openapi")]
    {
        router = router
            .route("/api/v1/openapi.json", get(openapi::openapi_spec))
            .route("/api/docs", get(openapi::scalar_docs));
    }

    router.with_state(state)
}

// Middleware types re-exported from middleware module
pub use auth_middleware::{
    AuthUser, admin_only_middleware, cookie_auth_middleware, demo_allowed_middleware,
};

#[cfg(test)]
mod tests;
