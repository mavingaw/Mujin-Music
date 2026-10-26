#![recursion_limit = "256"]

#[cfg(feature = "server")]
pub mod api;
pub mod app;
#[cfg(feature = "server")]
pub mod assets;
#[cfg(feature = "server")]
pub mod discovery;
pub mod domain;
#[cfg(feature = "server")]
pub mod http;
pub mod library;
#[cfg(feature = "server")]
pub mod native_output;
pub mod persistence;
mod playlist_rules;
pub mod product;
pub mod radio_bans;
pub mod recommendation;
#[cfg(feature = "server")]
pub mod server;
pub mod settings;
pub mod signal_path;
pub mod startup;
