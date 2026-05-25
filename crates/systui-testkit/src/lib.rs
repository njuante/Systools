//! SysTUI test kit: shared fixtures, golden-file helpers and mock builders used by
//! unit and integration tests across the workspace.
//!
//! Per `Product.md` §12, every command-output parser must be covered by fixtures.
//! [`fuzz`] complements those fixtures with property-based strategies that throw
//! adversarial input at the parsers so they degrade gracefully instead of panicking
//! (the hardening goal of phase 10, S10.4).

pub mod fuzz;
