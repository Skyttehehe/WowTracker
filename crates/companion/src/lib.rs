//! WowTracker companion: reads the addon's SavedVariables file and sends it to
//! the backend. The binary (`main.rs`) is the watch loop; the pieces live here
//! so they can be tested.

pub mod config;
pub mod discover;
pub mod lua;
pub mod combatlog;
pub mod probe;
pub mod process;
pub mod savedvars;
pub mod upload;
