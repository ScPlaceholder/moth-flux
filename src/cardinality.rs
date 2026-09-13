//! Cardinality-first modelling — the compiler refuses two states in one slot.
//!
//! # The finding this module is built on (measured 2026-09-13 01:20, `event.rs`)
//!
//! `outcome` looked like a trit, was actually FIVE states, and is really TWO DIMENSIONS
//! (did-anything-run x what-it-said). The bug that proved it: `no_match` was parked in a slot
//! chosen because it was FREE (`ran=Unknown`), not because it was TRUE (`ran=Yes`), and a correct
//! query read the wrong encoding and returned 329 where the answer was 348. Only the
//! cross-representation assertion caught it.
//!
//! So the discipline is: **a field's state count is not "how many values does it have" but "how
//! many DISTINCT things can the world tell me"** — and once you have enumerated them, the encoding
//! must give each one its own slot, checked by the compiler, not by convention.
//!
//! # What is compile-checkable and what is not — stated honestly
//!
//! Two different failures hide here, and this module only closes the first:
//!
//! 1. **Two states sharing a slot** (the merge: `absent` folded into `cannot_tell`). This IS
//!    compile-checkable: [`states!`] uses Rust's own duplicate-discriminant error, and
//!    [`injective`] const-asserts that no two states map to the same trit tuple. Both refuse at
//!    compile time, with no runtime cost.
//! 2. **A state in the WRONG slot** (the NoMatch bug: injective encoding, wrong semantics). No
//!    type system catches this, because the compiler cannot know what `ran` MEANS. The defence
//!    stays what it was the night it worked: every benchmark and every conversion asserts all
//!    representations return the same answer. This module does not replace that and must never
//!    be described as if it does.

/// Declare a state set as an enum where every state gets an explicit, distinct slot.
///
/// The duplicate check is not this macro being clever — it is rustc's own E0081. The macro's
/// contribution is refusing to let you OMIT the discriminant, so "I didn't think about the slot"
/// is not an expressible program.
///
/// ```
/// moth_flux::states! {
///     /// The five distinct things the record can say about an outcome.
///     pub enum Outcome5 {
///         NothingRan = 0,
///         Ok = 1,
///         Fail = 2,
///         CannotTell = 3,
///         NoMatch = 4,
///     }
/// }
/// assert_eq!(Outcome5::COUNT, 5);
/// assert_eq!(Outcome5::from_slot(3), Some(Outcome5::CannotTell));
/// assert_eq!(Outcome5::from_slot(9), None);
/// ```
///
/// Two states in one slot is a compile error, not a lint:
///
/// ```compile_fail
/// moth_flux::states! {
///     pub enum Merged {
///         Absent = 2,
///         CannotTell = 2, // E0081: discriminant value `2` assigned more than once
///     }
/// }
/// ```
#[macro_export]
macro_rules! states {
    ($(#[$meta:meta])* $vis:vis enum $name:ident {
        $($(#[$smeta:meta])* $state:ident = $slot:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
        #[repr(u8)]
        $vis enum $name {
            $($(#[$smeta])* $state = $slot),+
        }

        impl $name {
            /// How many distinct things the world can say in this field.
            pub const COUNT: usize = [$(Self::$state),+].len();
            /// Every state, for exhaustive tests and cross-representation sweeps.
            pub const ALL: [Self; Self::COUNT] = [$(Self::$state),+];

            /// Total decode: every u8 either names exactly one state or is refused with `None`.
            /// No state can be silently absorbed by a neighbour's slot.
            pub fn from_slot(b: u8) -> Option<Self> {
                match b {
                    $($slot => Some(Self::$state),)+
                    _ => None,
                }
            }

            pub fn slot(self) -> u8 {
                self as u8
            }
        }
    };
}

/// Const injectivity check for a trit-tuple encoding: no two states may share a tuple.
///
/// Usable in `const _: () = assert!(...)` position, so a shared slot is a COMPILE error at the
/// definition site of the encoding table — the failure surfaces where the table is written, not
/// three queries later as a wrong count.
///
/// ```
/// use moth_flux::cardinality::injective;
/// // The real Outcome encoding from event.rs — five states, five distinct tuples.
/// const ENC: [[u8; 3]; 5] = [[0, 1, 1], [2, 2, 2], [2, 0, 2], [2, 1, 2], [2, 1, 0]];
/// const _: () = assert!(injective(&ENC));
/// ```
///
/// And the merge that caused the original five-state finding is refused at compile time:
///
/// ```compile_fail
/// use moth_flux::cardinality::injective;
/// // absent and cannot_tell both encoded as (ran=Unknown, ...): one slot, two meanings.
/// const ENC: [[u8; 3]; 2] = [[1, 1, 1], [1, 1, 1]];
/// const _: () = assert!(injective(&ENC));
/// ```
pub const fn injective(codes: &[[u8; 3]]) -> bool {
    let mut i = 0;
    while i < codes.len() {
        let mut j = i + 1;
        while j < codes.len() {
            if codes[i][0] == codes[j][0]
                && codes[i][1] == codes[j][1]
                && codes[i][2] == codes[j][2]
            {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// Capacity arithmetic for a multi-trit encoding, stated so the "spare capacity" trap is visible
/// in numbers: k trits offer 3^k slots, and the NUMBER of free slots is exactly how much room
/// there is to park a state somewhere meaningless. 2 trits held 5 states with 4 slots spare —
/// and one of those spare slots is where the NoMatch bug lived.
pub const fn slots(trits: u32) -> usize {
    3usize.pow(trits)
}

/// The minimum trits needed for `states` distinct states — ceil(log3(states)).
pub const fn trits_needed(states: usize) -> u32 {
    let mut k = 0u32;
    let mut cap = 1usize;
    while cap < states {
        cap *= 3;
        k += 1;
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    states! {
        enum Demo {
            A = 0,
            B = 1,
            C = 2,
        }
    }

    #[test]
    fn decode_is_total_and_injective() {
        for s in Demo::ALL {
            assert_eq!(Demo::from_slot(s.slot()), Some(s), "{:?} must round-trip", s);
        }
        assert_eq!(Demo::from_slot(3), None, "an unnamed slot must be refused, not absorbed");
        assert_eq!(Demo::COUNT, 3);
    }

    #[test]
    fn injectivity_is_a_real_check_not_a_constant_verdict() {
        // A checker that says true for everything is worse than none — verify BOTH branches fire.
        assert!(injective(&[[0, 0, 0], [0, 0, 1]]));
        assert!(!injective(&[[0, 0, 0], [0, 0, 0]]));
        assert!(!injective(&[[1, 2, 0], [0, 1, 1], [1, 2, 0]]), "non-adjacent dup must be seen");
    }

    #[test]
    fn capacity_arithmetic() {
        assert_eq!(slots(1), 3);
        assert_eq!(slots(2), 9);
        assert_eq!(slots(3), 27);
        assert_eq!(trits_needed(3), 1);
        assert_eq!(trits_needed(4), 2); // 4 states do NOT fit a trit; the compiler should say so
        assert_eq!(trits_needed(5), 2); // 5 states in 9 slots: 4 spare = 4 places for a bug
        assert_eq!(trits_needed(10), 3);
        assert_eq!(trits_needed(1), 0);
    }
}
