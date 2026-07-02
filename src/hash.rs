//! Content hashing for drift detection (ADR-0008).
//!
//! Both modes hash **verbatim source bytes** with xxh3-64. Two tiers guard each
//! edit: a short per-anchor hash catches a localized change, and a longer
//! file-level hash guards the whole snapshot. Hashing is strict — any byte
//! change, including whitespace, is drift.

use xxhash_rust::xxh3::xxh3_64;

/// Per-anchor content hash: xxh3-64 truncated to **4 hex digits** (ADR-0008).
///
/// Short by design — the file-level hash is the global guard, so a per-anchor
/// hash only needs to distinguish edit sites within one read.
pub fn anchor_hash(bytes: &[u8]) -> String {
    format!("{:04x}", (xxh3_64(bytes) & 0xffff) as u16)
}

/// File-level snapshot hash: the full xxh3-64 as **16 hex digits**.
///
/// Longer than an anchor hash because it must distinguish whole-file states,
/// not just one edit site.
pub fn file_hash(bytes: &[u8]) -> String {
    format!("{:016x}", xxh3_64(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_have_the_specified_widths() {
        assert_eq!(anchor_hash(b"anything").len(), 4);
        assert_eq!(file_hash(b"anything").len(), 16);
    }

    #[test]
    fn hashing_is_deterministic() {
        assert_eq!(anchor_hash(b"(defun f ())"), anchor_hash(b"(defun f ())"));
        assert_eq!(file_hash(b"(defun f ())"), file_hash(b"(defun f ())"));
    }

    #[test]
    fn a_single_byte_change_is_drift() {
        assert_ne!(anchor_hash(b"(f x)"), anchor_hash(b"(f  x)"));
        assert_ne!(file_hash(b"(f x)"), file_hash(b"(f  x)"));
    }
}
