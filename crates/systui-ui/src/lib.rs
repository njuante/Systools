//! SysTUI terminal UI: the ratatui/crossterm event loop, global application state,
//! tab navigation, theme, key bindings and the shared UI states (loading, empty,
//! error, permission-denied, ...). The UI only requests actions; it never executes.
//!
//! Implemented in phase 0, session S0.6.
