//! A stable 64-bit content hash.
//!
//! D-035 seeds motion selection from stable identity, and D-043 keys the cache
//! on content. Both require a hash whose value is *the same next week, on
//! another machine, under another compiler* — otherwise a re-render picks a
//! different pan and every cache entry misses.
//!
//! `std::collections::hash_map::DefaultHasher` cannot be used for this. Its
//! documentation explicitly declines to guarantee stability across releases,
//! and `RandomState` reseeds per process. Using it here would produce a build
//! that renders correctly, caches correctly, and then silently invalidates
//! every cached segment after a compiler upgrade.
//!
//! So: FNV-1a, 64-bit, written out. It is not cryptographic and does not need
//! to be — nothing here is a security boundary, and the property we need is
//! reproducibility, not collision resistance.

/// FNV-1a 64-bit offset basis, from the reference specification.
const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime, from the reference specification.
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// An FNV-1a hash in progress.
///
/// The incremental form exists because some of the things D-043 hashes do not
/// fit comfortably in memory: a narration file can be hundreds of megabytes,
/// and reading it whole to hash it would make the cache key more expensive
/// than the work it saves. Feeding it in chunks produces the same value as
/// hashing the whole slice — asserted below, because a streaming hash that
/// disagreed with the one-shot hash would split every cache in two.
#[derive(Debug, Clone, Copy)]
pub struct Fnv1a {
    state: u64,
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv1a {
    /// A hash of nothing yet.
    #[must_use]
    pub const fn new() -> Self {
        Fnv1a {
            state: OFFSET_BASIS,
        }
    }

    /// Feed the next chunk.
    pub fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= u64::from(byte);
            self.state = self.state.wrapping_mul(PRIME);
        }
    }

    /// The hash of everything fed so far.
    #[must_use]
    pub const fn finish(&self) -> u64 {
        self.state
    }
}

/// Hash a byte slice with FNV-1a.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = Fnv1a::new();
    hash.write(bytes);
    hash.finish()
}

/// Hash several fields as one value, with an unambiguous separator.
///
/// The separator matters. Without it `("ab", "c")` and `("a", "bc")` hash
/// identically, which would let two different scenes share a motion seed and a
/// cache key. `0x1f` (ASCII unit separator) cannot occur in a path, a project
/// id, or a hex digest, so it cannot be forged by the field contents.
#[must_use]
pub fn fnv1a_fields(fields: &[&[u8]]) -> u64 {
    let mut hash = OFFSET_BASIS;
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            hash ^= 0x1f;
            hash = hash.wrapping_mul(PRIME);
        }
        for &byte in *field {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Published FNV-1a 64 test vectors. If these ever change, every cache key
    /// and every seeded motion choice in every existing project has changed
    /// with them — which is exactly why they are pinned here.
    #[test]
    fn matches_the_published_reference_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }

    /// The separator is the whole point of `fnv1a_fields`.
    #[test]
    fn field_boundaries_are_unambiguous() {
        assert_ne!(
            fnv1a_fields(&[b"ab", b"c"]),
            fnv1a_fields(&[b"a", b"bc"]),
            "field boundaries must survive hashing, or two scenes collide"
        );
    }

    /// Concatenation is not the same as field hashing, and must not be.
    #[test]
    fn is_not_plain_concatenation() {
        assert_ne!(fnv1a_fields(&[b"ab", b"c"]), fnv1a(b"abc"));
    }

    /// A chunked hash and a one-shot hash must agree, or the same file hashes
    /// differently depending on how it happened to be read.
    #[test]
    fn streaming_agrees_with_the_one_shot_form() {
        let data: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
        let mut streamed = Fnv1a::new();
        for chunk in data.chunks(37) {
            streamed.write(chunk);
        }
        assert_eq!(streamed.finish(), fnv1a(&data));
        assert_eq!(Fnv1a::new().finish(), fnv1a(b""));
    }

    #[test]
    fn is_deterministic_across_calls() {
        assert_eq!(
            fnv1a_fields(&[b"proj", b"7"]),
            fnv1a_fields(&[b"proj", b"7"])
        );
    }
}
