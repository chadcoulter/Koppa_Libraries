//! Koppa — deferred relational tracking, Rust port.
//! Unresolved ratio pairs; compose without dividing; resolve on demand.

pub mod inference;
pub mod ml;
pub mod finance;
pub mod predicates;
pub mod scan;

// Re-export the primary entry-point types.
pub use inference::{SP, ProjPair};
pub use ml::LazyRelational;
pub use finance::DeferredChain;
pub use predicates::{Ratio, compare};
pub use scan::{Pair, pair_from_ints, scan_double_buffer};
