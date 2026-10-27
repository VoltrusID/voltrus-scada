use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::params;

use super::export::read_all;
use super::*;
use crate::storage::Storage;

const SECRETS: &[&str] = &[
    "opc-secret-pw",
    "opcUrlSecret9",
    "urlTok3n",
    "sk-live-xyz",
    "nestedKey777",
    "flowSecretTok",
    "SLACKsecret",
    "WHsecretPath",
    "whAuthTok",
    "ftp-pass-1",
    "camera-pw",
    "hub-api-key-123",
    "HASH-DO-NOT-LEAK",
    "SALT-DO-NOT-LEAK",
    "APIKEYHASH-LEAK",
];

const SCRIPT_SOURCE: &str = "// scale\n\treturn x * 2;  \n  let s = `${PORT}`;\r\nünïcode ✓ \"quoted\" 'single'\n\nlast line without newline";

const SEED_SQL: &str = r#"
INSERT INTO modbus_devices (id,name,ip,port,unit_id,interval_ms,endianness,enabled,created_at,updated_at) VALUES
  ('plts_1','PLTS 1','192.168.1.10',502,1,1000,'big',1,100,100),
  ('plts_2','PLTS 2','192.168.1.11',1502,2,500,'little',0,100,100);
INSERT INTO modbus_registers (device_id,name,address,register_type,scale,sort_order,zero_based,register_area) VALUES
  ('plts_1','voltage',40001,'float32',1.0,0,0,'holding'),
  ('plts_1','current',40003,'float32',0.1,1,1,'input'),
  ('plts_2','power',40005,'uint16',1.0,0,0,'holding');
INSERT INTO opcua_devices (id,name,url,security_policy,security_mode,username,password,interval_ms,enabled,created_at) VALUES
  (1,'Inverter OPC','opc.tcp://svc:opcUrlSecret9@10.0.0.5:4840','Basic256','SignAndEncrypt','svc','opc-secret-pw',2000,1,100);
INSERT INTO opcua_nodes (device_id,node_id,name,display_name,data_type,created_at) VALUES
  (1,'ns=2;s=Temp','temp','Temperature','Double',100),(1,'ns=2;s=Pwr','power','','',101);
INSERT INTO s7_devices (id,name,url,rack,slot,interval_ms,enabled,created_at) VALUES (1,'Siemens PLC','s7://10.0.0.6',0,2,1000,1,100);
INSERT INTO s7_tags (device_id,tag_address,name,display_name,data_type,created_at) VALUES (1,'DB1.DBD0','speed','Speed','Real',100);
INSERT INTO enip_devices (id,name,url,slot,interval_ms,enabled,assembly_instance,created_at) VALUES (1,'Allen PLC','enip://10.0.0.7',1,1000,1,100,100);
INSERT INTO enip_tags (device_id,tag_name,name,display_name,data_type,assembly_offset,assembly_length,created_at) VALUES (1,'Motor.Speed','motor_speed','','REAL',4,4,100);
INSERT INTO dnp3_devices (id,name,ip,port,master_address,outstation_address,enable_unsolicited,integrity_poll_interval_secs,event_scan_interval_ms,enabled,created_at,updated_at) VALUES (1,'Outstation 10','10.0.0.8',20000,1,10,1,300,NULL,1,100,100);
INSERT INTO dnp3_points (device_id,group_num,variation,point_index,name,display_name,data_type,scale,deadband,created_at) VALUES (1,30,1,0,'ai0','Analog 0','float64',0.5,0.25,100);
INSERT INTO bacnet_devices (id,name,ip,port,device_instance,interval_ms,cov_enabled,cov_lifetime,enabled,created_at) VALUES (1,'AHU 1','10.0.0.9',47808,1001,5000,1,3600,1,100);
INSERT INTO bacnet_objects (device_id,object_type,object_instance,property,name,display_name,cov_increment,created_at) VALUES (1,'analogInput',1,'presentValue','sat','Supply Air Temp',0.5,100);
INSERT INTO voltrus_plc_devices (id,name,url,transport,interval_ms,enabled,created_at) VALUES (1,'Voltrus PLC','plc://10.0.0.10','grpc',1000,1,100);
INSERT INTO voltrus_plc_tags (device_id,variable_name,display_name,data_type,created_at) VALUES (1,'MAIN.counter','Counter','DINT',100);
INSERT INTO sim_profiles (id,name,data_type,config,created_at) VALUES (1,'noisy','float','{"period":30,"amp":2}',100);
INSERT INTO device_templates (template_id,source,version,manufacturer,model,protocol,verified,body,created_at) VALUES
 ('acme-pm100','local','1.0.0','Acme','PM100','modbus',1,'{"format":"voltrus-device-template/1","template_id":"acme-pm100","manufacturer":"Acme","model":"PM100","protocol":"modbus","description":"Power meter","version":"1.0.0","verified":true,"verified_notes":"Firmware 2.1 at site A","quirks":["Word order is swapped","Offsets are 0-based"],"source":"local","license":"CC-BY-4.0","port":1502,"interval_ms":2000,"unit_id":3,"endianness":"swapped","registers":[{"name":"voltage","offset":0,"register_type":"float32","register_area":"holding","scale":1.0,"unit":"V","description":"Phase voltage"},{"name":"energy","offset":2,"register_type":"int32","register_area":"input","scale":0.1,"writable":true}]}',100),
 ('acme-pm100','local','1.1.0','Acme','PM100','modbus',0,'{"format":"voltrus-device-template/1","template_id":"acme-pm100","manufacturer":"Acme","model":"PM100","protocol":"modbus","version":"1.1.0","source":"local","port":502,"interval_ms":1000,"unit_id":1,"endianness":"big","registers":[{"name":"voltage","offset":0}]}',101),
 ('vav-1','catalog:https://catalog.example.com/index.json','2.0.0','Globex','VAV-1','bacnet',0,'{"format":"voltrus-device-template/1","template_id":"vav-1","manufacturer":"Globex","model":"VAV-1","protocol":"bacnet","version":"2.0.0","source":"catalog:https://catalog.example.com/index.json","license":"CC0-1.0","port":47808,"interval_ms":5000,"cov_enabled":true,"cov_lifetime":600,"objects":[{"object_type":"analogInput","object_instance":1,"property":"presentValue","name":"room_temp","display_name":"Room temperature","cov_increment":0.5,"unit":"degC"},{"object_type":"binaryValue","object_instance":2,"name":"occupied","writable":true}]}',102);

INSERT INTO data_sources (id,key,name,unit,color,min_val,max_val,warn_val,crit_val,base_val,sort_order,decimals,source_type,source_config,device_id,equipment_id,chart_type,writable,write_min,write_max,write_max_rate,write_requires_reason,write_deadband,sim_value,path,on_delay_secs,off_delay_secs) VALUES
 (1,'voltage','Voltage','V','#f59e0b',0,450,400,420,380,0,1,'modbus','{}','plts_1',NULL,'sparkline',0,NULL,NULL,NULL,0,NULL,NULL,'site/plts1/voltage',5,10),
 (2,'current','Current','A','#06b6d4',0,100,85,95,50,1,1,'modbus','{"low_warn":5,"low_crit":2,"deadband":1.5,"expression":"x*2"}','plts_1','eq1','bar',1,0,100,5,1,0.5,12.5,NULL,0,0),
 (3,'power','Power','kW','#10b981',0,500,400,450,100,2,0,'modbus','{}','plts_1',NULL,'sparkline',0,NULL,NULL,NULL,0,NULL,NULL,NULL,0,0),
 (4,'power','Power duplicate','kW','#10b981',0,500,400,450,100,3,0,'modbus','{}','plts_1',NULL,'sparkline',0,NULL,NULL,NULL,0,NULL,NULL,NULL,0,0),
 (5,'Inverter OPC:temp','Temperature','','#f97316',0,0,0,0,0,4,1,'opcua','{}','Inverter OPC',NULL,'sparkline',0,NULL,NULL,NULL,0,NULL,NULL,NULL,0,0),
 (6,'ext','External API','','#ffffff',0,1,0,0,0,5,2,'api','{"url":"https://api.example.com/v1?token=urlTok3n&x=1","headers":{"Authorization":"Bearer sk-live-xyz","Accept":"json"},"nested":{"api_key":"nestedKey777"}}','plts_2',NULL,'sparkline',0,NULL,NULL,NULL,0,NULL,NULL,NULL,0,0);

INSERT INTO devices (id,name,type,sort_order,simulation_mode,sim_profile_id,sim_config) VALUES
  ('plts_1','PLTS 1','screen',0,1,1,'{"seed":7}'),('plts_2','PLTS 2','screen',1,0,NULL,NULL);
INSERT INTO screen_data_sources (screen_id,data_source_id,sort_order) VALUES ('plts_1',1,0),('plts_1',2,1),('plts_1',3,2),('plts_2',6,0),('plts_2',5,1);
INSERT INTO screen_layouts (screen_id,breakpoint,layout) VALUES
  ('plts_1','lg','[{"i":"1","x":0,"y":0,"w":4,"h":3},{"i":"2","x":4,"y":0,"w":4,"h":3},{"i":"999","x":0,"y":3,"w":2,"h":2}]'),
  ('plts_1','md','[{"i":"1","x":0,"y":0,"w":6,"h":3}]'),
  ('__overview','lg','[{"i":"3","x":0,"y":0,"w":4,"h":3}]');
INSERT INTO canvas_layouts (screen_id,data_source_id,x,y,width,height) VALUES ('plts_1',1,10,20,300,200),('plts_1',2,50.5,60,320,180);
INSERT INTO twin_layouts (screen_id,equipment_id,equipment_type,position_x,position_z,rotation_y,equipment_name) VALUES ('plts_1','inv1','inverter',1.5,2.5,90,'Inverter 1');
INSERT INTO twin_connections (screen_id,from_id,to_id) VALUES ('plts_1','inv1','inv2');
INSERT INTO scada_equipment (screen_id,equipment_id,equipment_type,equipment_name,x,y,width,height,rotation,sensor_bindings,config) VALUES
  ('plts_1','w1','gauge','Voltage gauge',10,20,100,60,0,'[1,2]','{"z":1,"label":"V"}'),
  ('plts_1','w2','tank','',0,0,100,60,45,'[]','{}');
INSERT INTO scada_connections (screen_id,connection_id,from_equipment,to_equipment,waypoints,connection_type) VALUES ('plts_1','c1','w1','w2','[{"x":1,"y":2}]','power');
INSERT INTO settings (key,value) VALUES ('twin_camera:plts_1','{"x":8,"y":6,"z":4}'),('alarm_cooldown_secs','300'),
  ('naming_template','{site}.{area}.{tag}'),('naming_regex','^[a-z]+$'),('oee_threshold_plts_1','{"metric":"power","running_above":1.5}'),
  ('user:alice:theme','dark'),('modbus_yaml_migrated','true');
UPDATE security_thresholds SET failed_login_threshold = 7 WHERE id = 1;
INSERT INTO compliance_config (id,config) VALUES (1,'{"enabled":true,"require_reason":["write"]}');

INSERT INTO flows (id,name,enabled,trigger_config,nodes,created_at,updated_at) VALUES
 ('f1','Boost',1,'{"type":"data_change","device_id":"plts_1","metric":"voltage"}',
  '[{"type":"write_out","id":"n1","data_source_id":2,"policy":"immediate","reason":"flow:f1"},{"type":"script","id":"n2","script_id":1,"inputs":["a"]},{"type":"http_out","id":"n3","url":"https://hooks.example.com/x","headers":{"Authorization":"Bearer flowSecretTok"},"timeout_ms":1000,"retry_count":1}]',100,100);
INSERT INTO recipes (name,category,device_id,"values",version,description,created_at,updated_at) VALUES ('Batch A','baking','plts_1','{"setpoint":42}',2,'first',100,100);
INSERT INTO notification_settings (source_key,channel_type,config,enabled,created_at) VALUES
 ('voltage','email','["ops@example.com"]',1,100),
 ('voltage','webhook','["https://hooks.slack.com/services/T0/B0/SLACKsecret"]',1,100),
 ('voltage','email','["b@example.com"]',0,101);
INSERT INTO outbound_webhooks (name,url,events,device_filter,headers,enabled,retry_count,timeout_ms,created_at) VALUES
 ('Slack','https://hooks.slack.com/services/T1/B1/WHsecretPath','["alarm","tag_change"]','"all"','{"Authorization":"Bearer whAuthTok","Content-Type":"application/json"}',1,3,10000,100);
INSERT INTO egress_jobs (name,enabled,schedule,format,destination,device_ids,time_range,compression,created_at,updated_at) VALUES ('Nightly',1,'daily','csv','sftp://bob:ftp-pass-1@files.example.com/out','all','24h',1,100,100);
INSERT INTO reports (name,schedule,enabled,device_ids,email_recipients,created_at) VALUES ('Daily','0 6 * * *',1,'all','a@x.com,b@x.com',100);
INSERT INTO cameras (name,url,enabled,snapshot_on_alarm,retention_hours,created_at) VALUES ('Cam 1','rtsp://cam:camera-pw@10.0.0.9/stream',1,1,48,100);
INSERT INTO hub_sites (name,url,api_key,enabled,created_at) VALUES ('Site B','https://site-b.example.com','hub-api-key-123',1,100);
INSERT INTO equipment_templates (name,industry,equipment_type,tag_template,alarm_template,dashboard_template,flow_template) VALUES ('Inverter','solar','inverter','[{"key":"p"}]','[]','{"cols":2}','[]');
INSERT INTO shifts (name,start_hour,start_minute,duration_hours,days_mask,created_at) VALUES ('Morning',6,0,8,127,100);
INSERT INTO custom_metrics (name,description,formula,unit,tag_refs,validation_error,created_at,updated_at) VALUES ('eff','Efficiency','current / voltage','%','[]',NULL,100,100);
INSERT INTO assets (id,device_ref,name,asset_type,vendor,criticality,tags,notes,created_at,updated_at) VALUES ('a1','plts_1','Inverter','plc','Acme','high','["site-a"]','note','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z');

-- Runtime and credential data that must never reach a project file.
INSERT INTO users (username,password_hash,salt,role) VALUES ('alice','HASH-DO-NOT-LEAK','SALT-DO-NOT-LEAK','admin');
INSERT INTO api_keys (name,key_hash,key_prefix,created_at,created_by,permissions) VALUES ('ci','APIKEYHASH-LEAK','vk_',1,'alice','[]');
INSERT INTO telemetry (device_id,timestamp,metric,value) VALUES ('plts_1',1,'voltage',230.5);
INSERT INTO alarms (source_key,severity,value,threshold,message,triggered_at) VALUES ('voltage','warning',410,400,'High voltage',5);
"#;

async fn seeded() -> Storage {
    let storage = Storage::new_in_memory().unwrap();
    storage
        .with_conn(|conn| {
            conn.execute_batch(SEED_SQL)?;
            conn.execute(
                "INSERT INTO scripts (id,name,source,syntax_error,cpu_limit_ms,memory_limit_bytes,enabled,created_at,updated_at) VALUES (1,'scale',?1,NULL,500,2097152,1,100,100)",
                params![SCRIPT_SOURCE],
            )?;
            conn.execute(
                "INSERT INTO scripts (id,name,source,syntax_error,cpu_limit_ms,memory_limit_bytes,enabled,created_at,updated_at) VALUES (2,'broken','(','boom',1000,1048576,0,100,100)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    storage
}

fn fresh() -> Storage {
    Storage::new_in_memory().unwrap()
}

/// Every `${VAR}` an export of `storage` mentions, with the real secret behind it.
async fn env_for(storage: &Storage) -> Env {
    let map: BTreeMap<String, String> = storage
        .with_conn(|conn| {
            let (snap, _) = read_all(conn)?;
            let mut out = BTreeMap::new();
            for sec in spec::SECTIONS {
                for row in snap.get(sec.name).map(Vec::as_slice).unwrap_or(&[]) {
                    let mut e = row.entity.clone();
                    out.extend(secrets::redact(sec.name, &row.label(), &mut e));
                }
            }
            Ok(out)
        })
        .await
        .unwrap();
    Arc::new(move |name: &str| map.get(name).cloned())
}

fn no_env() -> Env {
    Arc::new(|_| None)
}

async fn export_text(storage: &Storage) -> String {
    let mut doc = export_project_yaml(storage).await.unwrap();
    doc.generated_at = "2026-01-01T00:00:00Z".into();
    to_yaml(&doc)
}

async fn count(storage: &Storage, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    storage
        .with_conn(move |c| Ok(c.query_row(&sql, [], |r| r.get::<_, i64>(0))?))
        .await
        .unwrap()
}

async fn scalar_i64(storage: &Storage, sql: &str) -> i64 {
    let sql = sql.to_string();
    storage
        .with_conn(move |c| Ok(c.query_row(&sql, [], |r| r.get::<_, i64>(0))?))
        .await
        .unwrap()
}

async fn scalar_text(storage: &Storage, sql: &str) -> String {
    let sql = sql.to_string();
    storage
        .with_conn(move |c| Ok(c.query_row(&sql, [], |r| r.get::<_, String>(0))?))
        .await
        .unwrap()
}

const CONFIG_TABLES: &[&str] = &[
    "modbus_devices",
    "modbus_registers",
    "opcua_devices",
    "opcua_nodes",
    "s7_devices",
    "s7_tags",
    "enip_devices",
    "enip_tags",
    "dnp3_devices",
    "dnp3_points",
    "bacnet_devices",
    "bacnet_objects",
    "voltrus_plc_devices",
    "voltrus_plc_tags",
    "device_templates",
    "sim_profiles",
    "scripts",
    "data_sources",
    "devices",
    "screen_data_sources",
    "screen_layouts",
    "canvas_layouts",
    "twin_layouts",
    "twin_connections",
    "scada_equipment",
    "scada_connections",
    "flows",
    "recipes",
    "notification_settings",
    "outbound_webhooks",
    "egress_jobs",
    "reports",
    "cameras",
    "hub_sites",
    "equipment_templates",
    "shifts",
    "custom_metrics",
    "assets",
    "compliance_config",
];

async fn table_counts(storage: &Storage) -> BTreeMap<&'static str, i64> {
    let mut out = BTreeMap::new();
    for t in CONFIG_TABLES {
        out.insert(*t, count(storage, t).await);
    }
    out
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "voltrus-project-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn read_tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, root, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read_to_string(&p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn invalid(err: &anyhow::Error) -> &InvalidProject {
    err.chain()
        .find_map(|c| c.downcast_ref::<InvalidProject>())
        .expect("an InvalidProject error")
}

// ── Round trip ──

#[tokio::test]
async fn round_trip_is_byte_identical() {
    let a = seeded().await;
    let first = export_text(&a).await;

    let doc = parse_project_yaml(&first).unwrap();
    let b = fresh();
    let plan = apply_import_with_env(&b, &doc, Mode::Merge, env_for(&a).await)
        .await
        .unwrap();
    assert!(plan.totals.create > 0 && plan.totals.update == 0 && plan.totals.delete == 0);

    let second = export_text(&b).await;
    assert_eq!(
        first, second,
        "export -> import -> export must not change a byte"
    );

    // And once more through a second hop, with the plan reporting nothing to do.
    let doc2 = parse_project_yaml(&second).unwrap();
    let plan = plan_import_with_env(&b, &doc2, Mode::Replace, env_for(&a).await)
        .await
        .unwrap();
    assert!(!plan.has_changes(), "unexpected changes: {}", plan.render());
    assert_eq!(table_counts(&a).await, table_counts(&b).await);
}

#[tokio::test]
async fn export_covers_every_entity_kind() {
    let a = seeded().await;
    let doc = export_project_yaml(&a).await.unwrap();
    for sec in spec::SECTIONS {
        assert!(
            !doc.entities(sec.name).is_empty(),
            "section {} is empty in the export of the seeded project",
            sec.name
        );
    }
    assert_eq!(doc.format, FORMAT);

    let tags = doc.entities("tags");
    let refs: Vec<String> = tags
        .iter()
        .map(|t| super::export::entity_key(spec::section("tags").unwrap(), t))
        .collect();
    assert!(refs.contains(&"plts_1/power".to_string()));
    assert!(
        refs.contains(&"plts_1/power#2".to_string()),
        "duplicate tags get an ordinal ref: {refs:?}"
    );
}

#[tokio::test]
async fn tag_alarm_write_and_source_config_are_structured() {
    let a = seeded().await;
    let text = export_text(&a).await;
    let doc = parse_project_yaml(&text).unwrap();
    let current = doc
        .entities("tags")
        .iter()
        .find(|t| t.get("key").and_then(|v| v.as_str()) == Some("current"))
        .unwrap();
    let alarm = current.get("alarm").unwrap().as_mapping().unwrap();
    assert_eq!(alarm.get("warn").unwrap().as_f64(), Some(85.0));
    assert_eq!(alarm.get("low_warn").unwrap().as_f64(), Some(5.0));
    assert_eq!(alarm.get("deadband").unwrap().as_f64(), Some(1.5));
    let write = current.get("write").unwrap().as_mapping().unwrap();
    assert_eq!(write.get("writable").unwrap().as_bool(), Some(true));
    assert_eq!(write.get("max_rate").unwrap().as_i64(), Some(5));
    // source_config is a mapping, not a JSON string, and no longer carries the alarm keys.
    let cfg = current.get("source_config").unwrap().as_mapping().unwrap();
    assert!(cfg.get("expression").is_some());
    assert!(cfg.get("low_warn").is_none());

    // The voltage tag carries the ISA-18.2 delays.
    let voltage = doc
        .entities("tags")
        .iter()
        .find(|t| t.get("key").and_then(|v| v.as_str()) == Some("voltage"))
        .unwrap();
    let alarm = voltage.get("alarm").unwrap().as_mapping().unwrap();
    assert_eq!(alarm.get("on_delay_secs").unwrap().as_i64(), Some(5));
    assert_eq!(alarm.get("off_delay_secs").unwrap().as_i64(), Some(10));
}

#[tokio::test]
async fn import_recreates_everything_with_cross_references_intact() {
    let a = seeded().await;
    let doc = parse_project_yaml(&export_text(&a).await).unwrap();
    let b = fresh();
    apply_import_with_env(&b, &doc, Mode::Merge, env_for(&a).await)
        .await
        .unwrap();

    assert_eq!(table_counts(&a).await, table_counts(&b).await);

    // Tags: duplicate (device, key) pairs kept apart, ids follow document order.
    assert_eq!(
        scalar_i64(
            &b,
            "SELECT COUNT(*) FROM data_sources WHERE device_id='plts_1' AND key='power'"
        )
        .await,
        2
    );
    // Screen membership resolves to the right tags.
    assert_eq!(
        scalar_text(
            &b,
            "SELECT group_concat(d.key, ',') FROM (SELECT ds.key FROM screen_data_sources s JOIN data_sources ds ON ds.id = s.data_source_id WHERE s.screen_id='plts_1' ORDER BY s.sort_order) d"
        )
        .await,
        "voltage,current,power"
    );
    // Layout items point at the new ids of the same tags; a dangling id survives verbatim.
    let layout = scalar_text(
        &b,
        "SELECT layout FROM screen_layouts WHERE screen_id='plts_1' AND breakpoint='lg'",
    )
    .await;
    let layout: serde_json::Value = serde_json::from_str(&layout).unwrap();
    let voltage_id = scalar_i64(&b, "SELECT id FROM data_sources WHERE key='voltage'").await;
    let current_id = scalar_i64(&b, "SELECT id FROM data_sources WHERE key='current'").await;
    let ids: Vec<&str> = layout
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["i"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&voltage_id.to_string().as_str()));
    assert!(ids.contains(&current_id.to_string().as_str()));
    assert!(ids.contains(&"999"));
    // SCADA widget bindings, canvas, flow nodes and the simulation profile.
    let bindings = scalar_text(
        &b,
        "SELECT sensor_bindings FROM scada_equipment WHERE equipment_id='w1'",
    )
    .await;
    assert_eq!(bindings, format!("[{voltage_id},{current_id}]"));
    assert_eq!(
        scalar_i64(
            &b,
            "SELECT data_source_id FROM canvas_layouts WHERE x = 50.5"
        )
        .await,
        current_id
    );
    let nodes = scalar_text(&b, "SELECT nodes FROM flows WHERE id='f1'").await;
    let nodes: serde_json::Value = serde_json::from_str(&nodes).unwrap();
    assert_eq!(nodes[0]["data_source_id"], current_id);
    let script_id = scalar_i64(&b, "SELECT id FROM scripts WHERE name='scale'").await;
    assert_eq!(nodes[1]["script_id"], script_id);
    assert_eq!(
        scalar_text(&b, "SELECT p.name FROM devices d JOIN sim_profiles p ON p.id = d.sim_profile_id WHERE d.id='plts_1'").await,
        "noisy"
    );
    // Secrets came back from the environment, byte for byte.
    assert_eq!(
        scalar_text(&b, "SELECT password FROM opcua_devices").await,
        "opc-secret-pw"
    );
    assert_eq!(
        scalar_text(&b, "SELECT url FROM opcua_devices").await,
        "opc.tcp://svc:opcUrlSecret9@10.0.0.5:4840"
    );
    assert_eq!(
        scalar_text(&b, "SELECT api_key FROM hub_sites").await,
        "hub-api-key-123"
    );
    assert_eq!(
        scalar_text(
            &b,
            "SELECT config FROM notification_settings WHERE channel_type='webhook'"
        )
        .await,
        "[\"https://hooks.slack.com/services/T0/B0/SLACKsecret\"]"
    );
    // Script text is lossless and its syntax state is recomputed.
    assert_eq!(
        scalar_text(&b, "SELECT source FROM scripts WHERE name='scale'").await,
        SCRIPT_SOURCE
    );
    let scale_flagged = crate::script_engine::validate_syntax(SCRIPT_SOURCE).is_some() as i64;
    assert_eq!(
        scalar_i64(
            &b,
            "SELECT syntax_error IS NOT NULL FROM scripts WHERE name='scale'"
        )
        .await,
        scale_flagged
    );
    assert_eq!(
        scalar_i64(
            &b,
            "SELECT syntax_error IS NOT NULL FROM scripts WHERE name='broken'"
        )
        .await,
        1
    );
    // Settings whitelist.
    assert_eq!(
        scalar_text(
            &b,
            "SELECT value FROM settings WHERE key='alarm_cooldown_secs'"
        )
        .await,
        "300"
    );
    assert_eq!(
        scalar_text(
            &b,
            "SELECT value FROM settings WHERE key='twin_camera:plts_1'"
        )
        .await,
        r#"{"x":8,"y":6,"z":4}"#
    );
    assert_eq!(
        scalar_i64(
            &b,
            "SELECT failed_login_threshold FROM security_thresholds WHERE id=1"
        )
        .await,
        7
    );
    // User preferences and migration markers are not configuration.
    assert_eq!(
        scalar_i64(
            &b,
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'user:%' OR key='modbus_yaml_migrated'"
        )
        .await,
        0
    );
    // Derived custom-metric columns were computed.
    assert_eq!(
        scalar_text(&b, "SELECT tag_refs FROM custom_metrics").await,
        "[\"current\",\"voltage\"]"
    );
}

// ── Files ──

#[tokio::test]
async fn directory_form_round_trips_and_is_stable() {
    let a = seeded().await;
    let mut doc = export_project_yaml(&a).await.unwrap();
    doc.generated_at = "2026-01-01T00:00:00Z".into();

    let dir1 = temp_dir("dir1");
    write_project_dir(&doc, &dir1).unwrap();
    assert!(dir1.join("project.yaml").is_file());
    assert!(dir1.join("devices/modbus/plts_1.yaml").is_file());
    assert!(dir1.join("tags/plts_1.yaml").is_file());
    assert!(dir1.join("screens/plts_1.yaml").is_file());
    assert!(dir1.join("flows/f1.yaml").is_file());
    assert!(dir1.join("scripts/scale.yaml").is_file());
    assert!(dir1.join("settings.yaml").is_file());

    let read = read_project(&dir1).unwrap();
    assert_eq!(read.sections, doc.sections);
    assert_eq!(to_yaml(&read), to_yaml(&doc));

    // Import from the directory, export again, write a second directory: same bytes.
    let b = fresh();
    apply_import_with_env(&b, &read, Mode::Merge, env_for(&a).await)
        .await
        .unwrap();
    let mut doc_b = export_project_yaml(&b).await.unwrap();
    doc_b.generated_at = doc.generated_at.clone();
    let dir2 = temp_dir("dir2");
    write_project_dir(&doc_b, &dir2).unwrap();
    assert_eq!(read_tree(&dir1), read_tree(&dir2));

    // Re-exporting over an existing directory removes stale managed files.
    std::fs::write(
        dir1.join("flows/stale.yaml"),
        "id: gone\nname: gone\ntrigger: {}\nnodes: []\n",
    )
    .unwrap();
    write_project_dir(&doc, &dir1).unwrap();
    assert!(!dir1.join("flows/stale.yaml").exists());
    assert_eq!(read_tree(&dir1), read_tree(&dir2));

    // Single-file form too.
    let file = temp_dir("single").with_extension("yaml");
    write_project(&doc, &file).unwrap();
    assert_eq!(read_project(&file).unwrap().sections, doc.sections);
    let _ = std::fs::remove_dir_all(&dir1);
    let _ = std::fs::remove_dir_all(&dir2);
    let _ = std::fs::remove_file(&file);
}

// ── Merge / replace / dry run ──

fn doc_of(text: &str) -> ProjectDoc {
    parse_project_yaml(text).unwrap()
}

const SMALL: &str = r#"
format: voltrus-project/1
modbus_devices:
- id: d1
  name: Device 1
  ip: 10.0.0.1
  registers:
  - name: v
    address: 40001
tags:
- key: v
  device: d1
  name: V
  min: 0
  max: 10
  base: 5
screens:
- id: d1
  name: Device 1
  tags:
  - ref: d1/v
flows:
- id: keep
  name: Keep
  trigger: {type: manual}
  nodes: []
"#;

#[tokio::test]
async fn merge_keeps_unlisted_entities_replace_deletes_them() {
    let s = fresh();
    s.with_conn(|c| {
        c.execute_batch(
            "INSERT INTO modbus_devices (id,name,ip,created_at,updated_at) VALUES ('old','Old','1.1.1.1',1,1);
             INSERT INTO flows (id,name,trigger_config,nodes) VALUES ('stale','Stale','{}','[]');
             INSERT INTO data_sources (key,name,min_val,max_val,warn_val,crit_val,base_val,device_id) VALUES ('x','X',0,1,0,0,0,'old');
             INSERT INTO screen_data_sources (screen_id,data_source_id,sort_order) VALUES ('old',1,0);
             INSERT INTO settings (key,value) VALUES ('alarm_cooldown_secs','60');",
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let doc = doc_of(SMALL);
    let plan = plan_import(&s, &doc, Mode::Merge).await.unwrap();
    assert_eq!(plan.totals.delete, 0);
    assert_eq!(plan.totals.create, 4);
    apply_import(&s, &doc, Mode::Merge).await.unwrap();
    assert_eq!(
        count(&s, "modbus_devices").await,
        2,
        "merge leaves the old device"
    );
    assert_eq!(count(&s, "flows").await, 2);
    assert_eq!(count(&s, "data_sources").await, 2);
    assert_eq!(
        scalar_text(
            &s,
            "SELECT value FROM settings WHERE key='alarm_cooldown_secs'"
        )
        .await,
        "60"
    );

    let plan = plan_import(&s, &doc, Mode::Replace).await.unwrap();
    let deletes: Vec<String> = plan
        .changes
        .iter()
        .filter(|c| c.action == Action::Delete)
        .map(|c| format!("{}:{}", c.section, c.key))
        .collect();
    assert_eq!(
        deletes,
        vec![
            "modbus_devices:old",
            "tags:old/x",
            "screens:old",
            "flows:stale",
            "settings:settings"
        ]
    );
    assert_eq!(
        plan.totals.unchanged, 4,
        "the project's own entities are untouched"
    );
    // The plan alone changed nothing.
    assert_eq!(count(&s, "modbus_devices").await, 2);

    apply_import(&s, &doc, Mode::Replace).await.unwrap();
    assert_eq!(count(&s, "modbus_devices").await, 1);
    assert_eq!(count(&s, "flows").await, 1);
    assert_eq!(count(&s, "data_sources").await, 1);
    assert_eq!(
        count(&s, "screen_data_sources").await,
        1,
        "membership rows of deleted screens go too"
    );
    assert_eq!(
        scalar_i64(
            &s,
            "SELECT COUNT(*) FROM settings WHERE key='alarm_cooldown_secs'"
        )
        .await,
        0,
        "replace resets whitelisted settings the project does not mention"
    );
}

#[tokio::test]
async fn update_changes_only_what_differs_and_keeps_database_ids() {
    let s = fresh();
    apply_import(&s, &doc_of(SMALL), Mode::Merge).await.unwrap();
    let tag_id = scalar_i64(&s, "SELECT id FROM data_sources WHERE key='v'").await;

    let changed = SMALL
        .replace("max: 10", "max: 20")
        .replace("address: 40001", "address: 40005");
    let plan = apply_import(&s, &doc_of(&changed), Mode::Merge)
        .await
        .unwrap();
    assert_eq!(plan.totals.update, 2);
    assert_eq!(plan.totals.unchanged, 2);
    let tag = plan.changes.iter().find(|c| c.section == "tags").unwrap();
    assert_eq!(tag.action, Action::Update);
    assert_eq!(tag.fields, vec!["max"]);
    assert_eq!(
        scalar_i64(&s, "SELECT id FROM data_sources WHERE key='v'").await,
        tag_id
    );
    assert_eq!(
        scalar_i64(&s, "SELECT address FROM modbus_registers").await,
        40005
    );
    assert_eq!(
        scalar_i64(&s, "SELECT CAST(max_val AS INTEGER) FROM data_sources").await,
        20
    );

    // Importing the same project again is a no-op.
    let plan = apply_import(&s, &doc_of(&changed), Mode::Replace)
        .await
        .unwrap();
    assert!(!plan.has_changes());
}

#[tokio::test]
async fn dry_run_writes_nothing() {
    let a = seeded().await;
    let doc = doc_of(&export_text(&a).await);
    let b = fresh();
    let before = export_text(&b).await;
    let counts = table_counts(&b).await;

    let plan = plan_import_with_env(&b, &doc, Mode::Replace, env_for(&a).await)
        .await
        .unwrap();
    assert!(plan.dry_run);
    assert!(plan.totals.create > 20);
    assert!(plan.render().contains("+ create tags: plts_1/voltage"));
    assert_eq!(table_counts(&b).await, counts);
    assert_eq!(export_text(&b).await, before);
    // No row sequence was consumed either: a later real import still starts at id 1.
    apply_import_with_env(&b, &doc, Mode::Merge, env_for(&a).await)
        .await
        .unwrap();
    assert_eq!(scalar_i64(&b, "SELECT MIN(id) FROM data_sources").await, 1);
}

#[tokio::test]
async fn failed_import_rolls_everything_back() {
    let s = fresh();
    let bad = format!(
        "{SMALL}notifications:\n- source_key: v\n  channel_type: carrier-pigeon\n  config: []\n"
    );
    let err = apply_import(&s, &doc_of(&bad), Mode::Merge)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("notification rule"), "{err:#}");
    for t in CONFIG_TABLES {
        assert_eq!(
            count(&s, t).await,
            0,
            "{t} must be untouched after a failed import"
        );
    }
}

// ── Validation ──

#[tokio::test]
async fn validation_catches_dangling_references_and_duplicates() {
    let text = format!(
        "{SMALL}\
scripts:
- name: s1
  source: return 1;
- name: s1
  source: return 2;
"
    )
    .replace("  - ref: d1/v", "  - ref: d1/v\n  - ref: d1/ghost")
    .replace(
        "  nodes: []",
        "  nodes:\n  - {type: write_out, id: n, tag: d1/missing}\n  - {type: script, id: m, script: nope}",
    );
    let doc = doc_of(&text);
    let issues = validate_doc(&doc, None, false);
    let errors: Vec<String> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.to_string())
        .collect();
    let joined = errors.join("\n");
    assert!(joined.contains("unknown tag 'd1/ghost'"), "{joined}");
    assert!(joined.contains("unknown tag 'd1/missing'"), "{joined}");
    assert!(joined.contains("unknown script 'nope'"), "{joined}");
    assert!(joined.contains("duplicate script 's1'"), "{joined}");

    // The import refuses it and writes nothing.
    let s = fresh();
    let err = apply_import(&s, &doc, Mode::Merge).await.unwrap_err();
    assert!(invalid(&err).0.len() >= 4);
    assert_eq!(count(&s, "modbus_devices").await, 0);

    // A merge may refer to entities that already exist; a replace may not.
    let partial = doc_of(
        "format: voltrus-project/1\nscreens:\n- id: d1\n  name: D\n  tags:\n  - ref: d1/v\n",
    );
    let s = fresh();
    apply_import(&s, &doc_of(SMALL), Mode::Merge).await.unwrap();
    plan_import(&s, &partial, Mode::Merge).await.unwrap();
    let err = plan_import(&s, &partial, Mode::Replace).await.unwrap_err();
    assert!(format!("{err}").contains("unknown tag 'd1/v'"));
}

#[test]
fn parse_rejects_schema_violations_with_paths() {
    let err = parse_project_yaml("format: voltrus-project/2\n").unwrap_err();
    assert!(format!("{err}").contains("unsupported format"));
    let err = parse_project_yaml("modbus_devices: []\n").unwrap_err();
    assert!(format!("{err}").contains("missing 'format'"));
    let err = parse_project_yaml("format: voltrus-project/1\nwidgets: []\n").unwrap_err();
    assert!(format!("{err}").contains("unknown top-level key"));

    let err = parse_project_yaml(
        "format: voltrus-project/1\nmodbus_devices:\n- id: d\n  name: D\n  ip: 1.1.1.1\n  port: many\n  colour: red\n  registers:\n  - name: r\n",
    )
    .unwrap_err();
    let text = invalid(&err)
        .0
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("port: expected an integer"), "{text}");
    assert!(text.contains("unknown field 'colour'"), "{text}");
    assert!(
        text.contains("registers[0].address: missing required field"),
        "{text}"
    );
}

#[test]
fn parse_fills_defaults_and_normalises_numbers() {
    let doc = parse_project_yaml(
        "format: voltrus-project/1\nmodbus_devices:\n- id: d\n  name: D\n  ip: 1.1.1.1\n  registers:\n  - {name: r, address: 1, scale: 2}\ntags:\n- {key: k, name: K, min: 1, max: 2, base: 3, id_hint: 77}\n",
    )
    .unwrap();
    let d = &doc.entities("modbus_devices")[0];
    assert_eq!(d.get("port").unwrap().as_i64(), Some(502));
    assert_eq!(d.get("enabled").unwrap().as_bool(), Some(true));
    let reg = d.get("registers").unwrap().as_sequence().unwrap()[0]
        .as_mapping()
        .unwrap();
    assert_eq!(reg.get("scale").unwrap().as_f64(), Some(2.0));
    assert!(
        reg.get("scale").unwrap().is_f64(),
        "integers in real fields become floats"
    );
    assert_eq!(reg.get("register_type").unwrap().as_str(), Some("float32"));
    let t = &doc.entities("tags")[0];
    assert_eq!(t.get("device").unwrap().as_str(), Some("default"));
    assert!(
        t.get("alarm").is_none(),
        "an all-default alarm block is not written"
    );
}

#[tokio::test]
async fn id_hint_requests_a_database_id_for_new_tags() {
    let s = fresh();
    let doc = doc_of(
        "format: voltrus-project/1\ntags:\n- {key: a, name: A, id_hint: 41}\n- {key: b, name: B}\n- {key: c, name: C, id_hint: 41}\n",
    );
    apply_import(&s, &doc, Mode::Merge).await.unwrap();
    assert_eq!(
        scalar_i64(&s, "SELECT id FROM data_sources WHERE key='a'").await,
        41
    );
    assert!(
        scalar_i64(&s, "SELECT id FROM data_sources WHERE key='c'").await != 41,
        "a taken hint is ignored"
    );
    // The hint is not part of the exported form.
    assert!(!export_text(&s).await.contains("id_hint"));
}

// ── Secrets and excluded data ──

#[tokio::test]
async fn secrets_and_runtime_data_never_appear_in_output() {
    let a = seeded().await;
    let text = export_text(&a).await;
    for secret in SECRETS {
        assert!(!text.contains(secret), "{secret} leaked into the export");
    }
    for runtime in [
        "alice",
        "password_hash",
        "telemetry",
        "High voltage",
        "vk_",
        "dark",
    ] {
        assert!(
            !text.contains(runtime),
            "{runtime} must not be part of a project"
        );
    }
    assert!(text.contains("${VOLTRUS_OPCUA_DEVICES_INVERTER_OPC_PASSWORD}"));
    assert!(text.contains("${VOLTRUS_HUB_SITES_SITE_B_API_KEY}"));
    assert!(text.contains("https://api.example.com/v1?token=${VOLTRUS_TAGS_PLTS_2_EXT_SOURCE_CONFIG_URL_QUERY_TOKEN}&x=1"));
    assert!(
        text.contains("Accept: json"),
        "non-secret headers stay readable"
    );

    // The same holds for the directory form.
    let mut doc = export_project_yaml(&a).await.unwrap();
    doc.generated_at = "x".into();
    let dir = temp_dir("secrets");
    write_project_dir(&doc, &dir).unwrap();
    for (path, content) in read_tree(&dir) {
        for secret in SECRETS {
            assert!(!content.contains(secret), "{secret} leaked into {path}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn import_needs_the_environment_only_for_new_secrets() {
    let a = seeded().await;
    let doc = doc_of(&export_text(&a).await);

    // Into an empty database the references must resolve.
    let b = fresh();
    let err = apply_import_with_env(&b, &doc, Mode::Merge, no_env())
        .await
        .unwrap_err();
    let text = invalid(&err)
        .0
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("environment variable(s) not set"), "{text}");
    assert!(
        text.contains("VOLTRUS_OPCUA_DEVICES_INVERTER_OPC_PASSWORD"),
        "{text}"
    );
    assert_eq!(count(&b, "opcua_devices").await, 0, "nothing written");

    // Into the database the export came from, the stored values stand in.
    let plan = plan_import_with_env(&a, &doc, Mode::Replace, no_env())
        .await
        .unwrap();
    assert!(!plan.has_changes(), "{}", plan.render());

    // A changed environment value is a change; the plan does not print it.
    let mut env_map = BTreeMap::new();
    env_map.insert(
        "VOLTRUS_HUB_SITES_SITE_B_API_KEY".to_string(),
        "rotated-key".to_string(),
    );
    let env: Env = Arc::new(move |n| env_map.get(n).cloned());
    let plan = apply_import_with_env(&a, &doc, Mode::Merge, env)
        .await
        .unwrap();
    assert_eq!(plan.totals.update, 1);
    assert_eq!(plan.changes[0].fields, vec!["api_key"]);
    assert!(!plan.render().contains("rotated-key"));
    assert_eq!(
        scalar_text(&a, "SELECT api_key FROM hub_sites").await,
        "rotated-key"
    );
}

#[tokio::test]
async fn literal_secrets_in_hand_written_files_are_accepted() {
    let s = fresh();
    let doc = doc_of(
        "format: voltrus-project/1\nhub_sites:\n- {name: B, url: 'https://b', api_key: literal-key}\n",
    );
    apply_import(&s, &doc, Mode::Merge).await.unwrap();
    assert_eq!(
        scalar_text(&s, "SELECT api_key FROM hub_sites").await,
        "literal-key"
    );
    // ... and the next export replaces it by a reference.
    assert!(!export_text(&s).await.contains("literal-key"));
}

#[test]
fn redaction_handles_urls_headers_and_nested_json() {
    use serde_yaml_neo::Value;
    let mut e: serde_yaml_neo::Mapping = serde_yaml_neo::from_str(
        "name: x\nurl: 'https://u:pw@h/p?api_key=AAA&keep=1'\nheaders:\n  Authorization: Bearer t\n  X-Trace: abc\ncfg:\n  list:\n  - {password: p1}\n  - plain\n",
    )
    .unwrap();
    let captured = secrets::redact("cameras", "c1", &mut e);
    let out = serde_yaml_neo::to_string(&Value::Mapping(e.clone())).unwrap();
    for s in ["pw@", "AAA", "Bearer t", "p1"] {
        assert!(!out.contains(s), "{s} survived: {out}");
    }
    assert!(out.contains("X-Trace: abc") && out.contains("keep=1") && out.contains("- plain"));
    assert_eq!(captured.len(), 4);
    let missing = secrets::resolve(
        "cameras",
        &mut e,
        &|n| captured.get(n).cloned(),
        &BTreeMap::new(),
    );
    assert!(missing.is_empty());
    let back = serde_yaml_neo::to_string(&Value::Mapping(e)).unwrap();
    assert!(back.contains("https://u:pw@h/p?api_key=AAA&keep=1") && back.contains("Bearer t"));
}

#[test]
fn script_sources_with_awkward_whitespace_survive_yaml() {
    let mut e = serde_yaml_neo::Mapping::new();
    for source in [
        SCRIPT_SOURCE,
        "  leading",
        "trailing  ",
        "tab\there",
        "a\r\nb\r\n",
        "",
        "\n\n",
        "yes",
        "null",
        "- not a list",
        "key: value # comment",
        "${NAME}",
    ] {
        e.insert("source".into(), source.into());
        let doc = serde_yaml_neo::to_string(&serde_yaml_neo::Value::Mapping(e.clone())).unwrap();
        let back: serde_yaml_neo::Mapping = serde_yaml_neo::from_str(&doc).unwrap();
        assert_eq!(
            back.get("source").and_then(|v| v.as_str()),
            Some(source),
            "{doc}"
        );
    }
}

// ── Device templates ──

const MINIMAL_TEMPLATE: &str = r#"
format: voltrus-project/1
device_templates:
- template_id: bare
  version: 1.0.0
  source: local
  manufacturer: Acme
  model: Bare
  protocol: modbus
  registers:
  - {name: v, offset: 0}
"#;

async fn stored_templates(storage: &Storage) -> Vec<crate::device_templates::DeviceTemplate> {
    let mut all = storage.list_device_templates().await.unwrap();
    all.sort_by(|a, b| {
        (&a.template_id, &a.source, &a.version).cmp(&(&b.template_id, &b.source, &b.version))
    });
    all
}

#[tokio::test]
async fn device_templates_export_as_flat_template_fields() {
    let a = seeded().await;
    let doc = export_project_yaml(&a).await.unwrap();
    let text = to_yaml(&doc);
    assert!(!text.contains("body:"), "the stored body leaked:\n{text}");
    assert!(!text.contains("voltrus-device-template/1"));
    let templates = doc.entities("device_templates");
    assert_eq!(templates.len(), 3);
    let keys: Vec<String> = templates
        .iter()
        .map(|t| super::export::entity_key(spec::section("device_templates").unwrap(), t))
        .collect();
    assert_eq!(
        keys,
        [
            "acme-pm100/1.0.0/local",
            "acme-pm100/1.1.0/local",
            "vav-1/2.0.0/catalog:https://catalog.example.com/index.json",
        ]
    );
    let pm = &templates[0];
    assert_eq!(
        pm.get("endianness").and_then(|v| v.as_str()),
        Some("swapped")
    );
    assert_eq!(
        pm.get("registers")
            .and_then(|v| v.as_sequence())
            .map(|s| s.len()),
        Some(2)
    );
    // Defaults are omitted, so a register with only a name and an offset stays short.
    let second = templates[1]
        .get("registers")
        .and_then(|v| v.as_sequence())
        .unwrap()[0]
        .as_mapping()
        .unwrap();
    assert!(second.get("unit").is_none() && second.get("writable").is_none());
}

#[tokio::test]
async fn device_templates_import_recreates_identical_templates() {
    let a = seeded().await;
    let doc = parse_project_yaml(&export_text(&a).await).unwrap();
    let b = fresh();
    let plan = apply_import_with_env(&b, &doc, Mode::Merge, env_for(&a).await)
        .await
        .unwrap();
    assert_eq!(plan.sections["device_templates"].create, 3);

    let (before, after) = (stored_templates(&a).await, stored_templates(&b).await);
    assert_eq!(before.len(), 3);
    assert_eq!(
        before, after,
        "templates changed on the way through a project"
    );
    // The row columns the template routes search on match the body.
    assert_eq!(
        scalar_text(
            &b,
            "SELECT group_concat(template_id || ':' || manufacturer || ':' || protocol || ':' || verified, ',') FROM (SELECT * FROM device_templates ORDER BY template_id, version)"
        )
        .await,
        "acme-pm100:Acme:modbus:1,acme-pm100:Acme:modbus:0,vav-1:Globex:bacnet:0"
    );
}

#[tokio::test]
async fn hand_written_template_is_stored_normalised_and_then_unchanged() {
    let s = fresh();
    let doc = doc_of(MINIMAL_TEMPLATE);
    let plan = apply_import(&s, &doc, Mode::Merge).await.unwrap();
    assert_eq!(plan.totals.create, 1);

    let stored = stored_templates(&s).await;
    assert_eq!(stored.len(), 1);
    // The routes' defaults were filled in, exactly as for a template saved over HTTP.
    assert_eq!(stored[0].port, Some(502));
    assert_eq!(stored[0].unit_id, Some(1));
    assert_eq!(stored[0].endianness.as_deref(), Some("big"));
    assert_eq!(stored[0].interval_ms, Some(1000));
    assert_eq!(stored[0].license, "CC-BY-4.0");
    assert_eq!(stored[0].format, "voltrus-device-template/1");

    // Applying the same file again has nothing to do, and the export is stable.
    let again = plan_import(&s, &doc, Mode::Replace).await.unwrap();
    assert!(!again.has_changes(), "{}", again.render());
    let exported = export_project_yaml(&s).await.unwrap();
    assert_eq!(
        exported.entities("device_templates"),
        doc.entities("device_templates")
    );
}

#[tokio::test]
async fn device_template_update_and_replace_semantics() {
    let s = fresh();
    apply_import(&s, &doc_of(MINIMAL_TEMPLATE), Mode::Merge)
        .await
        .unwrap();

    let changed = MINIMAL_TEMPLATE.replace("model: Bare", "model: Bare 2");
    let plan = apply_import(&s, &doc_of(&changed), Mode::Merge)
        .await
        .unwrap();
    assert_eq!(plan.totals.update, 1);
    assert_eq!(plan.changes[0].fields, ["model"]);
    assert_eq!(stored_templates(&s).await[0].model, "Bare 2");

    // A project that lists other things leaves templates alone in merge mode...
    apply_import(&s, &doc_of(SMALL), Mode::Merge).await.unwrap();
    assert_eq!(stored_templates(&s).await.len(), 1);
    // ...and deletes them in replace mode, like every other section.
    let plan = apply_import(&s, &doc_of(SMALL), Mode::Replace)
        .await
        .unwrap();
    assert_eq!(plan.sections["device_templates"].delete, 1);
    assert!(stored_templates(&s).await.is_empty());
}

#[tokio::test]
async fn invalid_device_templates_are_reported_with_their_key() {
    let s = fresh();
    let cases = [
        (
            MINIMAL_TEMPLATE.replace("source: local", "source: builtin"),
            "built-in templates",
        ),
        (
            MINIMAL_TEMPLATE.replace("source: local", "source: remote"),
            "must be 'local' or 'catalog:<url>'",
        ),
        (
            MINIMAL_TEMPLATE.replace(
                "{name: v, offset: 0}",
                "{name: v, offset: 0, register_type: float16}",
            ),
            "register_type 'float16'",
        ),
        (
            MINIMAL_TEMPLATE.replace("version: 1.0.0", "version: one"),
            "semantic version",
        ),
        (
            MINIMAL_TEMPLATE.replace("protocol: modbus", "protocol: bacnet"),
            "registers do not apply to a bacnet template",
        ),
        (
            MINIMAL_TEMPLATE.replace("  model: Bare", "  model: Bare\n  body: {x: 1}"),
            "`body` is how the server stores a template",
        ),
    ];
    for (text, expected) in cases {
        let doc = doc_of(&text);
        let err = plan_import(&s, &doc, Mode::Merge).await.unwrap_err();
        let issues: Vec<String> = invalid(&err).0.iter().map(|i| i.to_string()).collect();
        assert!(
            issues
                .iter()
                .any(|m| m.contains(expected) && m.contains("device_templates[bare/")),
            "expected '{expected}' in {issues:?}"
        );
    }
    assert!(stored_templates(&s).await.is_empty());
}

#[tokio::test]
async fn duplicate_device_template_keys_are_rejected() {
    let doc = doc_of(&format!(
        "{MINIMAL_TEMPLATE}- template_id: bare\n  version: 1.0.0\n  source: local\n  manufacturer: Acme\n  model: Other\n  protocol: modbus\n  registers:\n  - {{name: v, offset: 0}}\n"
    ));
    let err = plan_import(&fresh(), &doc, Mode::Merge).await.unwrap_err();
    assert!(
        invalid(&err)
            .0
            .iter()
            .any(|i| i.message.contains("duplicate device template"))
    );
}
