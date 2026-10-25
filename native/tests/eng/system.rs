//! Data Manager › System against the live plant: every path the section
//! calls. The template instance and the wizard's project are created and
//! then removed again, with every data source, flow rule, device, screen
//! and shift they made, and the limits the instance set on the plant's own
//! sources put back. The SMTP settings are changed and put back exactly
//! (the password stays: the server never sends it, and a PUT carrying its
//! mask keeps the stored one).

use std::collections::HashSet;

use serde::de::IgnoredAny;
use serde_json::{Value, json};
use voltrus_scada_native::api::eng_system::{
    AUTOMATION_PATH, Automation, DOCS_EXPORT_PATH, FEED_TEMPLATES_PATH, FeedTemplate,
    FeedTemplateList, HEALTH_PATH, HealthReport, InstantiateBody, Instantiated, PASSWORD_MASK,
    PreviewBody, SMTP_PATH, SMTP_TEST_PATH, SYSTEM_PATH, SmtpSettings, SmtpTestAnswer,
    SmtpTestBody, TAG_XREF_PATH, TEMPLATE_PREVIEW_PATH, TOPOLOGY_PATH, TagXref, TemplateList,
    TemplatePreview, TemplateSource, Topology, VerticalTemplate, WIZARD_CREATE_PATH, WizardDraft,
    WizardResult, feed_template_path, instance_device_id, instantiate_path, template_path,
    templates_path,
};
use voltrus_scada_native::api::ops::encode;
use voltrus_scada_native::api::{DataSource, Method, ServerError, VoltrusApi};

pub(super) async fn round_trip(api: &VoltrusApi) {
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

/// What the plant has, to find and remove what a template or the wizard
/// made.
#[derive(Debug, Clone, PartialEq)]
struct Plant {
    /// Data sources with the limits a template may change.
    sources: Vec<(i64, f64, f64)>,
    devices: HashSet<String>,
    flows: HashSet<String>,
    shifts: HashSet<i64>,
}

async fn plant(api: &VoltrusApi) -> Plant {
    let sources: Vec<DataSource> = api
        .get_json("/api/v1/data-sources")
        .await
        .expect("GET /data-sources");
    let devices: Vec<Value> = api.get_json("/api/v1/screens").await.expect("GET /screens");
    let flows: Value = api.get_json("/api/v1/flows").await.expect("GET /flows");
    let shifts: Vec<Value> = api
        .get_json("/api/v1/oee/shifts")
        .await
        .expect("GET /oee/shifts");
    Plant {
        sources: sources
            .iter()
            .map(|source| (source.id, source.warn_val, source.crit_val))
            .collect(),
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
        shifts: shifts
            .iter()
            .filter_map(|shift| shift["id"].as_i64())
            .collect(),
    }
}

/// Remove what was made since `before`, and put back the limits it
/// changed on the sources that were there.
async fn restore(api: &VoltrusApi, before: &Plant) {
    let after = plant(api).await;
    let old: HashSet<i64> = before.sources.iter().map(|(id, _, _)| *id).collect();
    for (id, _, _) in after.sources.iter().filter(|(id, _, _)| !old.contains(id)) {
        api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/data-sources/{id}"))
            .await
            .expect("delete a made data source");
    }
    for (id, warn, crit) in &before.sources {
        let now = after.sources.iter().find(|(other, _, _)| other == id);
        if now.is_some_and(|(_, w, c)| w != warn || c != crit) {
            api.send_json::<_, IgnoredAny>(
                Method::PUT,
                &format!("/api/v1/data-sources/{id}"),
                &json!({ "warn_val": warn, "crit_val": crit }),
            )
            .await
            .expect("put a source's limits back");
        }
    }
    for id in after.flows.difference(&before.flows) {
        api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/flows/{}", encode(id)))
            .await
            .expect("delete a made flow");
    }
    for id in after.devices.difference(&before.devices) {
        api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/screens/{}", encode(id)))
            .await
            .expect("delete a made device or screen");
    }
    for id in after.shifts.difference(&before.shifts) {
        api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/oee/shifts/{id}"))
            .await
            .expect("delete a made shift");
    }
    assert_eq!(plant(api).await, *before, "the plant is as it was");
}

/// `POST /templates/{id}/instantiate`, one instance, then all of it removed.
async fn instantiate(api: &VoltrusApi, template: &VerticalTemplate) {
    let before = plant(api).await;
    let name = format!("e2e native {:x}", std::process::id());
    assert!(
        !before.devices.contains(&instance_device_id(&name)),
        "no device of that name yet"
    );
    let body = InstantiateBody::new(&template.id, &name, "1").expect("an instantiate body");
    let made: Instantiated = api
        .send_json(Method::POST, &instantiate_path(&template.id), &body)
        .await
        .expect("POST /templates/{id}/instantiate");
    assert_eq!(made.template_id, template.id);
    assert_eq!(made.device_id, instance_device_id(&name));
    assert!(made.tags_created > 0, "the instance made tags");
    let during = plant(api).await;
    assert!(during.devices.contains(&made.device_id));
    assert!(during.devices.contains(&made.screen_id));
    println!(
        "instantiated {} as {}: {} tags, {} alarm limits, {} flows, screen {}",
        template.id,
        made.device_id,
        made.tags_created,
        made.alarms_created,
        made.flows_created,
        made.screen_id
    );
    restore(api, &before).await;
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
    restore(api, &before).await;
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
    match original.restore() {
        Some(back) => {
            assert_eq!(
                original.password.as_deref(),
                Some(PASSWORD_MASK),
                "the server never sends the password"
            );
            let mut changed = back.clone();
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
            api.send_json::<_, IgnoredAny>(Method::PUT, SMTP_PATH, &back)
                .await
                .expect("PUT /settings/smtp back");
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

    // The test email: no mail server listens in CI, so an error is fine;
    // the route must answer.
    let body = SmtpTestBody::new("e2e@plant.local").expect("a test body");
    match api
        .send_json::<_, SmtpTestAnswer>(Method::POST, SMTP_TEST_PATH, &body)
        .await
    {
        Ok(answer) => {
            assert!(answer.ok);
            println!("smtp test: {}", answer.text());
        }
        Err(error) => {
            let server = error
                .downcast_ref::<ServerError>()
                .unwrap_or_else(|| panic!("the server did not answer: {error}"));
            assert_ne!(
                server.status,
                reqwest::StatusCode::NOT_FOUND,
                "the test route exists"
            );
            assert!(!server.message.is_empty());
            println!(
                "smtp test answered {}: {}",
                server.status.as_u16(),
                server.message
            );
        }
    }
}
