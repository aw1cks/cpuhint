#![no_std]

//! Allocation-free parsing and affinity-mask construction for CPU views.

use core::fmt;

/// A validated Linux CPU list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuList {
    count: usize,
    contiguous_from_zero: bool,
}

impl CpuList {
    /// Returns the number of CPUs in the list.
    pub const fn count(self) -> usize {
        self.count
    }

    /// Returns whether the list is exactly `0..count`.
    pub const fn is_contiguous_from_zero(self) -> bool {
        self.contiguous_from_zero
    }
}

/// Why a CPU list could not be used as a process-visible CPU view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    Empty,
    InvalidCharacter,
    InvalidNumber,
    ReversedRange,
    UnorderedOrOverlapping,
    Overflow,
    ZeroCount,
    CountExceedsAbi,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "CPU list is empty",
            Self::InvalidCharacter => "CPU list contains invalid syntax",
            Self::InvalidNumber => "CPU list contains an invalid CPU number",
            Self::ReversedRange => "CPU list contains a reversed range",
            Self::UnorderedOrOverlapping => "CPU list is unordered or overlapping",
            Self::Overflow => "CPU list arithmetic overflowed",
            Self::ZeroCount => "CPU list contains no CPUs",
            Self::CountExceedsAbi => "CPU count does not fit the public ABI",
        };
        f.write_str(message)
    }
}

/// Parses Linux CPU-list syntax such as `0-3,8-11`.
///
/// Input must be globally ordered and non-overlapping. Whitespace is accepted
/// only at the beginning and end, matching the newline-terminated sysfs file.
pub fn parse_cpu_list(input: &[u8]) -> Result<CpuList, ParseError> {
    let input = trim_ascii_whitespace(input);
    if input.is_empty() {
        return Err(ParseError::Empty);
    }

    let mut offset = 0;
    let mut count = 0usize;
    let mut previous_end = None;
    let mut contiguous_from_zero = true;

    while offset < input.len() {
        let start = parse_number(input, &mut offset)?;
        let end = if input.get(offset) == Some(&b'-') {
            offset += 1;
            let end = parse_number(input, &mut offset)?;
            if end < start {
                return Err(ParseError::ReversedRange);
            }
            end
        } else {
            start
        };

        if let Some(previous_end) = previous_end {
            if start <= previous_end {
                return Err(ParseError::UnorderedOrOverlapping);
            }
            if start != previous_end.checked_add(1).ok_or(ParseError::Overflow)? {
                contiguous_from_zero = false;
            }
        } else if start != 0 {
            contiguous_from_zero = false;
        }

        let range_count = end
            .checked_sub(start)
            .and_then(|width| width.checked_add(1))
            .ok_or(ParseError::Overflow)?;
        count = count.checked_add(range_count).ok_or(ParseError::Overflow)?;
        if count > i32::MAX as usize {
            return Err(ParseError::CountExceedsAbi);
        }
        previous_end = Some(end);

        if offset == input.len() {
            break;
        }
        if input[offset] != b',' {
            return Err(ParseError::InvalidCharacter);
        }
        offset += 1;
        if offset == input.len() {
            return Err(ParseError::InvalidCharacter);
        }
    }

    if count == 0 {
        return Err(ParseError::ZeroCount);
    }

    Ok(CpuList {
        count,
        contiguous_from_zero,
    })
}

/// Why a synthetic mask could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaskError {
    EmptyCpuCount,
    BufferTooSmall,
}

/// Clears `mask` and sets bits for contiguous virtual CPUs `0..count`.
pub fn build_contiguous_mask(mask: &mut [u8], count: usize) -> Result<(), MaskError> {
    if count == 0 {
        return Err(MaskError::EmptyCpuCount);
    }
    if mask.len() < count.div_ceil(8) {
        return Err(MaskError::BufferTooSmall);
    }

    mask.fill(0);
    for cpu in 0..count {
        mask[cpu / 8] |= 1 << (cpu % 8);
    }
    Ok(())
}

fn trim_ascii_whitespace(mut input: &[u8]) -> &[u8] {
    while matches!(input.first(), Some(byte) if byte.is_ascii_whitespace()) {
        input = &input[1..];
    }
    while matches!(input.last(), Some(byte) if byte.is_ascii_whitespace()) {
        input = &input[..input.len() - 1];
    }
    input
}

fn parse_number(input: &[u8], offset: &mut usize) -> Result<usize, ParseError> {
    let start = *offset;
    let mut value = 0usize;
    while let Some(byte) = input.get(*offset) {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add((byte - b'0') as usize))
            .ok_or(ParseError::Overflow)?;
        *offset += 1;
    }
    if *offset == start {
        return Err(ParseError::InvalidNumber);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{MaskError, ParseError, build_contiguous_mask, parse_cpu_list};

    #[test]
    fn parses_contiguous_lists() {
        for (input, count) in [(b"0".as_slice(), 1), (b"0-3\n", 4), (b"\t0-7 \r\n", 8)] {
            let list = parse_cpu_list(input).unwrap();
            assert_eq!(list.count(), count);
            assert!(list.is_contiguous_from_zero());
        }
    }

    #[test]
    fn parses_disjoint_lists_without_affinity_eligibility() {
        let list = parse_cpu_list(b"0-3,8-11").unwrap();
        assert_eq!(list.count(), 8);
        assert!(!list.is_contiguous_from_zero());

        let list = parse_cpu_list(b"5-5").unwrap();
        assert_eq!(list.count(), 1);
        assert!(!list.is_contiguous_from_zero());
    }

    #[test]
    fn rejects_invalid_lists() {
        for (input, error) in [
            (b"".as_slice(), ParseError::Empty),
            (b"1-0".as_slice(), ParseError::ReversedRange),
            (b"0-3,3-4".as_slice(), ParseError::UnorderedOrOverlapping),
            (b"2,1".as_slice(), ParseError::UnorderedOrOverlapping),
            (b"0,".as_slice(), ParseError::InvalidCharacter),
            (b"0, 1".as_slice(), ParseError::InvalidNumber),
            (b"x".as_slice(), ParseError::InvalidNumber),
            (b"0-2147483647".as_slice(), ParseError::CountExceedsAbi),
            (b"18446744073709551616".as_slice(), ParseError::Overflow),
        ] {
            assert_eq!(parse_cpu_list(input), Err(error));
        }
    }

    #[test]
    fn builds_and_clears_a_mask() {
        let mut mask = [0xff; 3];
        build_contiguous_mask(&mut mask, 10).unwrap();
        assert_eq!(mask, [0xff, 0x03, 0x00]);

        let mut aligned_mask = [0; 1];
        build_contiguous_mask(&mut aligned_mask, 8).unwrap();
        assert_eq!(aligned_mask, [0xff]);
    }

    #[test]
    fn rejects_invalid_masks_without_modification() {
        let mut mask = [0xaa];
        assert_eq!(
            build_contiguous_mask(&mut mask, 9),
            Err(MaskError::BufferTooSmall)
        );
        assert_eq!(mask, [0xaa]);
        assert_eq!(
            build_contiguous_mask(&mut mask, 0),
            Err(MaskError::EmptyCpuCount)
        );
    }
}
