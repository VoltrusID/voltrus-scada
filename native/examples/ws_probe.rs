//! Manual WS probe: login via REST, then connect the WS and dump the exact
//! server response to our named-MessagePack Auth frame.
//!
//! Run: cargo run -p voltrus-scada-native --example ws_probe -- http://localhost:3100 <user> <pass>

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use voltrus_scada_native::api::VoltrusApi;
use voltrus_scada_native::ws_types::{ClientMessage, ServerMessage};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .unwrap_or_else(|| "http://localhost:3100".into());
    let user = args.next().unwrap_or_else(|| "admin".into());
    let pass = args.next().expect("password");

    let mut api = VoltrusApi::new(&url);
    let login = api.login(&user, &pass).await.expect("login");
    println!("login ok: {} ({})", login.username, login.role);
    let token = api.token.clone().unwrap();
    println!(
        "token len={} head={:?} tail={:?}",
        token.len(),
        &token[..8.min(token.len())],
        &token[token.len().saturating_sub(6)..]
    );
    println!("token full: {token}");

    // Sanity: REST accepts this token?
    let me = api.data_sources().await;
    println!(
        "data_sources with token: {}",
        me.map(|v| format!("{} sources", v.len()))
            .unwrap_or_else(|e| format!("ERR {e}"))
    );

    let ws_url = url.replacen("http://", "ws://", 1) + "/api/v1/ws";
    let (ws, _resp) = tokio_tungstenite::connect_async(ws_url.as_str())
        .await
        .expect("ws connect");
    println!("ws connected");
    let (mut sink, mut stream) = ws.split();

    let auth = rmp_serde::to_vec_named(&ClientMessage::Auth {
        token: token.clone(),
    })
    .unwrap();
    println!(
        "auth frame: {} bytes, hex head: {:02x?}",
        auth.len(),
        &auth[..12.min(auth.len())]
    );
    sink.send(Message::Binary(auth.into()))
        .await
        .expect("send auth");

    match tokio::time::timeout(std::time::Duration::from_secs(5), stream.next()).await {
        Ok(Some(Ok(Message::Binary(data)))) => {
            match rmp_serde::from_slice::<ServerMessage>(&data) {
                Ok(msg) => println!("server replied: {msg:?}"),
                Err(e) => println!(
                    "decode error: {e}; raw={:02x?}",
                    &data[..40.min(data.len())]
                ),
            }
        }
        other => println!("no/other reply: {other:?}"),
    }

    // Also try the ARRAY encoding to compare.
    if let Ok((ws2, _)) = tokio_tungstenite::connect_async(ws_url.as_str()).await {
        let (mut sink2, mut stream2) = ws2.split();
        let auth_arr = rmp_serde::to_vec(&ClientMessage::Auth { token }).unwrap();
        sink2
            .send(Message::Binary(auth_arr.into()))
            .await
            .expect("send auth arr");
        match tokio::time::timeout(std::time::Duration::from_secs(5), stream2.next()).await {
            Ok(Some(Ok(Message::Binary(data)))) => {
                match rmp_serde::from_slice::<ServerMessage>(&data) {
                    Ok(msg) => println!("array-encoding reply: {msg:?}"),
                    Err(e) => println!("array decode error: {e}"),
                }
            }
            other => println!("array: no reply {other:?}"),
        }
    }
}
