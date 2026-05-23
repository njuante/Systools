//! SysTUI transport layer: the `Transport` trait and its `Local`, `Ssh` and
//! `Mock` implementations. Every collector and action runs through a transport,
//! so local and remote hosts are interchangeable.
//!
//! Implemented in phase 0, session S0.4.
