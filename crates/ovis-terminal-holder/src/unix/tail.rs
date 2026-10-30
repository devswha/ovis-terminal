//! Bounded retained raw output per terminal, addressed by absolute byte offset.

use std::collections::VecDeque;

use super::error::HolderError;

/// The newest `capacity` raw output bytes and the absolute offset after them.
#[derive(Debug)]
pub(super) struct RetainedTail {
    bytes: VecDeque<u8>,
    capacity: usize,
    head: u64,
}

/// Retained bytes from `from` up to the output head.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct TailSlice {
    pub(super) from: u64,
    pub(super) through: u64,
    pub(super) bytes: Vec<u8>,
    pub(super) gap: bool,
}

impl RetainedTail {
    pub(super) const fn new(capacity: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            capacity,
            head: 0,
        }
    }

    /// Absolute offset immediately after the newest output byte.
    pub(super) const fn head(&self) -> u64 {
        self.head
    }

    /// Appends output and returns the absolute offset of its first byte.
    pub(super) fn append(&mut self, chunk: &[u8]) -> Result<u64, HolderError> {
        let start = self.head;
        let length = u64::try_from(chunk.len()).map_err(|_| HolderError::CounterExhausted)?;
        self.head = start
            .checked_add(length)
            .ok_or(HolderError::CounterExhausted)?;
        self.bytes.extend(chunk);
        let excess = self.bytes.len().saturating_sub(self.capacity);
        self.bytes.drain(..excess);
        Ok(start)
    }

    /// Returns retained output from `from`, flagging a gap when it was evicted.
    pub(super) fn read_from(&self, from: u64) -> Result<TailSlice, HolderError> {
        if from > self.head {
            return Err(HolderError::OffsetBeyondHead {
                requested: from,
                head: self.head,
            });
        }
        let retained =
            u64::try_from(self.bytes.len()).map_err(|_| HolderError::CounterExhausted)?;
        let floor = self.head.saturating_sub(retained);
        let start = from.max(floor);
        let skip = usize::try_from(start.saturating_sub(floor))
            .map_err(|_| HolderError::CounterExhausted)?;
        Ok(TailSlice {
            from: start,
            through: self.head,
            bytes: self.bytes.iter().skip(skip).copied().collect(),
            gap: from < floor,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::RetainedTail;

    proptest! {
        #[test]
        fn read_from_returns_the_retained_suffix_of_the_stream(
            chunks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..40), 0..12),
            capacity in 1_usize..64,
            from_seed in any::<usize>(),
        ) {
            let mut tail = RetainedTail::new(capacity);
            let mut stream = Vec::new();
            for chunk in &chunks {
                let start = tail.append(chunk).expect("append");
                prop_assert_eq!(start, u64::try_from(stream.len()).expect("offset"));
                stream.extend_from_slice(chunk);
            }
            let total = stream.len();
            let from = from_seed.checked_rem(total.saturating_add(1)).expect("nonzero modulus");
            let floor = total.saturating_sub(capacity);

            let slice = tail.read_from(u64::try_from(from).expect("from")).expect("read");

            let start = from.max(floor);
            prop_assert_eq!(slice.from, u64::try_from(start).expect("start"));
            prop_assert_eq!(slice.through, u64::try_from(total).expect("total"));
            prop_assert_eq!(slice.bytes.as_slice(), stream.get(start..).expect("suffix"));
            prop_assert_eq!(slice.gap, from < floor);
        }
    }

    #[test]
    fn offset_beyond_head_is_rejected() {
        let mut tail = RetainedTail::new(8);
        tail.append(b"abc").expect("append");

        let error = tail.read_from(4).expect_err("offset 4 is past head 3");

        assert_eq!(error.kind(), "offset_beyond_head");
    }
}
