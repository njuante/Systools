//! SysTUI transport layer: implementations of the [`systui_core::Transport`]
//! contract. Every collector and action runs through a transport, so local and
//! remote hosts are interchangeable.
//!
//! - [`LocalTransport`] runs against the current machine.
//! - [`MockTransport`] returns pre-programmed responses for tests.
//!
//! The SSH transport arrives in phase 5 (v0.5).

pub mod local;
pub mod mock;

pub use local::LocalTransport;
pub use mock::MockTransport;
