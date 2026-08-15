//! The corpus, family by family.
//!
//! The split is not alphabetical and not arbitrary: it is `doc/adr/0053`'s own
//! classification. [`arithmetic`] holds the operators two conformant implementations
//! must agree on to the bit; [`transcendental`] holds the seven that ADR 0053 measured
//! *disagreeing* between our two adapters — `sin`, `cos`, `exp`, `sqrt`, `div` and
//! `atan` differ on 375 to 3 334 of 4 096 inputs — and which a program may therefore not
//! carry into a comparison without being refused. A reader who wants to know which half
//! of Table 42 is dangerous can read the module list.
//!
//! The other four families are the concerns that are not about arithmetic at all:
//! [`relational`]'s comparisons and the two operators wearing the name `not`,
//! [`conditional`]'s lowered branches, [`stack`]'s six shuffles, [`clipping`]'s two
//! normative clips, and [`refusal`]'s programs that must not be drawn.

pub mod arithmetic;
pub mod clipping;
pub mod conditional;
pub mod refusal;
pub mod relational;
pub mod stack;
pub mod transcendental;

use crate::case::Case;

/// One family of cases, named so that a failure says which part of Table 42 moved.
#[derive(Debug, Clone, Copy)]
pub struct Family {
    /// The family's name, as the module is called.
    pub name: &'static str,
    /// Its cases.
    pub cases: &'static [Case],
}

/// Every family, which is every case.
pub const FAMILIES: &[Family] = &[
    Family {
        name: "arithmetic",
        cases: arithmetic::CASES,
    },
    Family {
        name: "transcendental",
        cases: transcendental::CASES,
    },
    Family {
        name: "relational",
        cases: relational::CASES,
    },
    Family {
        name: "conditional",
        cases: conditional::CASES,
    },
    Family {
        name: "stack",
        cases: stack::CASES,
    },
    Family {
        name: "clipping",
        cases: clipping::CASES,
    },
    Family {
        name: "refusal",
        cases: refusal::CASES,
    },
];

/// Every case in the corpus, flat, in family order.
pub fn cases() -> impl Iterator<Item = &'static Case> {
    FAMILIES.iter().flat_map(|family| family.cases.iter())
}
