//! Decoder for the service data of a ReadPropertyMultiple-ACK
//! (ASHRAE 135, clause 15.7).
//!
//! The ACK is a sequence of ReadAccessResult:
//!
//! ```text
//! [0] objectIdentifier            context tag 0, 4 bytes
//! [1] listOfResults               opening tag 1
//!     [2] propertyIdentifier      context tag 2
//!     [3] propertyArrayIndex      context tag 3 (optional)
//!     [4] propertyValue           opening tag 4, application-tagged data, closing tag 4
//!   | [5] propertyAccessError     opening tag 5, error class, error code, closing tag 5
//!                                 closing tag 1
//! ```

use anyhow::{Context, Result, bail};

/// One decoded property of an object in the ACK.
#[derive(Debug, Clone, PartialEq)]
pub struct RpmProperty {
    pub property_id: u32,
    pub array_index: Option<u32>,
    pub value: RpmValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RpmValue {
    /// A single numeric, enumerated or boolean value.
    Number(f64),
    /// A value that has no numeric reading (string, bit string, list, ...).
    Unsupported,
    /// The device returned a property access error for this property.
    Error { class: u32, code: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpmObject {
    pub object_type: u32,
    pub instance: u32,
    pub properties: Vec<RpmProperty>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Opening,
    Closing,
    /// Primitive data of the given length. For an application BOOLEAN the
    /// length is 0 and the value is in `lvt`.
    Primitive(usize),
}

#[derive(Debug, Clone, Copy)]
struct Tag {
    number: u8,
    context: bool,
    lvt: u8,
    kind: TagKind,
    header_len: usize,
}

fn byte(data: &[u8], pos: usize) -> Result<u8> {
    data.get(pos)
        .copied()
        .context("ReadPropertyMultiple-ACK truncated")
}

fn read_tag(data: &[u8], pos: usize) -> Result<Tag> {
    let first = byte(data, pos)?;
    let mut header_len = 1;
    let mut number = first >> 4;
    if number == 0x0F {
        number = byte(data, pos + 1)?;
        header_len += 1;
    }
    let context = first & 0x08 != 0;
    let lvt = first & 0x07;
    let kind = if context && lvt == 6 {
        TagKind::Opening
    } else if context && lvt == 7 {
        TagKind::Closing
    } else if !context && number == 1 {
        // Application BOOLEAN keeps its value in the length field.
        TagKind::Primitive(0)
    } else if lvt == 5 {
        let ext = byte(data, pos + header_len)?;
        header_len += 1;
        let len = match ext {
            254 => {
                let v = u16::from_be_bytes([
                    byte(data, pos + header_len)?,
                    byte(data, pos + header_len + 1)?,
                ]) as usize;
                header_len += 2;
                v
            }
            255 => {
                let v = u32::from_be_bytes([
                    byte(data, pos + header_len)?,
                    byte(data, pos + header_len + 1)?,
                    byte(data, pos + header_len + 2)?,
                    byte(data, pos + header_len + 3)?,
                ]) as usize;
                header_len += 4;
                v
            }
            n => n as usize,
        };
        TagKind::Primitive(len)
    } else {
        TagKind::Primitive(lvt as usize)
    };
    Ok(Tag {
        number,
        context,
        lvt,
        kind,
        header_len,
    })
}

fn content<'a>(data: &'a [u8], pos: usize, tag: &Tag) -> Result<&'a [u8]> {
    let TagKind::Primitive(len) = tag.kind else {
        bail!("expected primitive data, found a constructed tag");
    };
    let start = pos + tag.header_len;
    data.get(start..start + len)
        .context("ReadPropertyMultiple-ACK truncated")
}

fn be_unsigned(bytes: &[u8]) -> Result<u64> {
    if bytes.is_empty() || bytes.len() > 8 {
        bail!("invalid unsigned length {}", bytes.len());
    }
    Ok(bytes.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64))
}

fn be_signed(bytes: &[u8]) -> Result<i64> {
    if bytes.is_empty() || bytes.len() > 8 {
        bail!("invalid signed length {}", bytes.len());
    }
    let unsigned = be_unsigned(bytes)?;
    let shift = 64 - 8 * bytes.len() as u32;
    Ok(((unsigned << shift) as i64) >> shift)
}

/// Read a context-tagged primitive with the given tag number; returns its
/// content and the position after it.
fn expect_context(data: &[u8], pos: usize, number: u8) -> Result<(&[u8], usize)> {
    let tag = read_tag(data, pos)?;
    if !tag.context || tag.number != number || !matches!(tag.kind, TagKind::Primitive(_)) {
        bail!("expected context tag {number} at byte {pos}");
    }
    let bytes = content(data, pos, &tag)?;
    Ok((bytes, pos + tag.header_len + bytes.len()))
}

fn expect_bracket(data: &[u8], pos: usize, number: u8, kind: TagKind) -> Result<usize> {
    let tag = read_tag(data, pos)?;
    if !tag.context || tag.number != number || tag.kind != kind {
        bail!("expected {kind:?} tag {number} at byte {pos}");
    }
    Ok(pos + tag.header_len)
}

fn is_bracket(data: &[u8], pos: usize, number: u8, kind: TagKind) -> bool {
    matches!(read_tag(data, pos), Ok(t) if t.context && t.number == number && t.kind == kind)
}

/// Decode one application-tagged value as a number when it is one.
fn application_number(data: &[u8], pos: usize, tag: &Tag) -> Result<Option<f64>> {
    if tag.context {
        return Ok(None);
    }
    let value = match tag.number {
        1 => Some(if tag.lvt != 0 { 1.0 } else { 0.0 }),
        2 | 9 => Some(be_unsigned(content(data, pos, tag)?)? as f64),
        3 => Some(be_signed(content(data, pos, tag)?)? as f64),
        4 => {
            let b = content(data, pos, tag)?;
            let b: [u8; 4] = b.try_into().context("REAL must be 4 bytes")?;
            Some(f32::from_be_bytes(b) as f64)
        }
        5 => {
            let b = content(data, pos, tag)?;
            let b: [u8; 8] = b.try_into().context("DOUBLE must be 8 bytes")?;
            Some(f64::from_be_bytes(b))
        }
        _ => None,
    };
    Ok(value)
}

/// Decode the property value between opening and closing tag 4. `pos` points
/// just after the opening tag; returns the value and the position of the
/// closing tag 4.
fn decode_value(data: &[u8], mut pos: usize) -> Result<(RpmValue, usize)> {
    let mut depth = 0usize;
    let mut items = 0usize;
    let mut first: Option<f64> = None;
    loop {
        let tag = read_tag(data, pos)?;
        match tag.kind {
            TagKind::Closing if depth == 0 => {
                if tag.number != 4 {
                    bail!("unbalanced closing tag {} in property value", tag.number);
                }
                break;
            }
            TagKind::Closing => {
                depth -= 1;
                pos += tag.header_len;
            }
            TagKind::Opening => {
                depth += 1;
                items += 1;
                pos += tag.header_len;
            }
            TagKind::Primitive(len) => {
                if depth == 0 {
                    items += 1;
                    if items == 1 {
                        first = application_number(data, pos, &tag)?;
                    }
                }
                if pos + tag.header_len + len > data.len() {
                    bail!("ReadPropertyMultiple-ACK truncated");
                }
                pos += tag.header_len + len;
            }
        }
    }
    let value = match (items, first) {
        (1, Some(v)) => RpmValue::Number(v),
        _ => RpmValue::Unsupported,
    };
    Ok((value, pos))
}

/// Decode the error class and code between opening and closing tag 5.
fn decode_error(data: &[u8], pos: usize) -> Result<(RpmValue, usize)> {
    let class_tag = read_tag(data, pos)?;
    if class_tag.context || class_tag.number != 9 {
        bail!("expected enumerated error class at byte {pos}");
    }
    let class = be_unsigned(content(data, pos, &class_tag)?)? as u32;
    let pos = pos + class_tag.header_len + content(data, pos, &class_tag)?.len();
    let code_tag = read_tag(data, pos)?;
    if code_tag.context || code_tag.number != 9 {
        bail!("expected enumerated error code at byte {pos}");
    }
    let code = be_unsigned(content(data, pos, &code_tag)?)? as u32;
    let pos = pos + code_tag.header_len + content(data, pos, &code_tag)?.len();
    Ok((RpmValue::Error { class, code }, pos))
}

/// Decode the service data of a ReadPropertyMultiple-ACK.
pub fn decode_rpm_ack(data: &[u8]) -> Result<Vec<RpmObject>> {
    let mut objects = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let (oid, next) = expect_context(data, pos, 0)?;
        let oid: [u8; 4] = oid
            .try_into()
            .context("object identifier must be 4 bytes")?;
        let oid = u32::from_be_bytes(oid);
        pos = expect_bracket(data, next, 1, TagKind::Opening)?;

        let mut properties = Vec::new();
        while !is_bracket(data, pos, 1, TagKind::Closing) {
            let (pid, next) = expect_context(data, pos, 2)?;
            let property_id = be_unsigned(pid)? as u32;
            pos = next;

            let mut array_index = None;
            let tag = read_tag(data, pos)?;
            if tag.context && tag.number == 3 && matches!(tag.kind, TagKind::Primitive(_)) {
                let (idx, next) = expect_context(data, pos, 3)?;
                array_index = Some(be_unsigned(idx)? as u32);
                pos = next;
            }

            let value = if is_bracket(data, pos, 4, TagKind::Opening) {
                let (value, closing) = decode_value(data, pos + read_tag(data, pos)?.header_len)?;
                pos = expect_bracket(data, closing, 4, TagKind::Closing)?;
                value
            } else if is_bracket(data, pos, 5, TagKind::Opening) {
                let (value, next) = decode_error(data, pos + read_tag(data, pos)?.header_len)?;
                pos = expect_bracket(data, next, 5, TagKind::Closing)?;
                value
            } else {
                bail!("expected property value or error at byte {pos}");
            };
            properties.push(RpmProperty {
                property_id,
                array_index,
                value,
            });
        }
        pos = expect_bracket(data, pos, 1, TagKind::Closing)?;

        objects.push(RpmObject {
            object_type: oid >> 22,
            instance: oid & 0x3F_FFFF,
            properties,
        });
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Byte sequences below follow the encoding examples of ASHRAE 135
    // Annex F (F.3.7, ReadPropertyMultiple-ACK) and clause 20.2.

    #[test]
    fn decodes_real_and_present_value_id_85() {
        // AI 5: Present_Value (85 = 0x55) = 72.3 REAL, Reliability (103) = 0 ENUMERATED
        let data = [
            0x0C, 0x00, 0x00, 0x00, 0x05, // [0] analog-input,5
            0x1E, // [1] {
            0x29, 0x55, // [2] present-value
            0x4E, 0x44, 0x42, 0x90, 0x99, 0x9A, 0x4F, // [4] { REAL 72.3 }
            0x29, 0x67, // [2] reliability
            0x4E, 0x91, 0x00, 0x4F, // [4] { ENUMERATED 0 }
            0x1F, // [1] }
        ];
        let objs = decode_rpm_ack(&data).unwrap();
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].object_type, 0);
        assert_eq!(objs[0].instance, 5);
        assert_eq!(objs[0].properties.len(), 2);
        assert_eq!(objs[0].properties[0].property_id, 85);
        let RpmValue::Number(v) = objs[0].properties[0].value else {
            panic!("expected a number");
        };
        assert!((v - 72.3).abs() < 1e-4, "got {v}");
        assert_eq!(objs[0].properties[1].property_id, 103);
        assert_eq!(objs[0].properties[1].value, RpmValue::Number(0.0));
    }

    #[test]
    fn decodes_multiple_objects_and_types() {
        let data = [
            // binary-input,1: present-value BOOLEAN-like ENUMERATED active(1)
            0x0C, 0x00, 0xC0, 0x00, 0x01, 0x1E, 0x29, 0x55, 0x4E, 0x91, 0x01, 0x4F, 0x1F,
            // multi-state-value,2: present-value UNSIGNED 300 (2 bytes)
            0x0C, 0x04, 0xC0, 0x00, 0x02, 0x1E, 0x29, 0x55, 0x4E, 0x22, 0x01, 0x2C, 0x4F, 0x1F,
            // binary-value,3: present-value application BOOLEAN true
            0x0C, 0x01, 0x40, 0x00, 0x03, 0x1E, 0x29, 0x55, 0x4E, 0x11, 0x4F, 0x1F,
            // analog-value,4: present-value DOUBLE -2.5, SIGNED -3
            0x0C, 0x00, 0x80, 0x00, 0x04, 0x1E, 0x29, 0x55, 0x4E, 0x55, 0x08, 0xC0, 0x04, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x4F, 0x29, 0x55, 0x4E, 0x31, 0xFD, 0x4F, 0x1F,
        ];
        let objs = decode_rpm_ack(&data).unwrap();
        assert_eq!(objs.len(), 4);
        assert_eq!((objs[0].object_type, objs[0].instance), (3, 1));
        assert_eq!(objs[0].properties[0].value, RpmValue::Number(1.0));
        assert_eq!((objs[1].object_type, objs[1].instance), (19, 2));
        assert_eq!(objs[1].properties[0].value, RpmValue::Number(300.0));
        assert_eq!((objs[2].object_type, objs[2].instance), (5, 3));
        assert_eq!(objs[2].properties[0].value, RpmValue::Number(1.0));
        assert_eq!((objs[3].object_type, objs[3].instance), (2, 4));
        assert_eq!(objs[3].properties[0].value, RpmValue::Number(-2.5));
        assert_eq!(objs[3].properties[1].value, RpmValue::Number(-3.0));
    }

    #[test]
    fn decodes_property_access_error() {
        // Annex F.3.7 second example: AI 7 Present_Value OK, AI 1 unknown-object error
        let data = [
            0x0C, 0x00, 0x00, 0x00, 0x07, 0x1E, 0x29, 0x55, 0x4E, 0x44, 0x41, 0x90, 0x00, 0x00,
            0x4F, 0x1F, // analog-input,7 present-value 18.0
            0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x29, 0x55, 0x5E, 0x91, 0x01, 0x91, 0x1F, 0x5F,
            0x1F, // analog-input,1 error(object, unknown-object)
        ];
        let objs = decode_rpm_ack(&data).unwrap();
        assert_eq!(objs.len(), 2);
        assert_eq!(objs[0].properties[0].value, RpmValue::Number(18.0));
        assert_eq!(
            objs[1].properties[0].value,
            RpmValue::Error { class: 1, code: 31 }
        );
    }

    #[test]
    fn non_numeric_and_array_index() {
        let data = [
            0x0C, 0x02, 0x00, 0x00, 0x09, 0x1E, // device,9
            0x29, 0x4D, 0x4E, 0x75, 0x04, 0x00, b'A', b'H', b'U', 0x4F, // object-name "AHU"
            0x29, 0x4C, 0x39, 0x00, 0x4E, 0x21, 0x05, 0x4F, // object-list[0] = 5
            0x29, 0x6F, 0x4E, 0x82, 0x04, 0x00, 0x4F, // status-flags bit string
            0x1F,
        ];
        let objs = decode_rpm_ack(&data).unwrap();
        let props = &objs[0].properties;
        assert_eq!(props[0].value, RpmValue::Unsupported);
        assert_eq!(props[1].array_index, Some(0));
        assert_eq!(props[1].value, RpmValue::Number(5.0));
        assert_eq!(props[2].value, RpmValue::Unsupported);
    }

    #[test]
    fn rejects_truncated_data() {
        let data = [
            0x0C, 0x00, 0x00, 0x00, 0x05, 0x1E, 0x29, 0x55, 0x4E, 0x44, 0x42,
        ];
        assert!(decode_rpm_ack(&data).is_err());
    }
}
