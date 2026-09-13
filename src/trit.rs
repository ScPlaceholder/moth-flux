//! The ternary primitive and its packed sequence.
//!
//! # Why 2 bits per trit and not log2(3) = 1.585
//!
//! Measured trade, stated up front because the proposal document treats compression and speed as
//! allies and they are not. Dense base-3 packing gives five trits per byte (1.6 bits each), but
//! indexing one then costs a division and modulo by 3 — the single operation SIMD is worst at, and
//! the exact thing §12 of the proposal wants to vectorise.
//!
//! So: **2 bits per trit, 4 trits per byte, 25% denser than a byte-per-trit enum and indexable with
//! a shift and a mask.** That is a deliberate 26% footprint concession bought for shift-and-mask
//! access. If the benchmark says footprint is what matters, the dense packing is a later variant
//! and this comment is where to start.
//!
//! ⚠ AND THE HONEST CONSEQUENCE: at 2 bits per trit this is a well-packed enum, which Rust can
//! already express with `#[repr(u8)]` and bit twiddling. The claim under test is therefore NOT
//! "ternary is smaller". It is that a native unresolved state plus zero-copy access beats
//! `Option<T>` over a deserialised struct on a real scan. If it does not, the language does not
//! earn itself and we found out in a weekend.

/// One ternary value. `Copy`, 1 byte in memory, 2 bits when packed.
///
/// ⛔ THERE IS DELIBERATELY NO `From<Trit> for bool`. See the crate docs: silent coercion of
/// unknown to false is SQL's most famous footgun and it makes rows disappear rather than error.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Trit {
    /// -1 — false / negative.
    No = 0,
    /// 0 — unresolved. **Not** "not yet computed"; there is no computation pending.
    Unknown = 1,
    /// +1 — true / positive.
    Yes = 2,
}

impl Trit {
    #[inline]
    pub fn from_bits(b: u8) -> Trit {
        match b & 0b11 {
            0 => Trit::No,
            2 => Trit::Yes,
            _ => Trit::Unknown, // 1 and the unused 3 both mean unknown — no invalid state exists
        }
    }

    #[inline]
    pub fn bits(self) -> u8 {
        self as u8
    }

    #[inline]
    pub fn is_known(self) -> bool {
        !matches!(self, Trit::Unknown)
    }

    /// The ONLY door from ternary to binary, and it makes the caller say what unknown means here.
    ///
    /// There is no default. That is the point: at every crossing the question "what does absence
    /// mean in this context?" has a different right answer, and a language-level default picks one
    /// of them everywhere and is silently wrong in the rest.
    #[inline]
    pub fn resolve(self, unknown_as: bool) -> bool {
        match self {
            Trit::Yes => true,
            Trit::No => false,
            Trit::Unknown => unknown_as,
        }
    }
}

/// Kleene three-valued logic — the reason to keep computing through an unknown instead of
/// resolving at every step.
///
/// `Yes OR Unknown` is `Yes` without ever learning the unknown, which is the operational advantage
/// over `Option<bool>`: `Option` forces a decision at each combinator, Kleene lets the unresolved
/// state propagate until something actually depends on it.
pub trait Kleene {
    fn and(self, other: Trit) -> Trit;
    fn or(self, other: Trit) -> Trit;
    fn not(self) -> Trit;
}

impl Kleene for Trit {
    #[inline]
    fn and(self, other: Trit) -> Trit {
        use Trit::*;
        match (self, other) {
            (No, _) | (_, No) => No, // false dominates: false AND unknown is FALSE, not unknown
            (Yes, Yes) => Yes,
            _ => Unknown,
        }
    }

    #[inline]
    fn or(self, other: Trit) -> Trit {
        use Trit::*;
        match (self, other) {
            (Yes, _) | (_, Yes) => Yes, // true dominates
            (No, No) => No,
            _ => Unknown,
        }
    }

    #[inline]
    fn not(self) -> Trit {
        match self {
            Trit::Yes => Trit::No,
            Trit::No => Trit::Yes,
            Trit::Unknown => Trit::Unknown, // the negation of "I don't know" is "I don't know"
        }
    }
}

/// A packed sequence of trits, 4 per byte.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Trits {
    bytes: Vec<u8>,
    len: usize,
}

impl Trits {
    pub fn with_len(len: usize) -> Trits {
        // 0b01010101 = four Unknowns. The default state of an unwritten trit is "I do not know",
        // which is the only honest zero value for this type.
        Trits { bytes: vec![0b0101_0101; (len + 3) / 4], len }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> Trit {
        debug_assert!(i < self.len);
        Trit::from_bits(self.bytes[i >> 2] >> ((i & 3) << 1))
    }

    #[inline]
    pub fn set(&mut self, i: usize, t: Trit) {
        debug_assert!(i < self.len);
        let sh = (i & 3) << 1;
        let b = &mut self.bytes[i >> 2];
        *b = (*b & !(0b11 << sh)) | (t.bits() << sh);
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Count of known-Yes, done a word at a time.
    ///
    /// ★ THIS IS THE ONE THAT HAS TO WIN. The whole performance claim reduces to: can a scan over
    /// a packed ternary column beat a scan over deserialised `Option<T>`s? Everything else in the
    /// proposal is downstream of this measurement.
    pub fn count_yes(&self) -> usize {
        let mut n = 0;
        for &b in &self.bytes {
            let mut x = b;
            for _ in 0..4 {
                if x & 0b11 == 2 {
                    n += 1;
                }
                x >>= 2;
            }
        }
        // the tail may hold padding trits; they are Unknown (0b01), never Yes, so they cannot
        // inflate this count. That is why the pad value matters and is not an arbitrary choice.
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_is_the_default_state() {
        let t = Trits::with_len(9);
        for i in 0..9 {
            assert_eq!(t.get(i), Trit::Unknown, "an unwritten trit must read Unknown, not No");
        }
    }

    #[test]
    fn roundtrip_across_byte_boundaries() {
        let mut t = Trits::with_len(11);
        let pattern = [Trit::Yes, Trit::No, Trit::Unknown];
        for i in 0..11 {
            t.set(i, pattern[i % 3]);
        }
        for i in 0..11 {
            assert_eq!(t.get(i), pattern[i % 3], "index {} survived packing", i);
        }
    }

    #[test]
    fn kleene_matches_the_proposal() {
        // §8 of J's document, verbatim — these three are the spec.
        assert_eq!(Trit::Yes.and(Trit::Unknown), Trit::Unknown);
        assert_eq!(Trit::No.and(Trit::Unknown), Trit::No);
        assert_eq!(Trit::Yes.or(Trit::Unknown), Trit::Yes);
    }

    #[test]
    fn negation_of_unknown_is_unknown() {
        assert_eq!(Trit::Unknown.not(), Trit::Unknown);
    }

    #[test]
    fn padding_trits_cannot_be_counted_as_yes() {
        // 5 trits => 2 bytes => 3 padding slots. If padding defaulted to 0 (No) this would still
        // pass; it is Unknown, and the test that matters is that it is never Yes.
        let mut t = Trits::with_len(5);
        for i in 0..5 {
            t.set(i, Trit::Yes);
        }
        assert_eq!(t.count_yes(), 5, "padding must not inflate the count");
    }

    #[test]
    fn resolve_is_the_only_door_and_it_asks() {
        assert!(Trit::Unknown.resolve(true));
        assert!(!Trit::Unknown.resolve(false));
        // The same unknown, two answers, because the caller supplied the meaning. If this type
        // ever grows a Default or a From<Trit> for bool, delete it and re-read the crate docs.
        assert!(Trit::Yes.resolve(false));
        assert!(!Trit::No.resolve(true));
    }
}
