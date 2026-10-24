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
#[cfg(feature = "server")]
pub mod imports;
pub mod library;
pub mod persistence;
mod playlist_rules;
pub mod product;
pub mod recommendation;
#[cfg(feature = "server")]
pub mod server;
pub mod settings;
pub mod startup;
