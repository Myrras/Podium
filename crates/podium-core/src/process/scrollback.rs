//! Bounded ring buffer of raw terminal output (ANSI bytes included).

use std::collections::VecDeque;

/// Default byte cap: 2 MiB of raw output per process.
pub const DEFAULT_CAP_BYTES: usize = 2 * 1024 * 1024;

/// Ring buffer of output chunks with a monotonically increasing sequence
/// number per appended chunk. Oldest chunks are evicted once the byte cap
/// is exceeded; the sequence keeps counting regardless.
#[derive(Debug)]
pub struct ScrollbackBuffer {
    chunks: VecDeque<Vec<u8>>,
    total_bytes: usize,
    cap_bytes: usize,
    next_seq: u64,
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
        }
    }

    /// Append a chunk and return the sequence number assigned to it.
    pub fn append(&mut self, bytes: &[u8]) -> u64 {
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
    fn tail_bytes_returns_suffix() {
        let mut buf = ScrollbackBuffer::new();
        buf.append(b"0123456789");
        assert_eq!(buf.tail_bytes(4), b"6789");
        assert_eq!(buf.tail_bytes(100), b"0123456789");
    }
}
