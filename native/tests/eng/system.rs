//! Data Manager › System against the live plant: every path the section
//! calls. The template instance and the wizard's device are created and
//! then removed again: only the rows this test made (claimed by id before
//! it asks, so a failure halfway removes them too, from a drop guard) and
//! the limits the instance may have set on the plant's own sources put
//! back; nobody else's rows are touched. The SMTP settings are changed and
//! put back exactly (the password stays: the server never sends it, and a
//! PUT carrying its mask keeps the stored one). No test email is sent.

use std::collections::HashSet;

use serde::de::IgnoredAny;
use serde_json::{Value, json};
use voltrus_scada_native::api::eng_system::{
    AUTOMATION_PATH, Automation, DOCS_EXPORT_PATH, FEED_TEMPLATES_PATH, FeedTemplate,
    FeedTemplateList, HEALTH_PATH, HealthReport, InstantiateBody, Instantiated, PASSWORD_MASK,
    PreviewAlarm, PreviewBody, SMTP_PATH, SMTP_TEST_PATH, SYSTEM_PATH, SmtpSettings,
    SmtpTestAnswer, SmtpTestBody, SmtpUpdate, TAG_XREF_PATH, TEMPLATE_PREVIEW_PATH, TOPOLOGY_PATH,
    TagXref, TemplateList, TemplatePreview, TemplateSource, Topology, VerticalTemplate,
    WIZARD_CREATE_PATH, WizardDraft, WizardResult, feed_template_path, instance_device_id,
    instantiate_path, template_path, templates_path,
};
use voltrus_scada_native::api::insights::ShiftConfig;
use voltrus_scada_native::api::ops::encode;
use voltrus_scada_native::api::{DataSource, Method, ServerError, VoltrusApi};

/// The test email's recipient, should one ever be sent: a reserved domain
/// that never resolves (RFC 2606), so no mail can be delivered.
const PROBE_RECIPIENT: &str = "e2e@voltrus.invalid";

pub(crate) async fn round_trip(api: &VoltrusApi) {
    overview(api).await;
    docs(api).await;
    let template = templates(api).await;
    instantiate(api, &template).await;
    wizard(api).await;
    health(api).await;
    smtp(api).await;
}

async fn overview(api: &VoltrusApi) {
    let info: voltrus_scada_native::api::eng_system::SystemInfo =
        api.get_json(SYSTEM_PATH).await.expect("GET /system");
    assert!(!info.hostname.is_empty(), "the host names itself");
    assert!(info.process.pid > 0, "the server process has a pid");
    assert!(info.memory_total_mb > 0.0);
    assert!((0.0..=100.0).contains(&info.memory_percent));
    println!(
        "system: {} ({}), cpu {:.1}%, memory {:.0}/{:.0} MB, db {:.1} MB, pid {}",
        info.hostname,
        info.os,
        info.cpu_usage,
        info.memory_used_mb,
        info.memory_total_mb,
        info.db_size_mb,
        info.process.pid
    );
}

async fn docs(api: &VoltrusApi) {
    let topology: Topology = api
        .get_json(TOPOLOGY_PATH)
        .await
        .expect("GET /docs/topology");
    assert_eq!(topology.summary.total_devices, topology.devices.len());
    assert_eq!(
        topology.summary.online_count + topology.summary.offline_count,
        topology.devices.len()
    );
    let xref: TagXref = api
        .get_json(TAG_XREF_PATH)
        .await
        .expect("GET /docs/tag-xref");
    assert_eq!(xref.total_tags, xref.tags.len());
    assert!(xref.total_tags > 0, "the plant has tags");
    let automation: Automation = api
        .get_json(AUTOMATION_PATH)
        .await
        .expect("GET /docs/automation");
    assert_eq!(automation.summary.total_flows, automation.flows.len());
    assert_eq!(automation.summary.total_scripts, automation.scripts.len());
    let html = api
        .get_text(DOCS_EXPORT_PATH)
        .await
        .expect("GET /docs/export?format=html");
    assert!(html.starts_with("<!DOCTYPE html>"), "the export is a page");
    assert!(html.contains("Voltrus System Docs"));
    println!(
        "docs: {} devices, {} tags, {} flows, {} scripts, export {} bytes",
        topology.devices.len(),
        xref.total_tags,
        automation.flows.len(),
        automation.scripts.len(),
        html.len()
    );
}

/// The list, one industry, a template's detail and its preview; the
/// template with the fewest tags, for the instance.
async fn templates(api: &VoltrusApi) -> VerticalTemplate {
    let all: TemplateList = api
        .get_json(&templates_path(None))
        .await
        .expect("GET /templates");
    assert!(!all.templates.is_empty(), "the server has templates");
    assert!(!all.industries.is_empty(), "and industries");
    let smallest = all
        .templates
        .iter()
        .min_by_key(|template| template.tag_count)
        .expect("a template")
        .clone();
    let one: TemplateList = api
        .get_json(&templates_path(Some(&smallest.industry)))
        .await
        .expect("GET /templates?industry=");
    assert!(
        one.templates
            .iter()
            .all(|template| template.industry == smallest.industry),
        "the industry filter holds"
    );
    assert!(
        one.templates
            .iter()
            .any(|template| template.id == smallest.id)
    );
    let detail: VerticalTemplate = api
        .get_json(&template_path(&smallest.id))
        .await
        .expect("GET /templates/{id}");
    assert_eq!(detail.id, smallest.id);
    assert_eq!(detail.tags.len(), smallest.tag_count);
    let body = PreviewBody::new(&smallest.id, "1").expect("a preview body");
    let preview: TemplatePreview = api
        .send_json(Method::POST, TEMPLATE_PREVIEW_PATH, &body)
        .await
        .expect("POST /templates/preview");
    assert_eq!(preview.template_id, smallest.id);
    assert!(
        !preview.expanded_tags.is_empty(),
        "the preview expands tags"
    );
    assert_eq!(preview.flows.len(), detail.flows.len());
    println!(
        "templates: {} in {} industries; {} previews {} tags, {} alarms, {} flows",
        all.templates.len(),
        all.industries.len(),
        smallest.id,
        preview.expanded_tags.len(),
        preview.expanded_alarms.len(),
        preview.flows.len()
    );
    detail
}

/// One data source as the cleanup needs it.
#[derive(Debug, Clone, PartialEq)]
struct Source {
    id: i64,
    key: String,
    device_id: String,
    warn: f64,
    crit: f64,
}

impl From<&DataSource> for Source {
    fn from(source: &DataSource) -> Self {
        Self {
            id: source.id,
            key: source.key.clone(),
            device_id: source.device_id.clone(),
            warn: source.warn_val,
            crit: source.crit_val,
        }
    }
}

/// What the plant has, to tell what is already there from what a step
/// may make.
#[derive(Debug, Clone)]
struct Plant {
    sources: Vec<Source>,
    devices: HashSet<String>,
    flows: HashSet<String>,
    shifts: Vec<ShiftConfig>,
}

async fn plant(api: &VoltrusApi) -> Plant {
    let sources: Vec<DataSource> = api
        .get_json("/api/v1/data-sources")
        .await
        .expect("GET /data-sources");
    let devices: Vec<Value> = api.get_json("/api/v1/screens").await.expect("GET /screens");
    let flows: Value = api.get_json("/api/v1/flows").await.expect("GET /flows");
    let shifts: Vec<ShiftConfig> = api
        .get_json("/api/v1/oee/shifts")
        .await
        .expect("GET /oee/shifts");
    Plant {
        sources: sources.iter().map(Source::from).collect(),
        devices: devices
            .iter()
            .filter_map(|device| device["id"].as_str().map(str::to_owned))
            .collect(),
        flows: flows["flows"]
            .as_array()
            .map(|flows| {
                flows
                    .iter()
                    .filter_map(|flow| flow["id"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        shifts,
    }
}

/// Exactly what one step of this test made, and what it changed: only
/// these rows are removed or put back, never one someone else made in the
/// meantime.
#[derive(Debug, Clone, Default)]
struct Made {
    /// Devices this test made: every data source on them is its own.
    source_devices: Vec<String>,
    /// The plant's own sources whose limits the step may change, as they
    /// were.
    limits: Vec<Source>,
    flows: Vec<String>,
    /// Devices and screens (both live under `/api/v1/screens`).
    devices: Vec<String>,
    shifts: Vec<i64>,
    /// The SMTP settings to put back.
    smtp: Option<SmtpUpdate>,
}

impl Made {
    fn is_empty(&self) -> bool {
        self.source_devices.is_empty()
            && self.limits.is_empty()
            && self.flows.is_empty()
            && self.devices.is_empty()
            && self.shifts.is_empty()
            && self.smtp.is_none()
    }

    /// Remove what was made and put back what was changed; what failed (a
    /// row that is already gone is not a failure: the step may have failed
    /// before making it).
    async fn undo(&self, api: &VoltrusApi) -> Vec<String> {
        let mut failed = Vec::new();
        if !self.source_devices.is_empty() || !self.limits.is_empty() {
            match api
                .get_json::<Vec<DataSource>>("/api/v1/data-sources")
                .await
            {
                Ok(now) => {
                    for id in made_sources(&now, &self.source_devices) {
                        let result = api
                            .send_empty(Method::DELETE, &format!("/api/v1/data-sources/{id}"))
                            .await;
                        settle(&mut failed, format!("delete data source {id}"), result);
                    }
                    for old in changed_limits(&now, &self.limits) {
                        let result = api
                            .send_json(
                                Method::PUT,
                                &format!("/api/v1/data-sources/{}", old.id),
                                &json!({ "warn_val": old.warn, "crit_val": old.crit }),
                            )
                            .await;
                        settle(
                            &mut failed,
                            format!("put back the limits of {}", old.key),
                            result,
                        );
                    }
                }
                Err(error) => failed.push(format!("GET /data-sources: {error}")),
            }
        }
        for id in &self.flows {
            let result = api
                .send_empty(Method::DELETE, &format!("/api/v1/flows/{}", encode(id)))
                .await;
            settle(&mut failed, format!("delete flow {id}"), result);
        }
        for id in &self.devices {
            let result = api
                .send_empty(Method::DELETE, &format!("/api/v1/screens/{}", encode(id)))
                .await;
            settle(&mut failed, format!("delete device or screen {id}"), result);
        }
        for id in &self.shifts {
            let result = api
                .send_empty(Method::DELETE, &format!("/api/v1/oee/shifts/{id}"))
                .await;
            settle(&mut failed, format!("delete shift {id}"), result);
        }
        if let Some(back) = &self.smtp {
            let result = api.send_json(Method::PUT, SMTP_PATH, back).await;
            settle(&mut failed, "put the SMTP settings back".to_owned(), result);
        }
        failed
    }
}

/// Note `what` as failed unless it worked or the row was already gone.
fn settle(failed: &mut Vec<String>, what: String, result: anyhow::Result<IgnoredAny>) {
    let Err(error) = result else {
        return;
    };
    let gone = error
        .downcast_ref::<ServerError>()
        .is_some_and(|server| server.status == reqwest::StatusCode::NOT_FOUND);
    if !gone {
        failed.push(format!("{what}: {error}"));
    }
}

/// Undoes `made` when dropped, so a step that panics halfway still leaves
/// the plant as it found it; `finish` undoes it on the normal path and
/// checks that it worked.
struct Cleanup {
    api: VoltrusApi,
    made: Made,
}

impl Cleanup {
    fn new(api: &VoltrusApi) -> Self {
        Self {
            api: api.clone(),
            made: Made::default(),
        }
    }

    /// Undo what was made; it all must go. What it was, to check after.
    async fn finish(mut self) -> Made {
        let made = std::mem::take(&mut self.made);
        let failed = made.undo(&self.api).await;
        assert!(failed.is_empty(), "the cleanup failed: {failed:?}");
        made
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let made = std::mem::take(&mut self.made);
        if made.is_empty() {
            return;
        }
        // The test's runtime is single-threaded and unwinding: undo on a
        // thread with a runtime and a connection pool of its own.
        let base = self.api.base.clone();
        let token = self.api.token.clone();
        let undone = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a cleanup runtime");
            runtime.block_on(async move {
                let mut api = VoltrusApi::new(&base);
                api.token = token;
                made.undo(&api).await
            })
        })
        .join();
        match undone {
            Ok(failed) if failed.is_empty() => eprintln!("cleanup after a failure: done"),
            Ok(failed) => eprintln!("cleanup after a failure left: {failed:?}"),
            Err(_) => eprintln!("cleanup after a failure panicked"),
        }
    }
}

/// The ids of the sources on devices this test made.
fn made_sources(now: &[DataSource], devices: &[String]) -> Vec<i64> {
    now.iter()
        .filter(|source| devices.contains(&source.device_id))
        .map(|source| source.id)
        .collect()
}

/// The sources of `before` whose limits differ now.
fn changed_limits<'a>(now: &[DataSource], before: &'a [Source]) -> Vec<&'a Source> {
    before
        .iter()
        .filter(|old| {
            now.iter().any(|source| {
                source.id == old.id && (source.warn_val != old.warn || source.crit_val != old.crit)
            })
        })
        .collect()
}

/// The plant's sources an instance's alarm limits may land on: the server
/// sets each alarm on the first source whose key ends with its tag pattern
/// or holds `:pattern` (`Instantiator::instantiate`), the plant's own
/// included.
fn at_risk_limits(sources: &[Source], alarms: &[PreviewAlarm]) -> Vec<Source> {
    sources
        .iter()
        .filter(|source| {
            alarms.iter().any(|alarm| {
                source.key.ends_with(&alarm.tag_pattern)
                    || source.key.contains(&format!(":{}", alarm.tag_pattern))
            })
        })
        .cloned()
        .collect()
}

/// The id `Instantiator` gives a template's flow.
fn instance_flow_id(template: &str, flow: &str) -> String {
    format!(
        "vt_{template}_{}",
        flow.to_lowercase().replace([' ', '/'], "_")
    )
}

/// The id `Instantiator` gives an instance's screen.
fn instance_screen_id(template: &str, name: &str) -> String {
    format!("{template}_{}", name.to_lowercase().replace(' ', "_"))
}

/// The shift `Instantiator` adds for OEE, only to a plant that has none:
/// "Day Shift" from 06:00 for 8 hours, every day.
fn instance_shifts(before: &[ShiftConfig], after: &[ShiftConfig]) -> Vec<i64> {
    if !before.is_empty() {
        return Vec::new();
    }
    after
        .iter()
        .filter(|shift| {
            shift.name == "Day Shift"
                && shift.start_hour == 6
                && shift.start_minute == 0
                && shift.duration_hours == 8
                && shift.days_mask == 127
        })
        .map(|shift| shift.id)
        .collect()
}

/// Nothing `made` named is left, and the limits are as they were.
async fn assert_undone(api: &VoltrusApi, made: &Made) {
    let now = plant(api).await;
    assert!(
        !now.sources
            .iter()
            .any(|source| made.source_devices.contains(&source.device_id)),
        "the made data sources are gone"
    );
    for old in &made.limits {
        if let Some(source) = now.sources.iter().find(|source| source.id == old.id) {
            assert_eq!(
                (source.warn, source.crit),
                (old.warn, old.crit),
                "the limits of {} are back",
                old.key
            );
        }
    }
    for id in &made.flows {
        assert!(!now.flows.contains(id), "flow {id} is gone");
    }
    for id in &made.devices {
        assert!(!now.devices.contains(id), "device {id} is gone");
    }
    for id in &made.shifts {
        assert!(
            now.shifts.iter().all(|shift| shift.id != *id),
            "shift {id} is gone"
        );
    }
}

/// `POST /templates/{id}/instantiate`, one instance, then all of it removed.
async fn instantiate(api: &VoltrusApi, template: &VerticalTemplate) {
    let before = plant(api).await;
    let name = format!("e2e native {:x}", std::process::id());
    let device = instance_device_id(&name);
    let screen = instance_screen_id(&template.id, &name);
    assert!(
        !before.devices.contains(&device),
        "no device of that name yet"
    );
    assert!(
        !before.devices.contains(&screen),
        "no screen of that name yet"
    );
    let body = PreviewBody::new(&template.id, "1").expect("a preview body");
    let preview: TemplatePreview = api
        .send_json(Method::POST, TEMPLATE_PREVIEW_PATH, &body)
        .await
        .expect("POST /templates/preview");
    // Claimed before asking, so a failure halfway removes them too.
    let mut cleanup = Cleanup::new(api);
    cleanup.made = Made {
        source_devices: vec![device.clone()],
        limits: at_risk_limits(&before.sources, &preview.expanded_alarms),
        flows: template
            .flows
            .iter()
            .map(|flow| instance_flow_id(&template.id, &flow.name))
            .filter(|id| !before.flows.contains(id))
            .collect(),
        devices: vec![screen.clone(), device.clone()],
        ..Made::default()
    };
    let body = InstantiateBody::new(&template.id, &name, "1").expect("an instantiate body");
    let made: Instantiated = api
        .send_json(Method::POST, &instantiate_path(&template.id), &body)
        .await
        .expect("POST /templates/{id}/instantiate");
    assert_eq!(made.template_id, template.id);
    assert_eq!(made.device_id, device);
    assert_eq!(made.screen_id, screen);
    assert!(made.tags_created > 0, "the instance made tags");
    for flow in &made.flows {
        let id = instance_flow_id(&template.id, flow);
        assert!(
            cleanup.made.flows.contains(&id),
            "flow {id} is one this test claimed"
        );
    }
    let during = plant(api).await;
    assert!(during.devices.contains(&made.device_id));
    assert!(during.devices.contains(&made.screen_id));
    if template.oee_config.is_some() {
        cleanup.made.shifts = instance_shifts(&before.shifts, &during.shifts);
    }
    println!(
        "instantiated {} as {}: {} tags, {} alarm limits, {} flows, screen {}",
        template.id,
        made.device_id,
        made.tags_created,
        made.alarms_created,
        made.flows_created,
        made.screen_id
    );
    let undone = cleanup.finish().await;
    assert_undone(api, &undone).await;
    println!("instance removed: its sources, flows, device, screen and shift");
}

async fn wizard(api: &VoltrusApi) {
    let feeds: FeedTemplateList = api
        .get_json(FEED_TEMPLATES_PATH)
        .await
        .expect("GET /wizard/feed-templates");
    let first = feeds.templates.first().expect("a FEED template").clone();
    let feed: FeedTemplate = api
        .get_json(&feed_template_path(&first.id))
        .await
        .expect("GET /wizard/feed-templates/{id}");
    assert_eq!(feed.id, first.id);
    assert!(!feed.io_list.is_empty(), "a FEED template has an I/O list");
    println!(
        "FEED templates: {}; {} has {} signals",
        feeds.templates.len(),
        feed.id,
        feed.io_list.len()
    );

    // The full wizard with no template and no scan: only a device, which
    // goes again after.
    let before = plant(api).await;
    let device = format!("e2e_wizard_{:x}", std::process::id());
    assert!(
        !before.devices.contains(&device),
        "no device of that name yet"
    );
    let mut cleanup = Cleanup::new(api);
    cleanup.made = Made {
        source_devices: vec![device.clone()],
        devices: vec![device.clone()],
        ..Made::default()
    };
    let mut draft = WizardDraft {
        industry: "Energy".to_owned(),
        source: TemplateSource::Vertical,
        template: None,
        device_id: device.clone(),
        auto_scan: false,
        ..WizardDraft::default()
    };
    draft.set_protocol("simulation");
    let config = draft.config().expect("a wizard body");
    let result: WizardResult = api
        .send_json(Method::POST, WIZARD_CREATE_PATH, &config)
        .await
        .expect("POST /wizard/create");
    assert!(result.success);
    assert_eq!(result.device_id, device);
    assert_eq!(result.tags_created, 0, "no template, no scan, no tags");
    assert!(
        !result.warnings.is_empty(),
        "the server warns of an empty device"
    );
    assert!(plant(api).await.devices.contains(&device));
    let undone = cleanup.finish().await;
    assert_undone(api, &undone).await;
    println!("wizard created device {device} and it was removed");
}

async fn health(api: &VoltrusApi) {
    let report: HealthReport = api
        .get_json(HEALTH_PATH)
        .await
        .expect("GET /wizard/health-check");
    assert!(report.overall_score <= 100);
    assert!(report.summary.total_tags > 0, "the plant has tags");
    println!(
        "health: score {}, {} issues ({} critical, {} warnings)",
        report.overall_score,
        report.issues.len(),
        report.summary.critical_issues,
        report.summary.warning_issues
    );
}

async fn smtp(api: &VoltrusApi) {
    let original: SmtpSettings = api.get_json(SMTP_PATH).await.expect("GET /settings/smtp");
    let configured = original.restore().is_some();
    match original.restore() {
        Some(back) => {
            assert_eq!(
                original.password.as_deref(),
                Some(PASSWORD_MASK),
                "the server never sends the password"
            );
            let mut cleanup = Cleanup::new(api);
            cleanup.made.smtp = Some(back.clone());
            let mut changed = back;
            changed.from_name = Some(format!(
                "{} (e2e)",
                original.from_name.clone().unwrap_or_default()
            ));
            api.send_json::<_, IgnoredAny>(Method::PUT, SMTP_PATH, &changed)
                .await
                .expect("PUT /settings/smtp");
            let now: SmtpSettings = api.get_json(SMTP_PATH).await.expect("GET again");
            assert_eq!(now.from_name, changed.from_name);
            assert_eq!(now.host, original.host);
            cleanup.finish().await;
            let restored: SmtpSettings = api.get_json(SMTP_PATH).await.expect("GET restored");
            assert_eq!(restored, original, "the SMTP settings are as they were");
            println!(
                "smtp: {} changed and put back, the password kept",
                original.host.clone().unwrap_or_default()
            );
        }
        None => {
            // No settings to change and put back (no route removes them
            // again): the route still answers, refusing an empty host
            // without saving anything.
            let refused = api
                .send_json::<_, IgnoredAny>(
                    Method::PUT,
                    SMTP_PATH,
                    &json!({ "host": " ", "username": "" }),
                )
                .await
                .expect_err("an empty host is refused");
            let server = refused
                .downcast_ref::<ServerError>()
                .expect("the server answered");
            assert_ne!(server.status, reqwest::StatusCode::NOT_FOUND);
            assert!(server.message.contains("host"), "{}", server.message);
            let still: SmtpSettings = api.get_json(SMTP_PATH).await.expect("GET again");
            assert!(!still.configured, "nothing was saved");
            eprintln!("note: the plant has no SMTP settings; PUT checked by its refusal only");
        }
    }

    // The test email, without sending one: a blank recipient is refused
    // before the server reads its SMTP settings (`test_smtp_connection`),
    // so the route answers and no mail leaves. The console refuses it too.
    assert!(SmtpTestBody::new(" ").is_err(), "the console asks for one");
    let refused = api
        .send_json::<_, SmtpTestAnswer>(Method::POST, SMTP_TEST_PATH, &json!({ "recipient": " " }))
        .await
        .expect_err("a blank recipient is refused");
    let server = refused
        .downcast_ref::<ServerError>()
        .unwrap_or_else(|| panic!("the server did not answer: {refused}"));
    assert_ne!(
        server.status,
        reqwest::StatusCode::NOT_FOUND,
        "the test route exists"
    );
    assert!(server.message.contains("Recipient"), "{}", server.message);
    println!(
        "smtp test refused a blank recipient: {} {}",
        server.status.as_u16(),
        server.message
    );
    if !configured {
        // With no SMTP settings there is nothing to send with, so the
        // console's own body goes through to the server's refusal; its
        // recipient could not receive mail anyway.
        let body = SmtpTestBody::new(PROBE_RECIPIENT).expect("a test body");
        let refused = api
            .send_json::<_, SmtpTestAnswer>(Method::POST, SMTP_TEST_PATH, &body)
            .await
            .expect_err("nothing to send with");
        let server = refused
            .downcast_ref::<ServerError>()
            .unwrap_or_else(|| panic!("the server did not answer: {refused}"));
        assert_ne!(server.status, reqwest::StatusCode::NOT_FOUND);
        println!("smtp test without settings: {}", server.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: i64, key: &str, device_id: &str, warn: f64, crit: f64) -> DataSource {
        DataSource {
            id,
            key: key.to_owned(),
            device_id: device_id.to_owned(),
            warn_val: warn,
            crit_val: crit,
            ..DataSource::default()
        }
    }

    fn shift(id: i64, name: &str, start_hour: u32) -> ShiftConfig {
        ShiftConfig {
            id,
            name: name.to_owned(),
            start_hour,
            start_minute: 0,
            duration_hours: 8,
            days_mask: 127,
            ..ShiftConfig::default()
        }
    }

    #[test]
    fn only_the_sources_of_made_devices_are_deleted() {
        let now = [
            source(1, "plant:PV_POWER", "inverter", 0.0, 0.0),
            source(2, "vt_e2e:PMP1_RUN", "vt_e2e", 0.0, 0.0),
            // Someone else's source, made while the test ran.
            source(3, "vt_other:PMP1_RUN", "vt_other", 0.0, 0.0),
        ];
        assert_eq!(made_sources(&now, &["vt_e2e".to_owned()]), [2]);
        assert!(made_sources(&now, &[]).is_empty());
    }

    #[test]
    fn only_changed_at_risk_limits_are_put_back() {
        let before: Vec<Source> = [
            source(1, "plant:PMP1_RUN", "plant", 10.0, 20.0),
            source(2, "plant:PV_POWER", "plant", 1.0, 2.0),
            source(3, "plantPMP1_RUN", "plant", 5.0, 6.0),
        ]
        .iter()
        .map(Source::from)
        .collect();
        let alarms = [PreviewAlarm {
            tag_pattern: "PMP1_RUN".to_owned(),
            ..PreviewAlarm::default()
        }];
        let at_risk = at_risk_limits(&before, &alarms);
        assert_eq!(
            at_risk.iter().map(|source| source.id).collect::<Vec<_>>(),
            [1, 3],
            "the server's own match: `:pattern` or a key ending in it"
        );
        let now = [
            source(1, "plant:PMP1_RUN", "plant", 99.0, 20.0),
            // Changed by someone else: not at risk, left alone.
            source(2, "plant:PV_POWER", "plant", 7.0, 2.0),
            source(3, "plantPMP1_RUN", "plant", 5.0, 6.0),
        ];
        let back: Vec<i64> = changed_limits(&now, &at_risk)
            .iter()
            .map(|source| source.id)
            .collect();
        assert_eq!(back, [1]);
    }

    #[test]
    fn instance_ids_follow_the_instantiator() {
        assert_eq!(
            instance_flow_id("car_wash", "Pump Fault/Trip alarm"),
            "vt_car_wash_pump_fault_trip_alarm"
        );
        assert_eq!(
            instance_screen_id("car_wash", "e2e native 1f"),
            "car_wash_e2e_native_1f"
        );
    }

    #[test]
    fn the_day_shift_is_claimed_only_on_a_plant_without_shifts() {
        let after = [shift(4, "Day Shift", 6), shift(5, "Night", 18)];
        assert_eq!(instance_shifts(&[], &after), [4]);
        assert!(
            instance_shifts(&[shift(1, "Early", 5)], &after).is_empty(),
            "a plant with shifts gets none from the instance"
        );
        // Someone else's shift, not the instantiator's.
        assert!(instance_shifts(&[], &[shift(6, "Day Shift", 7)]).is_empty());
    }

    #[test]
    fn nothing_made_is_nothing_to_undo() {
        assert!(Made::default().is_empty());
        let made = Made {
            shifts: vec![1],
            ..Made::default()
        };
        assert!(!made.is_empty());
    }

    #[test]
    fn the_probe_recipient_cannot_receive_mail() {
        assert!(PROBE_RECIPIENT.ends_with(".invalid"));
        assert!(SmtpTestBody::new(PROBE_RECIPIENT).is_ok());
    }
}
