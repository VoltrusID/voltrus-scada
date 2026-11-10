//! Queued writes through the executor against an in-process Modbus server,
//! read back from the server's own memory.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::net::{TcpListener, TcpStream};
use tokio_modbus::prelude::*;
use tokio_modbus::server::Service;
use tokio_modbus::server::tcp::{Server, accept_tcp_connection};

use super::CommandQueue;
use crate::models::Command;

#[derive(Default)]
struct Memory {
    holding: Vec<u16>,
    coils: Vec<bool>,
}

#[derive(Clone)]
struct MemoryService(Arc<Mutex<Memory>>);

impl Service for MemoryService {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = std::future::Ready<Result<Response, ExceptionCode>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let mut m = self.0.lock().unwrap();
        let out = match req {
            Request::WriteSingleRegister(a, w) => {
                m.holding[a as usize] = w;
                Ok(Response::WriteSingleRegister(a, w))
            }
            Request::WriteMultipleRegisters(a, words) => {
                let a0 = a as usize;
                m.holding[a0..a0 + words.len()].copy_from_slice(&words);
                Ok(Response::WriteMultipleRegisters(a, words.len() as u16))
            }
            Request::WriteSingleCoil(a, on) => {
                m.coils[a as usize] = on;
                Ok(Response::WriteSingleCoil(a, on))
            }
            _ => Err(ExceptionCode::IllegalFunction),
        };
        std::future::ready(out)
    }
}

async fn start_server(memory: Arc<Mutex<Memory>>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let server = Server::new(listener);
        let new_service = move |_: SocketAddr| -> std::io::Result<Option<MemoryService>> {
            Ok(Some(MemoryService(memory.clone())))
        };
        let on_connected = |stream: TcpStream, socket_addr: SocketAddr| {
            let new_service = &new_service;
            async move { accept_tcp_connection(stream, socket_addr, new_service) }
        };
        let _ = server.serve(&on_connected, |_| {}).await;
    });
    addr
}

fn command(data_source_id: i64, value: &str) -> Command {
    Command {
        id: format!("test-{data_source_id}-{value}"),
        data_source_id,
        device_id: 0,
        protocol: "modbus".into(),
        value: value.into(),
        status: "pending".into(),
        policy: "immediate".into(),
        scheduled_at: None,
        retry_count: 0,
        max_retries: 3,
        error_message: None,
        created_at: 0,
        sent_at: None,
        acked_at: None,
        resolved_at: None,
        created_by: "test".into(),
        device_ref: "plc".into(),
        readback_status: None,
        readback_value: None,
        readback_attempts: 0,
        readback_ts: None,
    }
}

async fn tag(storage: &crate::storage::Storage, key: &str, source_config: &str) -> i64 {
    storage
        .create_data_source(
            key,
            key,
            "",
            "#000",
            0.0,
            100.0,
            0.0,
            0.0,
            0.0,
            0,
            1,
            "modbus",
            source_config,
            "plc",
            None,
            "sparkline",
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn queued_writes_use_the_registers_type_scale_and_area() {
    let memory = Arc::new(Mutex::new(Memory {
        holding: vec![0; 32],
        coils: vec![false; 16],
    }));
    let addr = start_server(memory.clone()).await;

    let storage = crate::storage::Storage::new_in_memory().unwrap();
    let registers: Vec<crate::models::ModbusRegisterInput> =
        serde_json::from_value(serde_json::json!([
            {"name": "setpoint", "offset": 4, "register_type": "float32", "scale": 0.5},
            {"name": "pump", "offset": 3, "register_type": "uint16", "register_area": "coil"},
            {"name": "level", "offset": 8, "register_type": "int16", "scale": 0.1},
        ]))
        .unwrap();
    storage
        .create_modbus_device(
            "plc",
            "PLC",
            &addr.ip().to_string(),
            addr.port(),
            1,
            1000,
            "little",
            &registers,
        )
        .await
        .unwrap();

    let setpoint = tag(&storage, "setpoint", "{}").await;
    let pump = tag(&storage, "plc.pump", "{}").await;
    let level = tag(
        &storage,
        "lvl",
        r#"{"address": 8, "register_area": "holding"}"#,
    )
    .await;
    let unknown = tag(&storage, "nothing", "{}").await;

    let queue = CommandQueue::new(
        storage,
        crate::cache::DeviceCache::new(),
        Arc::new(crate::modbus::ModbusPool::new()),
    );

    let r = queue.execute_write(&command(setpoint, "441")).await;
    assert!(r.success, "{r:?}");
    let r = queue.execute_write(&command(pump, "1")).await;
    assert!(r.success, "{r:?}");
    let r = queue.execute_write(&command(level, "-12.3")).await;
    assert!(r.success, "{r:?}");
    let r = queue.execute_write(&command(unknown, "1")).await;
    assert!(!r.success);
    assert!(r.error.unwrap().contains("matches no register"));

    let m = memory.lock().unwrap();
    // 441 / 0.5 = 882.0 as float32 0x445C8000, low word first ("little").
    assert_eq!(&m.holding[4..6], &[0x8000, 0x445C]);
    let bits = (u32::from(m.holding[5]) << 16) | u32::from(m.holding[4]);
    assert_eq!(f32::from_bits(bits) * 0.5, 441.0);
    assert!(m.coils[3]);
    assert_eq!(
        m.holding[3], 0,
        "the coil write left holding register 3 alone"
    );
    assert_eq!(m.holding[8] as i16, -123);
}
