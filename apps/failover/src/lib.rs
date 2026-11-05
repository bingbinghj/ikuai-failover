#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod api;
pub mod appearance;
pub mod auth;
pub mod config;
pub mod discovery;
pub mod events;
mod ikuai_info;
pub mod ipv6;
pub mod journal;
pub mod logging;
pub mod manual_selection;
pub mod notification;
pub mod policy;
pub mod runtime;
pub mod settings;
pub mod state;
mod web;
