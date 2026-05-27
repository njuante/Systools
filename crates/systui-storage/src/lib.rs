//! SysTUI local storage: persistence of configuration, host profiles, cache and
//! the local audit log under the platform config/data/cache directories
//! (`Product.md` §11).
//!
//! Config loading lands here in phase 0 (S0.5); the audit log in phase 2 (v0.2).

pub mod audit;
pub mod config;
pub mod paths;
pub mod store;

pub use audit::AuditLog;
pub use config::{
    load as load_config, load_from as load_config_from, remove_host, remove_host_from,
    save_general_theme, save_general_theme_to, save_host, save_host_to,
};
pub use store::{HealthSnapshot, PersistentState, SavedSearch, SessionNote, StateStore};
