//! SysTUI transport layer: implementations of the [`systui_core::Transport`]
//! contract. Every collector and action runs through a transport, so local and
//! remote hosts are interchangeable.
//!
//! - [`LocalTransport`] runs against the current machine.
//! - [`SshTransport`] runs against a remote host via the system OpenSSH client.
//! - [`MockTransport`] returns pre-programmed responses for tests.

pub mod local;
pub mod mock;
pub mod ssh;

pub use local::LocalTransport;
pub use mock::MockTransport;
pub use ssh::{SshTransport, build_remote_command};
