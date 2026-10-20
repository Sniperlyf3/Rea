//! Jitter / playout buffer.
//!
//! Incoming media packets arrive over unreliable QUIC datagrams: they may be
//! lost, duplicated or reordered. This buffer absorbs network jitter, restores
//! ordering, de-duplicates, and hands the playout clock exactly one outcome per
//! tick: a frame to decode, a gap to conceal, or starvation (pre-buffering).
//!
//! Sequence numbers are 16-bit and wrap; we extend them to a monotonic 64-bit
//! space using RFC 3550-style rollover tracking so ordering survives the wrap.

use std::collections::BTreeMap;

/// Extends wrapping 16-bit sequence numbers to a monotonic 64-bit value.
#[derive(Debug, Default)]
pub struct SeqExtender {
    initialized: bool,
    last: u16,
    cycles: u64,
}

impl SeqExtender {
    pub fn extend(&mut self, seq: u16) -> u64 {
        if !self.initialized {
            self.initialized = true;
            self.last = seq;
            return seq as u64;
        }
        // Signed 16-bit delta tells us whether we moved forward or wrapped.
        let delta = seq.wrapping_sub(self.last) as i16;
        if delta >= 0 {
            if seq < self.last {
                // forward across a wrap boundary
                self.cycles += 1 << 16;
            }
        } else if seq > self.last {
            // backward across a wrap boundary (late packet from previous cycle)
            return self.cycles.wrapping_sub(1 << 16) + seq as u64;
        }
        self.last = seq;
        self.cycles + seq as u64
    }
}

/// What the playout clock should do this tick.
#[derive(Debug, PartialEq, Eq)]
pub enum Playout {
    /// Decode and render this encoded payload.
    Frame(Vec<u8>),
    /// Expected packet is missing; conceal the loss (decoder PLC or silence).
    Conceal,
    /// Not enough buffered yet; stay silent while pre-buffering.
    Starved,
}

/// Running counters exposed for diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub struct JitterStats {
    pub pushed: u64,
    pub played: u64,
    pub concealed: u64,
    pub duplicates: u64,
    pub late_drops: u64,
    pub overflow_drops: u64,
    pub depth: u32,
}

/// Fixed-target jitter buffer keyed by extended sequence number.
pub struct JitterBuffer {
    target: usize,
    cap: usize,
    map: BTreeMap<u64, Vec<u8>>,
    ext: SeqExtender,
    next: Option<u64>,
    prebuffering: bool,
    stats: JitterStats,
}

impl JitterBuffer {
    /// `target_frames`: how many frames to buffer before playout starts (and
    /// the depth we try to maintain). `max_frames`: hard cap before we drop the
    /// oldest to bound latency.
    pub fn new(target_frames: usize, max_frames: usize) -> Self {
        JitterBuffer {
            target: target_frames.max(1),
            cap: max_frames.max(target_frames.max(1) + 1),
            map: BTreeMap::new(),
            ext: SeqExtender::default(),
            next: None,
            prebuffering: true,
            stats: JitterStats::default(),
        }
    }

    pub fn stats(&self) -> JitterStats {
        let mut s = self.stats;
        s.depth = self.map.len() as u32;
        s
    }

    /// Insert a received packet.
    pub fn push(&mut self, seq: u16, payload: Vec<u8>) {
        let key = self.ext.extend(seq);
        self.stats.pushed += 1;

        if let Some(next) = self.next {
            if key < next {
                // Already played past this one.
                self.stats.late_drops += 1;
                return;
            }
        }
        if self.map.contains_key(&key) {
            self.stats.duplicates += 1;
            return;
        }
        self.map.insert(key, payload);

        // Bound buffering: if we overflow, drop the oldest and resync.
        while self.map.len() > self.cap {
            if let Some((&oldest, _)) = self.map.iter().next() {
                self.map.remove(&oldest);
                self.stats.overflow_drops += 1;
                // Playback head follows the drop so we don't conceal forever.
                if let Some(next) = self.next {
                    if oldest == next {
                        self.next = Some(next + 1);
                    }
                }
            } else {
                break;
            }
        }
    }

    /// Advance the playout clock by one frame.
    pub fn pop(&mut self) -> Playout {
        if self.prebuffering {
            if self.map.len() < self.target {
                return Playout::Starved;
            }
            self.prebuffering = false;
            // Anchor playback at the oldest buffered packet.
            self.next = self.map.keys().next().copied();
        }

        let next = match self.next {
            Some(n) => n,
            None => return Playout::Starved,
        };

        if let Some(payload) = self.map.remove(&next) {
            self.next = Some(next + 1);
            self.stats.played += 1;
            Playout::Frame(payload)
        } else if self.map.is_empty() {
            // Nothing buffered at all — fall back to pre-buffering to rebuild depth.
            self.prebuffering = true;
            self.stats.concealed += 1;
            Playout::Conceal
        } else {
            // A gap: conceal and step over the missing sequence.
            self.next = Some(next + 1);
            self.stats.concealed += 1;
            Playout::Conceal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(b: u8) -> Vec<u8> {
        vec![b]
    }

    #[test]
    fn seq_extender_wraps() {
        let mut e = SeqExtender::default();
        assert_eq!(e.extend(65534), 65534);
        assert_eq!(e.extend(65535), 65535);
        assert_eq!(e.extend(0), 65536);
        assert_eq!(e.extend(1), 65537);
    }

    #[test]
    fn in_order_playout() {
        let mut j = JitterBuffer::new(2, 8);
        assert_eq!(j.pop(), Playout::Starved);
        j.push(0, p(0));
        j.push(1, p(1));
        assert_eq!(j.pop(), Playout::Frame(p(0)));
        j.push(2, p(2));
        assert_eq!(j.pop(), Playout::Frame(p(1)));
        assert_eq!(j.pop(), Playout::Frame(p(2)));
    }

    #[test]
    fn reorders_and_dedups() {
        let mut j = JitterBuffer::new(2, 8);
        j.push(1, p(1));
        j.push(0, p(0));
        j.push(1, p(1)); // duplicate
        assert_eq!(j.pop(), Playout::Frame(p(0)));
        assert_eq!(j.pop(), Playout::Frame(p(1)));
        assert_eq!(j.stats().duplicates, 1);
    }

    #[test]
    fn conceals_gap() {
        let mut j = JitterBuffer::new(2, 8);
        j.push(0, p(0));
        j.push(2, p(2)); // seq 1 lost
        assert_eq!(j.pop(), Playout::Frame(p(0)));
        assert_eq!(j.pop(), Playout::Conceal);
        assert_eq!(j.pop(), Playout::Frame(p(2)));
        assert_eq!(j.stats().concealed, 1);
    }
}
