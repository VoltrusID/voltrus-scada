//! The y-sync v1 messages (`y-protocols/sync`) and awareness updates
//! (`y-protocols/awareness`) as they travel inside the WebSocket's
//! `DesignSync` and `DesignAwareness` frames. A `DesignSync` payload is one
//! sync message without y-websocket's outer message type: `varuint(0|1|2)`
//! then the state vector or update as a length-prefixed buffer.

use yrs::encoding::read;
use yrs::sync::SyncMessage;
use yrs::sync::awareness::{AwarenessUpdate, AwarenessUpdateEntry};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, Origin, ReadTxn, StateVector, Transact, Update};

pub use yrs::sync::SyncMessage as Sync;

/// A sync step 1 of `doc`: its state vector.
pub fn step1(doc: &Doc) -> Vec<u8> {
    SyncMessage::SyncStep1(doc.transact().state_vector()).encode_v1()
}

/// An update message carrying `update`.
pub fn update(update: Vec<u8>) -> Vec<u8> {
    SyncMessage::Update(update).encode_v1()
}

/// Decode one sync message.
pub fn decode(data: &[u8]) -> Result<SyncMessage, read::Error> {
    SyncMessage::decode_v1(data)
}

/// What a sync message did to the document.
#[derive(Debug)]
pub enum Handled {
    /// A step 1: send this step 2 back.
    Reply(Vec<u8>),
    /// A step 2 or an update was applied.
    Applied,
}

/// Why a sync message was not applied.
#[derive(Debug)]
pub enum SyncError {
    Decode(read::Error),
    /// Refused before decoding ([`crate::guard`]).
    Refused(crate::guard::Refused),
    Apply(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Decode(e) => write!(f, "malformed sync message: {e}"),
            SyncError::Refused(e) => write!(f, "update refused: {e}"),
            SyncError::Apply(e) => write!(f, "update not applied: {e}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// Answer or apply one sync message. Updates are applied in a transaction
/// with `origin`, so observers can tell who made them. An update is checked
/// ([`crate::guard::check_update`]) before it is decoded: one that nests
/// values too deeply or holds what a screen document never does is refused
/// whole and nothing is applied.
pub fn handle(doc: &Doc, origin: &str, msg: SyncMessage) -> Result<Handled, SyncError> {
    match msg {
        SyncMessage::SyncStep1(sv) => {
            let diff = doc.transact().encode_diff_v1(&sv);
            Ok(Handled::Reply(SyncMessage::SyncStep2(diff).encode_v1()))
        }
        SyncMessage::SyncStep2(u) | SyncMessage::Update(u) => {
            crate::guard::check_update(&u).map_err(SyncError::Refused)?;
            let update = Update::decode_v1(&u).map_err(SyncError::Decode)?;
            let mut txn = doc.transact_mut_with(Origin::from(origin));
            txn.apply_update(update)
                .map_err(|e| SyncError::Apply(e.to_string()))?;
            Ok(Handled::Applied)
        }
    }
}

/// The whole document as one update (what a newcomer needs).
pub fn state(doc: &Doc) -> Vec<u8> {
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
}

// ── Awareness ──

/// One client's awareness entry: its Yjs client id, clock and JSON state
/// (`"null"` when it left).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presence {
    pub client: u64,
    pub clock: u32,
    pub json: String,
}

/// Decode an awareness update into its entries.
pub fn decode_awareness(data: &[u8]) -> Result<Vec<Presence>, read::Error> {
    let u = AwarenessUpdate::decode_v1(data)?;
    let mut out: Vec<Presence> = u
        .clients
        .into_iter()
        .map(|(client, e)| Presence {
            client: client.get(),
            clock: e.clock,
            json: e.json.to_string(),
        })
        .collect();
    out.sort_by_key(|p| p.client);
    Ok(out)
}

/// Encode awareness entries as one update.
pub fn encode_awareness(entries: &[Presence]) -> Vec<u8> {
    let clients = entries
        .iter()
        .map(|p| {
            (
                yrs::ClientID::new(p.client),
                AwarenessUpdateEntry {
                    clock: p.clock,
                    json: p.json.as_str().into(),
                },
            )
        })
        .collect();
    AwarenessUpdate { clients }.encode_v1()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Run the y-sync handshake both ways between two documents.
    pub(crate) fn exchange(a: &Doc, b: &Doc) {
        for (from, to) in [(a, b), (b, a)] {
            let s1 = decode(&step1(from)).unwrap();
            if let Handled::Reply(r) = handle(to, "peer", s1).unwrap() {
                let s2 = decode(&r).unwrap();
                handle(from, "peer", s2).unwrap();
            }
        }
    }

    #[test]
    fn handshake_converges() {
        let a = Doc::new();
        let b = Doc::new();
        {
            let mut t = a.transact_mut();
            crate::document::load(
                &mut t,
                &crate::document::Rows {
                    widgets: vec![crate::document::tests::wrow("w", 1.0, "{}")],
                    connections: vec![],
                },
            );
        }
        exchange(&a, &b);
        assert_eq!(
            crate::document::read(&a.transact()),
            crate::document::read(&b.transact())
        );
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode(&[9, 9, 9]).is_err());
        assert!(decode(&[2, 3, 1, 2, 3]).is_ok_and(|m| handle(&Doc::new(), "x", m).is_err()));
        assert!(decode_awareness(&[200, 200, 200, 200, 200, 200]).is_err());
    }

    /// The review's stack bomb: one frame used to abort the server.
    #[test]
    fn a_stack_bomb_is_refused_not_a_crash() {
        let bomb = crate::guard::tests::stack_bomb();
        let msg = update(bomb);
        assert!(msg.len() < 256 * 1024);
        let doc = Doc::new();
        let before = state(&doc);
        // On a small stack, as a tokio worker's, so a recursive decode
        // would overflow.
        let r = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                matches!(
                    handle(&doc, "x", decode(&msg).unwrap()),
                    Err(SyncError::Refused(_))
                ) && state(&doc) == before
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(r);
    }

    #[test]
    fn awareness_round_trip() {
        let e = vec![Presence {
            client: 7,
            clock: 3,
            json: r#"{"user":{"name":"ana"}}"#.into(),
        }];
        assert_eq!(decode_awareness(&encode_awareness(&e)).unwrap(), e);
    }
}
