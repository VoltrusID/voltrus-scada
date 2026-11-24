//! Master against the simulator outstation over localhost TCP.

use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

use crate::dnp3::application::{AnalogCommand, Control, Crob, Fragment, FunctionCode};
use crate::dnp3::channel::Channel;
use crate::dnp3::master::{Dnp3MasterSession, OperateMode, TimeSyncMode};
use crate::dnp3::objects::Dnp3PointValue;
use crate::dnp3::outstation::{OutstationConfig, OutstationHandle, serve};

fn quiet() -> OutstationConfig {
    OutstationConfig {
        event_period: None,
        confirm_timeout: Duration::from_secs(2),
        ..OutstationConfig::default()
    }
}

async fn start(cfg: OutstationConfig) -> (Dnp3MasterSession, OutstationHandle) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (master, outstation) = (cfg.master_address, cfg.address);
    let handle = serve(listener, cfg).await;
    let mut m = Dnp3MasterSession::connect("127.0.0.1", port, master, outstation)
        .await
        .unwrap();
    m.set_response_timeout(Duration::from_secs(3));
    (m, handle)
}

/// Confirms are sent after the master has its data, so the outstation may
/// see them a moment after the call returns.
async fn wait_events_released(h: &OutstationHandle) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while h.read(|db| db.pending_events()) > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(h.read(|db| db.pending_events()), 0);
}

fn find(points: &[Dnp3PointValue], group: u8, index: u16) -> Option<&Dnp3PointValue> {
    points
        .iter()
        .rev()
        .find(|p| p.group == group && p.index == index)
}

#[tokio::test]
async fn integrity_poll_at_startup_clears_restart_and_sets_time() {
    let (mut m, h) = start(quiet()).await;
    assert!(h.read(|db| db.restart_bit() && db.need_time_bit()));
    let points = m.startup(true).await.unwrap();

    assert_eq!(find(&points, 30, 2).unwrap().value, 200.0);
    assert_eq!(find(&points, 30, 2).unwrap().variation, 1);
    assert_eq!(find(&points, 1, 3).unwrap().value, 0.0);
    assert_eq!(find(&points, 10, 0).unwrap().variation, 2);
    assert_eq!(find(&points, 40, 1).unwrap().variation, 2);
    assert!(points.iter().all(|p| p.is_good()));

    assert!(!m.iin.device_restart());
    assert!(!m.iin.need_time());
    assert!(h.read(|db| !db.restart_bit() && !db.need_time_bit()));
    assert!(h.read(|db| db.unsolicited_enabled()));
}

#[tokio::test]
async fn lan_time_sync() {
    let (mut m, h) = start(quiet()).await;
    m.time_sync = TimeSyncMode::Lan;
    m.startup(false).await.unwrap();
    assert!(h.read(|db| !db.need_time_bit()));
}

#[tokio::test]
async fn event_poll_returns_and_confirms_events() {
    let (mut m, h) = start(quiet()).await;
    m.startup(false).await.unwrap();
    h.update(|db| {
        db.set_analog_input(1, 555);
        db.set_binary_input(2, true);
    });
    let events = m.event_poll().await.unwrap();
    let ai = find(&events, 32, 1).unwrap();
    assert_eq!((ai.variation, ai.value), (1, 555.0));
    let bi = find(&events, 2, 2).unwrap();
    assert_eq!((bi.variation, bi.value), (2, 1.0));
    assert!(bi.time_ms.unwrap() > 1_600_000_000_000);
    // Confirmed, so the outstation dropped them.
    wait_events_released(&h).await;
    assert!(m.event_poll().await.unwrap().is_empty());
}

#[tokio::test]
async fn unsolicited_events_are_received_and_confirmed() {
    let (mut m, h) = start(quiet()).await;
    m.startup(true).await.unwrap();
    h.update(|db| db.set_analog_input(3, -42));
    let (_tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut got = Vec::new();
    while got.is_empty() && Instant::now() < deadline {
        m.idle_until(Instant::now() + Duration::from_millis(200), &mut rx)
            .await
            .unwrap();
        got.extend(m.take_unsolicited());
    }
    assert_eq!(find(&got, 32, 3).unwrap().value, -42.0);
    wait_events_released(&h).await;
}

#[tokio::test]
async fn crob_select_before_operate_and_direct_operate() {
    let (mut m, h) = start(quiet()).await;
    m.startup(false).await.unwrap();

    let on = Control::Crob {
        index: 1,
        crob: Crob::latch(true),
    };
    let r = m
        .operate(on, OperateMode::SelectBeforeOperate)
        .await
        .unwrap();
    assert!(r.success, "{r:?}");
    assert!(h.read(|db| db.binary_outputs[1]));
    assert_eq!(h.read(|db| db.operations), 1);

    let off = Control::Crob {
        index: 1,
        crob: Crob::latch(false),
    };
    let r = m.operate(off, OperateMode::DirectOperate).await.unwrap();
    assert!(r.success, "{r:?}");
    assert!(h.read(|db| !db.binary_outputs[1]));

    // The new state shows in the next static read.
    let points = m.integrity_poll().await.unwrap();
    assert_eq!(find(&points, 10, 1).unwrap().value, 0.0);

    let bad = Control::Crob {
        index: 99,
        crob: Crob::latch(true),
    };
    let r = m
        .operate(bad, OperateMode::SelectBeforeOperate)
        .await
        .unwrap();
    assert!(!r.success);
    assert!(r.error.unwrap().contains("not supported"));
    assert_eq!(h.read(|db| db.operations), 2);
}

#[tokio::test]
async fn analog_output_commands() {
    let (mut m, h) = start(quiet()).await;
    m.startup(false).await.unwrap();
    for (value, mode) in [
        (AnalogCommand::I32(1234), OperateMode::SelectBeforeOperate),
        (AnalogCommand::I16(-7), OperateMode::DirectOperate),
        (AnalogCommand::F32(99.0), OperateMode::DirectOperate),
    ] {
        let c = Control::Analog { index: 0, value };
        let r = m.operate(c, mode).await.unwrap();
        assert!(r.success, "{r:?}");
    }
    assert_eq!(h.read(|db| db.analog_outputs[0]), 99);
    let points = m.integrity_poll().await.unwrap();
    assert_eq!(find(&points, 40, 0).unwrap().value, 99.0);

    let c = Control::Analog {
        index: 0,
        value: AnalogCommand::I32(100_000),
    };
    let r = m.operate(c, OperateMode::DirectOperate).await.unwrap();
    assert!(r.error.unwrap().contains("out of range"));
}

/// Speak to the outstation directly to check its SBO rules.
#[tokio::test]
async fn operate_without_matching_select_is_refused() {
    let cfg = quiet();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _h = serve(listener, cfg.clone()).await;
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut ch = Channel::new(stream, cfg.master_address, cfg.address, true);

    // The null unsolicited response the outstation sends on connect.
    let deadline = || Instant::now() + Duration::from_secs(2);
    let null = ch.recv(deadline()).await.unwrap().unwrap();
    assert_eq!(null.function, FunctionCode::UnsolicitedResponse);
    assert!(null.iin.unwrap().device_restart());
    ch.send(&Fragment::confirm(null.control.seq, true))
        .await
        .unwrap();

    let crob = Control::Crob {
        index: 0,
        crob: Crob::latch(true),
    };
    let echo_status = |f: &Fragment| *f.objects.last().unwrap();

    ch.send(&crob.fragment(0, FunctionCode::Operate))
        .await
        .unwrap();
    let r = ch.recv(deadline()).await.unwrap().unwrap();
    assert_eq!(echo_status(&r), 2, "operate with no select");

    ch.send(&crob.fragment(1, FunctionCode::Select))
        .await
        .unwrap();
    assert_eq!(echo_status(&ch.recv(deadline()).await.unwrap().unwrap()), 0);
    // Wrong sequence: must be select + 1.
    ch.send(&crob.fragment(5, FunctionCode::Operate))
        .await
        .unwrap();
    assert_eq!(echo_status(&ch.recv(deadline()).await.unwrap().unwrap()), 2);

    ch.send(&crob.fragment(6, FunctionCode::Select))
        .await
        .unwrap();
    assert_eq!(echo_status(&ch.recv(deadline()).await.unwrap().unwrap()), 0);
    ch.send(&crob.fragment(7, FunctionCode::Operate))
        .await
        .unwrap();
    assert_eq!(echo_status(&ch.recv(deadline()).await.unwrap().unwrap()), 0);

    // Unknown function code: IIN2.0.
    ch.send(&Fragment::request(8, FunctionCode::ColdRestart, vec![]))
        .await
        .unwrap();
    let r = ch.recv(deadline()).await.unwrap().unwrap();
    assert_eq!(
        r.iin.unwrap().request_error(),
        Some("function code not supported")
    );
}

#[tokio::test]
async fn multi_fragment_multi_frame_integrity_poll() {
    let cfg = OutstationConfig {
        analog_inputs: 300,
        binary_inputs: 100,
        // Over one link frame (249) per fragment, several fragments in all.
        max_fragment: 600,
        ..quiet()
    };
    let (mut m, _h) = start(cfg).await;
    let points = m.startup(false).await.unwrap();
    for i in [0u16, 31, 32, 150, 299] {
        assert_eq!(find(&points, 30, i).unwrap().value, i as f64 * 100.0);
    }
    assert_eq!(points.iter().filter(|p| p.group == 1).count() % 100, 0);
    assert!(find(&points, 1, 99).is_some());
}

#[tokio::test]
async fn simulator_generates_events_periodically() {
    let cfg = OutstationConfig {
        event_period: Some(Duration::from_millis(50)),
        ..quiet()
    };
    let (mut m, _h) = start(cfg).await;
    m.startup(false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let events = m.event_poll().await.unwrap();
    assert!(find(&events, 32, 0).is_some());
    assert!(find(&events, 2, 0).is_some());
}

#[test]
fn points_map_to_configured_tags() {
    use crate::models::{Dnp3DeviceConfig, Dnp3PointConfig};
    let point = |g, i, name: &str| Dnp3PointConfig {
        group_num: g,
        variation: 1,
        point_index: i,
        name: name.into(),
        scale: 2.0,
    };
    let cfg = Dnp3DeviceConfig {
        id: "dnp3_1".into(),
        ip: "127.0.0.1".into(),
        port: 20000,
        master_address: 1,
        outstation_address: 10,
        event_scan_interval_ms: None,
        enable_unsolicited: false,
        integrity_poll_interval_secs: 0,
        points: vec![
            point(30, 0, "ai0"),
            point(12, 1, "relay"),
            point(1, 1, "di"),
        ],
    };
    let pv = |group, index, value| Dnp3PointValue {
        group,
        variation: 1,
        index,
        value,
        flags: 1,
        time_ms: None,
    };
    let m = crate::dnp3::points_to_metrics(
        &cfg,
        &[
            pv(30, 0, 1.0),
            pv(32, 0, 5.0),
            pv(10, 1, 1.0),
            pv(1, 2, 1.0),
        ],
    );
    assert_eq!(m.get("ai0"), Some(&10.0));
    assert_eq!(m.get("relay"), Some(&2.0));
    assert_eq!(m.get("di"), None);
}
