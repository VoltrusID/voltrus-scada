//! `GET /api/v1/devices/{id}/traffic`: a device's recent protocol frames,
//! decoded, paired, explained and mapped to the tags they fed
//! (docs/traffic.md).

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};
use voltrus_dissect::{DecodedFrame, Dir, TagRef};

use super::{AppError, AppState};
use crate::models::ModbusRegister;

/// Most frames one call returns.
pub const MAX_LIMIT: usize = 2_000;

#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[serde(deny_unknown_fields)]
pub struct TrafficQuery {
    /// Only this function code (Modbus: 1-127, without the exception bit).
    pub function: Option<u16>,
    /// Only frames with an error: exceptions, malformed frames, unanswered requests.
    #[serde(default)]
    pub errors: bool,
    /// Only frames touching addresses in `addr_from..=addr_to` (0-based wire addresses).
    pub addr_from: Option<u32>,
    pub addr_to: Option<u32>,
    /// Only this data table: coil, discrete, input or holding.
    pub space: Option<String>,
    /// Only frames newer than this `seq` (for polling).
    pub after: Option<u64>,
    /// Newest frames to return, 1-2000 (default 200).
    pub limit: Option<usize>,
    /// Include the field tree and raw bytes (default true).
    pub detail: Option<bool>,
    /// Start or extend the capture (default true). With false, only reads.
    pub watch: Option<bool>,
}

/// GET /api/v1/devices/{id}/traffic
#[utoipa::path(
    get,
    path = "/api/v1/devices/{id}/traffic",
    tag = "Devices",
    params(("id" = String, Path, description = "Device ID"), TrafficQuery),
    responses(
        (status = 200, description = "`{device_id, capturing, capture_until_ms, frames: [...]}`; frames oldest first", body = serde_json::Value),
        (status = 400, description = "Bad filter"),
        (status = 404, description = "No Modbus device with this id"),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_device_traffic(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TrafficQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(device_traffic(&state, &id, q).await?))
}

pub async fn device_traffic(
    state: &AppState,
    id: &str,
    q: TrafficQuery,
) -> Result<Value, AppError> {
    if state.storage.get_modbus_device(id).await?.is_none() {
        return Err(AppError::not_found(format!(
            "no Modbus device with id {id}: traffic capture covers Modbus TCP devices for now"
        )));
    }
    if let Some(space) = q.space.as_deref()
        && !crate::models::REGISTER_AREAS.contains(&space)
    {
        return Err(AppError::bad_request(
            "space must be coil, discrete, input or holding",
        ));
    }
    if let (Some(a), Some(b)) = (q.addr_from, q.addr_to)
        && a > b
    {
        return Err(AppError::bad_request("addr_from is after addr_to"));
    }
    let limit = q.limit.unwrap_or(200).clamp(1, MAX_LIMIT);
    let detail = q.detail.unwrap_or(true);

    let dev = state.cache.traffic().device(id);
    let was_capturing = dev.recording();
    let capture_until_ms = if q.watch.unwrap_or(true) {
        dev.watch()
    } else {
        dev.snapshot().watch_until_ms
    };
    let snap = dev.snapshot();
    let now_us = crate::timeutil::now_millis() * 1000;
    let mut frames = crate::traffic::decode(id, &snap.frames, now_us);
    let total = frames.len();

    frames.retain(|f| keep(f, &q));
    let start = frames.len().saturating_sub(limit);
    let mut frames = frames.split_off(start);

    let registers = state.storage.list_modbus_registers(id).await?;
    let tags: HashMap<String, (i64, String)> = state
        .storage
        .list_data_sources(None)
        .await?
        .into_iter()
        .filter(|d| d.device_id == id && d.source_type == "modbus")
        .map(|d| (d.key, (d.id, d.name)))
        .collect();
    for f in &mut frames {
        f.tag_refs = tag_refs(f, &registers, &tags);
    }

    let frames: Vec<Value> = frames
        .iter()
        .map(|f| {
            let mut v = serde_json::to_value(f).unwrap_or(Value::Null);
            if !detail && let Some(o) = v.as_object_mut() {
                o.remove("fields");
                o.remove("raw");
            }
            v
        })
        .collect();
    let mut out = json!({
        "device_id": id,
        "protocol": "modbus_tcp",
        "capturing": capture_until_ms > crate::timeutil::now_millis(),
        "capture_until_ms": capture_until_ms,
        "capacity": snap.capacity,
        "kept": total,
        "evicted": snap.evicted,
        "frames": frames,
    });
    if !was_capturing && total == 0 && q.watch.unwrap_or(true) {
        out["note"] = json!(
            "Capture started now: frames appear as the device is polled. Ask again in a few seconds."
        );
    }
    Ok(out)
}

fn keep(f: &DecodedFrame, q: &TrafficQuery) -> bool {
    if q.after.is_some_and(|after| f.seq <= after) {
        return false;
    }
    if q.errors && f.error.is_none() {
        return false;
    }
    if q.function.is_some() && f.function != q.function {
        return false;
    }
    if let Some(space) = q.space.as_deref()
        && f.range.is_none_or(|r| r.space != space)
    {
        return false;
    }
    if q.addr_from.is_some() || q.addr_to.is_some() {
        let from = q.addr_from.unwrap_or(0);
        let to = q.addr_to.unwrap_or(u32::MAX);
        if f.range.is_none_or(|r| !r.overlaps(from, to)) {
            return false;
        }
    }
    true
}

/// Registers a Modbus driver reads per register type.
fn width(reg: &ModbusRegister) -> u32 {
    match reg.register_area.as_str() {
        "coil" | "discrete" => 1,
        _ => match reg.register_type.as_str() {
            "float64" => 4,
            "int32" | "float32" => 2,
            _ => 1,
        },
    }
}

/// The configured registers a frame read or wrote, with their bytes in the
/// frame where the frame carries their values.
fn tag_refs(
    f: &DecodedFrame,
    registers: &[ModbusRegister],
    tags: &HashMap<String, (i64, String)>,
) -> Vec<TagRef> {
    let Some(range) = f.range else {
        return Vec::new();
    };
    if f.error
        .as_ref()
        .is_some_and(|e| e.kind == voltrus_dissect::ErrorKind::Malformed)
    {
        return Vec::new();
    }
    let function = f.function.unwrap_or(0);
    let end = range.start + range.count.max(1);
    let mut out = Vec::new();
    for reg in registers.iter().filter(|r| r.register_area == range.space) {
        let addr = u32::from(crate::models::protocol_address(reg.address, reg.zero_based));
        let w = width(reg);
        if addr + w <= range.start || addr >= end {
            continue;
        }
        let rel = addr.saturating_sub(range.start);
        let bits = matches!(range.space, "coil" | "discrete");
        // Where the values are: read responses after the byte count,
        // multiple writes after their byte count, single writes at 10.
        let at = match (f.dir, function, f.error.is_some()) {
            (Dir::Rx, 1..=4 | 23, false) => Some(9),
            (Dir::Tx, 15 | 16, _) => Some(13),
            (Dir::Tx | Dir::Rx, 5 | 6, false) => Some(10),
            _ => None,
        };
        let (offset, len) = match at {
            Some(base) if bits => (Some(base + rel / 8), Some(1)),
            Some(base) => (Some(base + 2 * rel), Some(2 * w)),
            None => (None, None),
        };
        let (offset, len) = match (offset, len) {
            (Some(o), Some(l)) if (o as usize) < f.raw.len() => {
                (Some(o), Some(l.min(f.raw.len() as u32 - o)))
            }
            _ => (None, None),
        };
        let tag = tags.get(&reg.name);
        out.push(TagRef {
            key: reg.name.clone(),
            tag_id: tag.map(|t| t.0),
            tag_name: tag.map(|t| t.1.clone()),
            offset,
            len,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use voltrus_dissect::modbus::Session;

    fn reg(name: &str, address: u16, ty: &str, area: &str) -> ModbusRegister {
        ModbusRegister {
            id: 0,
            device_id: "d".into(),
            name: name.into(),
            address,
            register_type: ty.into(),
            register_area: area.into(),
            scale: 1.0,
            sort_order: 0,
            zero_based: false,
        }
    }

    fn adu(pdu: &[u8]) -> Vec<u8> {
        let mut v = vec![0, 1, 0, 0];
        v.extend(((pdu.len() + 1) as u16).to_be_bytes());
        v.push(1);
        v.extend(pdu);
        v
    }

    #[test]
    fn maps_a_read_response_to_its_tags_and_bytes() {
        let regs = vec![
            reg("voltage", 40001, "float32", "holding"),
            reg("current", 40003, "uint16", "holding"),
            reg("outside", 40010, "uint16", "holding"),
            reg("other_area", 30001, "uint16", "input"),
        ];
        let tags = HashMap::from([("voltage".to_string(), (7, "Voltage L1".to_string()))]);
        let mut s = Session::new();
        let req = s.decode(&adu(&[3, 0, 0, 0, 3]), Dir::Tx, 0, 1);
        let resp = s.decode(&adu(&[3, 6, 0x43, 0x5c, 0, 0, 0, 5]), Dir::Rx, 10, 2);
        let refs = tag_refs(&resp, &regs, &tags);
        let got: Vec<_> = refs
            .iter()
            .map(|r| (r.key.as_str(), r.offset, r.len))
            .collect();
        assert_eq!(
            got,
            [
                ("voltage", Some(9), Some(4)),
                ("current", Some(13), Some(2))
            ]
        );
        assert_eq!(refs[0].tag_id, Some(7));
        // The request names the same tags, without value bytes.
        let refs = tag_refs(&req, &regs, &tags);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].offset, None);
    }

    #[test]
    fn filters() {
        let mut s = Session::new();
        let read = s.decode(&adu(&[3, 0, 100, 0, 10]), Dir::Tx, 0, 1);
        let q = |f: fn(&mut TrafficQuery)| {
            let mut q = TrafficQuery::default();
            f(&mut q);
            q
        };
        assert!(keep(&read, &q(|q| q.function = Some(3))));
        assert!(!keep(&read, &q(|q| q.function = Some(4))));
        assert!(keep(
            &read,
            &q(|q| (q.addr_from, q.addr_to) = (Some(109), Some(200)))
        ));
        assert!(!keep(&read, &q(|q| q.addr_from = Some(110))));
        assert!(!keep(&read, &q(|q| q.errors = true)));
        assert!(!keep(&read, &q(|q| q.space = Some("input".into()))));
        assert!(!keep(&read, &q(|q| q.after = Some(1))));
    }
}
