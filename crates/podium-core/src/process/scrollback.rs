//! Bounded ring buffer of raw terminal output (ANSI bytes included).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Default byte cap: 2 MiB of raw output per process.
pub const DEFAULT_CAP_BYTES: usize = 2 * 1024 * 1024;

/// Two chunks further apart than this belong to different bursts of output.
/// A working agent's spinner repaints every ~100 ms, so it never leaves a
/// gap this long; a lone redraw (after a resize, drawing a prompt) does.
pub const BURST_GAP: Duration = Duration::from_secs(1);

/// How recently, and for how long, a process has been printing. Backs the
/// UI's "working…" indicator: a lone redraw is one short burst, sustained
/// output (a spinner, streamed text) is a burst that keeps growing. Ages
/// rather than timestamps, so the frontend has no clock to align.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputActivity {
    /// Milliseconds since the last chunk of output.
    pub quiet_ms: u64,
    /// Milliseconds the latest burst lasted: from its first chunk (the first
    /// after a gap of at least [`BURST_GAP`]) to its last.
    pub burst_ms: u64,
}

/// Ring buffer of output chunks with a monotonically increasing sequence
/// number per appended chunk. Oldest chunks are evicted once the byte cap
/// is exceeded; the sequence keeps counting regardless.
#[derive(Debug)]
pub struct ScrollbackBuffer {
    chunks: VecDeque<Vec<u8>>,
    total_bytes: usize,
    cap_bytes: usize,
    next_seq: u64,
    /// When the last chunk arrived; `None` before any output. Drives the
    /// assistant's "this agent may be waiting on input" detection.
    last_append_at: Option<Instant>,
    /// First chunk of the burst `last_append_at` belongs to.
    burst_started_at: Option<Instant>,
}

impl ScrollbackBuffer {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAP_BYTES)
    }

    pub fn with_capacity(cap_bytes: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            total_bytes: 0,
            cap_bytes,
            next_seq: 0,
            last_append_at: None,
            burst_started_at: None,
        }
    }

    /// Append a chunk and return the sequence number assigned to it.
    pub fn append(&mut self, bytes: &[u8]) -> u64 {
        self.append_at(bytes, Instant::now())
    }

    /// [`Self::append`] with an explicit clock, so burst timing is testable.
    fn append_at(&mut self, bytes: &[u8], now: Instant) -> u64 {
        if self
            .last_append_at
            .is_none_or(|last| now.duration_since(last) > BURST_GAP)
        {
            self.burst_started_at = Some(now);
        }
        self.last_append_at = Some(now);
        let seq = self.next_seq;
        self.next_seq += 1;
        self.total_bytes += bytes.len();
        self.chunks.push_back(bytes.to_vec());
        // Evict oldest chunks over the cap, but always retain the newest
        // chunk even if it alone exceeds the cap.
        while self.total_bytes > self.cap_bytes && self.chunks.len() > 1 {
            if let Some(front) = self.chunks.pop_front() {
                self.total_bytes -= front.len();
            }
        }
        seq
    }

    /// Contiguous copy of everything retained, plus the next expected seq.
    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        let mut out = Vec::with_capacity(self.total_bytes);
        for chunk in &self.chunks {
            out.extend_from_slice(chunk);
        }
        (out, self.next_seq)
    }

    /// When output last arrived, or `None` while the process has been silent
    /// since it started.
    pub fn last_append_at(&self) -> Option<Instant> {
        self.last_append_at
    }

    /// Output timing as of `now`, or `None` while nothing was ever printed.
    pub fn output_activity(&self, now: Instant) -> Option<OutputActivity> {
        let last = self.last_append_at?;
        let start = self.burst_started_at.unwrap_or(last);
        let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        Some(OutputActivity {
            quiet_ms: ms(now.saturating_duration_since(last)),
            burst_ms: ms(last.saturating_duration_since(start)),
        })
    }

    /// The last `max` bytes of retained output.
    pub fn tail_bytes(&self, max: usize) -> Vec<u8> {
        let (all, _) = self.snapshot();
        if all.len() <= max {
            all
        } else {
            all[all.len() - max..].to_vec()
        }
    }
}

impl Default for ScrollbackBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_is_monotonic_per_chunk() {
        let mut buf = ScrollbackBuffer::new();
        assert_eq!(buf.append(b"a"), 0);
        assert_eq!(buf.append(b"bb"), 1);
        assert_eq!(buf.append(b""), 2);
        let (_, next) = buf.snapshot();
        assert_eq!(next, 3);
    }

    #[test]
    fn snapshot_returns_contiguous_bytes() {
        let mut buf = ScrollbackBuffer::new();
        buf.append(b"hello ");
        buf.append(b"world");
        let (bytes, next) = buf.snapshot();
        assert_eq!(bytes, b"hello world");
        assert_eq!(next, 2);
    }

    #[test]
    fn evicts_oldest_chunks_over_cap() {
        let mut buf = ScrollbackBuffer::with_capacity(10);
        buf.append(b"aaaa"); // 4 bytes
        buf.append(b"bbbb"); // 8 bytes
        buf.append(b"cccc"); // 12 -> evict "aaaa"
        let (bytes, next) = buf.snapshot();
        assert_eq!(bytes, b"bbbbcccc");
        assert_eq!(next, 3);

        buf.append(b"dddddddddd"); // 10-byte chunk evicts everything else
        let (bytes, next) = buf.snapshot();
        assert_eq!(bytes, b"dddddddddd");
        assert_eq!(next, 4);
    }

    #[test]
    fn oversized_single_chunk_is_retained() {
        let mut buf = ScrollbackBuffer::with_capacity(4);
        buf.append(b"toolongchunk");
        let (bytes, _) = buf.snapshot();
        assert_eq!(bytes, b"toolongchunk");
    }

    #[test]
    fn output_activity_tracks_quiet_time_and_burst_length() {
        let t0 = Instant::now();
        let mut buf = ScrollbackBuffer::new();
        assert_eq!(buf.output_activity(t0), None);

        // A spinner repainting every 100 ms is one growing burst.
        buf.append_at(b"a", t0);
        buf.append_at(b"b", t0 + Duration::from_millis(100));
        buf.append_at(b"c", t0 + Duration::from_millis(200));
        let a = buf
            .output_activity(t0 + Duration::from_millis(250))
            .unwrap();
        assert_eq!(
            a,
            OutputActivity {
                quiet_ms: 50,
                burst_ms: 200
            }
        );

        // A lone redraw after a silence longer than the gap is a fresh burst
        // of zero length — a resize repaint must not read as work.
        buf.append_at(b"d", t0 + Duration::from_secs(5));
        let a = buf
            .output_activity(t0 + Duration::from_millis(5010))
            .unwrap();
        assert_eq!(
            a,
            OutputActivity {
                quiet_ms: 10,
                burst_ms: 0
            }
        );
    }

    #[test]
    fn tail_bytes_returns_suffix() {
        let mut buf = ScrollbackBuffer::new();
        buf.append(b"0123456789");
        assert_eq!(buf.tail_bytes(4), b"6789");
        assert_eq!(buf.tail_bytes(100), b"0123456789");
    }
}
