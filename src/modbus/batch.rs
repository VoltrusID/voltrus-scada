//! Grouping of a device's registers into Modbus read requests.

use crate::models::RegisterConfig;

/// Most registers one Read Holding Registers (0x03) or Read Input Registers
/// (0x04) request may ask for (Modbus Application Protocol 1.1b3, 6.3 and 6.4).
pub(super) const MAX_REGISTERS_PER_READ: u32 = 125;

/// Most bits one Read Coils (0x01) or Read Discrete Inputs (0x02) request may
/// ask for (6.1 and 6.2).
pub(super) const MAX_BITS_PER_READ: u32 = 2000;

/// What a register area is read with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadKind {
    Holding,
    Input,
    Coil,
    Discrete,
}

impl ReadKind {
    /// Unknown area names read as holding registers, like the register scanner.
    pub(super) fn from_area(area: &str) -> Self {
        match area {
            "input" => Self::Input,
            "coil" => Self::Coil,
            "discrete" => Self::Discrete,
            _ => Self::Holding,
        }
    }

    fn max_per_read(self) -> u32 {
        match self {
            Self::Holding | Self::Input => MAX_REGISTERS_PER_READ,
            Self::Coil | Self::Discrete => MAX_BITS_PER_READ,
        }
    }

    /// How many addresses the register occupies in this area: a coil or discrete
    /// input is one bit whatever the register type says.
    pub(super) fn width(self, reg: &RegisterConfig) -> u32 {
        match self {
            Self::Holding | Self::Input => u32::from(reg.register_count()),
            Self::Coil | Self::Discrete => 1,
        }
    }
}

/// One read request and the registers (as a range of the sorted slice given to
/// [`plan_batches`]) it answers.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ReadBatch {
    pub kind: ReadKind,
    pub start: u16,
    pub count: u16,
    pub first: usize,
    pub end: usize,
}

/// Order registers by area, then address, so [`plan_batches`] can merge neighbours.
pub(super) fn sort_registers(regs: &mut [&RegisterConfig]) {
    regs.sort_by_key(|r| (r.register_area.as_str().to_owned(), r.protocol_address()));
}

/// Merge registers that touch or overlap in the same area into one request,
/// starting a new request whenever the current one would exceed the protocol
/// limit for its function code. A register is never split across requests.
/// `regs` must be sorted by [`sort_registers`]. A register that does not fit in
/// the address space (it would run past 65535) is read as far as it goes and
/// answered NaN by the caller.
pub(super) fn plan_batches(regs: &[&RegisterConfig]) -> Vec<ReadBatch> {
    let mut batches = Vec::new();
    let mut i = 0;
    while i < regs.len() {
        let first = regs[i];
        let kind = ReadKind::from_area(&first.register_area);
        let limit = kind.max_per_read();
        let start = u32::from(first.protocol_address());
        let mut end = start + kind.width(first).min(limit);

        let mut group_end = i + 1;
        while group_end < regs.len() {
            let next = regs[group_end];
            if next.register_area != first.register_area {
                break;
            }
            let next_start = u32::from(next.protocol_address());
            if next_start > end {
                break;
            }
            let next_end = (next_start + kind.width(next)).max(end);
            if next_end - start > limit {
                break;
            }
            end = next_end;
            group_end += 1;
        }

        // The wire address space ends at 65535.
        let end = end.min(0x1_0000);
        batches.push(ReadBatch {
            kind,
            start: start as u16,
            count: (end - start) as u16,
            first: i,
            end: group_end,
        });
        i = group_end;
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RegisterType;

    fn reg(name: &str, address: u16, reg_type: RegisterType, area: &str) -> RegisterConfig {
        RegisterConfig {
            name: name.into(),
            address,
            reg_type,
            scale: 1.0,
            register_area: area.into(),
            zero_based: true,
        }
    }

    fn plan(regs: &[RegisterConfig]) -> Vec<ReadBatch> {
        let mut refs: Vec<&RegisterConfig> = regs.iter().collect();
        sort_registers(&mut refs);
        plan_batches(&refs)
    }

    fn contiguous(n: u16, area: &str) -> Vec<RegisterConfig> {
        (0..n)
            .map(|a| reg(&format!("r{a}"), a, RegisterType::Uint16, area))
            .collect()
    }

    #[test]
    fn adjacent_registers_share_one_request() {
        let b = plan(&contiguous(100, "holding"));
        assert_eq!(b.len(), 1);
        assert_eq!(
            (b[0].start, b[0].count, b[0].first, b[0].end),
            (0, 100, 0, 100)
        );
    }

    #[test]
    fn exactly_125_registers_is_one_request() {
        let b = plan(&contiguous(125, "holding"));
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].count, 125);
    }

    #[test]
    fn a_126th_register_starts_a_second_request() {
        let b = plan(&contiguous(126, "input"));
        assert_eq!(b.len(), 2);
        assert_eq!(
            (b[0].kind, b[0].start, b[0].count),
            (ReadKind::Input, 0, 125)
        );
        assert_eq!(
            (b[1].start, b[1].count, b[1].first, b[1].end),
            (125, 1, 125, 126)
        );
    }

    #[test]
    fn no_request_exceeds_the_limit_and_every_register_is_covered_once() {
        let regs = contiguous(1000, "holding");
        let b = plan(&regs);
        assert_eq!(b.len(), 8);
        assert!(
            b.iter()
                .all(|r| u32::from(r.count) <= MAX_REGISTERS_PER_READ)
        );
        let mut next = 0;
        for r in &b {
            assert_eq!(r.first, next, "ranges are contiguous");
            next = r.end;
        }
        assert_eq!(next, 1000);
    }

    #[test]
    fn a_multi_word_register_is_not_split_at_the_boundary() {
        // 124 single registers, then a float32 at 124..126: it does not fit in the
        // first request (126 > 125) and moves whole into the next.
        let mut regs = contiguous(124, "holding");
        regs.push(reg("f", 124, RegisterType::Float32, "holding"));
        let b = plan(&regs);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].start, b[0].count, b[0].end), (0, 124, 124));
        assert_eq!(
            (b[1].start, b[1].count, b[1].first, b[1].end),
            (124, 2, 124, 125)
        );
    }

    #[test]
    fn a_gap_or_another_area_starts_a_new_request() {
        let regs = vec![
            reg("a", 0, RegisterType::Uint16, "holding"),
            reg("b", 2, RegisterType::Uint16, "holding"),
            reg("c", 0, RegisterType::Uint16, "input"),
        ];
        let b = plan(&regs);
        assert_eq!(b.len(), 3);
        assert_eq!(
            (b[0].kind, b[0].start, b[0].count),
            (ReadKind::Holding, 0, 1)
        );
        assert_eq!(
            (b[1].kind, b[1].start, b[1].count),
            (ReadKind::Holding, 2, 1)
        );
        assert_eq!((b[2].kind, b[2].start, b[2].count), (ReadKind::Input, 0, 1));
    }

    #[test]
    fn overlapping_registers_merge() {
        let regs = vec![
            reg("f", 10, RegisterType::Float32, "holding"),
            reg("g", 11, RegisterType::Uint16, "holding"),
        ];
        let b = plan(&regs);
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].start, b[0].count), (10, 2));
    }

    #[test]
    fn coils_split_at_2000_bits() {
        let b = plan(&contiguous(2001, "coil"));
        assert_eq!(b.len(), 2);
        assert_eq!(
            (b[0].kind, b[0].start, b[0].count),
            (ReadKind::Coil, 0, 2000)
        );
        assert_eq!((b[1].start, b[1].count), (2000, 1));

        let b = plan(&contiguous(2000, "discrete"));
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].kind, b[0].count), (ReadKind::Discrete, 2000));
    }

    #[test]
    fn a_register_at_the_top_of_the_address_space_does_not_overflow() {
        let regs = vec![reg("f", 65535, RegisterType::Float32, "holding")];
        let b = plan(&regs);
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].start, b[0].count), (65535, 1));
    }
}
