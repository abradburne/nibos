//! A/B slots: the boot control block that SPL reads to pick a slot, and what it decides
//! (`docs/design/ab-rollback.md`). `nibos-bootctl` reads and writes the block; SPL's own copy
//! of [`Block::boot`] is the RG DS+'s U-Boot patch, and the two must agree.
//!
//! The block is one 512-byte sector, written as two copies in a row at the end of the Falcon
//! partition: magic `NIBB`, version 1, the active slot, tries left and good per slot, a sequence
//! number (the valid copy with the higher one wins), zeros, and a CRC-32 (zlib's, as U-Boot's
//! `crc32(0, ...)`) of the first 508 bytes.

/// The block's size.
pub const SECTOR: usize = 512;
/// The block's two copies, in sectors from the Falcon partition's start (its last KiB).
pub const BLOCK_OFFSET: u64 = 0x20000 - 2;
/// Slot B's Falcon FIT, in sectors from the partition's start (32 MiB); slot A's is at 0.
pub const SLOT_B_OFFSET: u64 = 0x10000;
/// Tries a slot gets when it is set to be tried.
pub const TRIES: u8 = 3;

const MAGIC: &[u8; 4] = b"NIBB";
const VERSION: u8 = 1;

/// A slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    pub fn index(self) -> usize {
        match self {
            Slot::A => 0,
            Slot::B => 1,
        }
    }
    pub fn other(self) -> Slot {
        match self {
            Slot::A => Slot::B,
            Slot::B => Slot::A,
        }
    }
    pub fn parse(s: &str) -> Option<Slot> {
        match s {
            "a" | "A" => Some(Slot::A),
            "b" | "B" => Some(Slot::B),
            _ => None,
        }
    }
    pub fn name(self) -> char {
        match self {
            Slot::A => 'A',
            Slot::B => 'B',
        }
    }
}

/// The block's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub active: Slot,
    pub tries: [u8; 2],
    pub good: [bool; 2],
    pub seq: u32,
}

/// What SPL boots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boot {
    /// This slot's Falcon FIT.
    Slot(Slot),
    /// The fallback U-Boot: no slot is good and none has tries left.
    Fallback,
}

/// zlib's CRC-32, as U-Boot's `crc32(0, buf, len)`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

impl Block {
    /// A new block: slot A active and good, B empty.
    pub fn new() -> Block {
        Block {
            active: Slot::A,
            tries: [0, 0],
            good: [true, false],
            seq: 1,
        }
    }

    pub fn encode(&self) -> [u8; SECTOR] {
        let mut s = [0u8; SECTOR];
        s[0..4].copy_from_slice(MAGIC);
        s[4] = VERSION;
        s[5] = self.active.index() as u8;
        s[6..8].copy_from_slice(&self.tries);
        s[8] = self.good[0] as u8;
        s[9] = self.good[1] as u8;
        s[10..14].copy_from_slice(&self.seq.to_le_bytes());
        let crc = crc32(&s[..508]);
        s[508..].copy_from_slice(&crc.to_le_bytes());
        s
    }

    /// One copy, if it is whole.
    pub fn decode(s: &[u8]) -> Option<Block> {
        if s.len() < SECTOR || &s[0..4] != MAGIC || s[4] != VERSION || s[5] > 1 {
            return None;
        }
        if crc32(&s[..508]) != u32::from_le_bytes(s[508..512].try_into().ok()?) {
            return None;
        }
        Some(Block {
            active: if s[5] == 0 { Slot::A } else { Slot::B },
            tries: [s[6].min(7), s[7].min(7)],
            good: [s[8] != 0, s[9] != 0],
            seq: u32::from_le_bytes(s[10..14].try_into().ok()?),
        })
    }

    /// The two copies as read: the whole one with the higher sequence number.
    pub fn from_copies(first: &[u8], second: &[u8]) -> Option<Block> {
        match (Block::decode(first), Block::decode(second)) {
            (Some(a), Some(b)) => Some(if b.seq > a.seq { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    /// Which copy to write first: the one that isn't the newest, so one stays whole.
    pub fn older_copy(first: &[u8], second: &[u8]) -> usize {
        match (Block::decode(first), Block::decode(second)) {
            (Some(a), Some(b)) if b.seq < a.seq => 1,
            (Some(_), None) => 1,
            _ => 0,
        }
    }

    /// What SPL does with the block at power-on (A not held): the slot to boot, and the
    /// block to write first if it changed (a try used up, or a rollback).
    pub fn boot(&self) -> (Boot, Option<Block>) {
        let s = self.active;
        if self.good[s.index()] {
            return (Boot::Slot(s), None);
        }
        let mut next = self.clone();
        next.seq = self.seq.wrapping_add(1);
        if self.tries[s.index()] > 0 {
            next.tries[s.index()] -= 1;
            return (Boot::Slot(s), Some(next));
        }
        let o = s.other();
        if self.good[o.index()] {
            next.active = o;
            return (Boot::Slot(o), Some(next));
        }
        (Boot::Fallback, None)
    }

    /// The running slot is confirmed good.
    pub fn mark_good(&self) -> Block {
        let mut next = self.clone();
        next.good[self.active.index()] = true;
        next.seq = self.seq.wrapping_add(1);
        next
    }

    /// `slot` is to be tried next: active, [`TRIES`] tries, not good.
    pub fn try_slot(&self, slot: Slot) -> Block {
        let mut next = self.clone();
        next.active = slot;
        next.tries[slot.index()] = TRIES;
        next.good[slot.index()] = false;
        next.seq = self.seq.wrapping_add(1);
        next
    }

    /// Back to the other slot, if it is good (the safe mode's "go back").
    pub fn rollback(&self) -> Option<Block> {
        let o = self.active.other();
        self.good[o.index()].then(|| {
            let mut next = self.clone();
            next.active = o;
            next.tries[self.active.index()] = 0;
            next.seq = self.seq.wrapping_add(1);
            next
        })
    }
}

impl Default for Block {
    fn default() -> Block {
        Block::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_is_zlibs() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn round_trip_and_damage() {
        let b = Block::new().try_slot(Slot::B);
        let s = b.encode();
        assert_eq!(Block::decode(&s), Some(b.clone()));
        let mut bad = s;
        bad[7] ^= 1;
        assert_eq!(Block::decode(&bad), None, "the CRC catches a flipped bit");
        assert_eq!(Block::decode(&[0u8; SECTOR]), None, "an empty card");
    }

    #[test]
    fn the_newer_whole_copy_wins() {
        let old = Block::new();
        let new = old.try_slot(Slot::B);
        let (o, n) = (old.encode(), new.encode());
        assert_eq!(Block::from_copies(&o, &n), Some(new.clone()));
        assert_eq!(Block::from_copies(&n, &o), Some(new.clone()));
        let mut torn = n;
        torn[100] = 1;
        assert_eq!(
            Block::from_copies(&torn, &o),
            Some(old),
            "a torn write leaves the old"
        );
        assert_eq!(Block::older_copy(&o, &n), 0);
        assert_eq!(Block::older_copy(&n, &o), 1);
    }

    #[test]
    fn a_good_slot_boots_without_a_write() {
        assert_eq!(Block::new().boot(), (Boot::Slot(Slot::A), None));
    }

    #[test]
    fn a_bad_update_rolls_back_after_three_tries() {
        let mut b = Block::new().try_slot(Slot::B);
        for left in (0..TRIES).rev() {
            let (boot, next) = b.boot();
            assert_eq!(boot, Boot::Slot(Slot::B));
            b = next.expect("a try is written");
            assert_eq!(b.tries[1], left);
        }
        let (boot, next) = b.boot();
        assert_eq!(boot, Boot::Slot(Slot::A), "back to A");
        let b = next.unwrap();
        assert_eq!(b.active, Slot::A);
        assert_eq!(
            b.boot(),
            (Boot::Slot(Slot::A), None),
            "and A stays, no writes"
        );
    }

    #[test]
    fn a_good_update_stays() {
        let b = Block::new().try_slot(Slot::B);
        let (_, b) = b.boot();
        let b = b.unwrap().mark_good();
        assert_eq!(b.boot(), (Boot::Slot(Slot::B), None));
        assert!(b.good[0], "A stays good to go back to");
    }

    #[test]
    fn nothing_good_is_the_fallback() {
        let b = Block {
            active: Slot::B,
            tries: [0, 0],
            good: [false, false],
            seq: 9,
        };
        assert_eq!(b.boot(), (Boot::Fallback, None));
    }

    #[test]
    fn going_back_needs_a_good_slot() {
        let b = Block::new().try_slot(Slot::B);
        let back = b.rollback().unwrap();
        assert_eq!(back.active, Slot::A);
        assert_eq!(back.tries[1], 0);
        let only_b = Block {
            active: Slot::B,
            tries: [0, 2],
            good: [false, false],
            seq: 1,
        };
        assert_eq!(only_b.rollback(), None);
    }
}
