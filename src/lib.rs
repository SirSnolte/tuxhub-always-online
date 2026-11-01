//! Keeps Discord, Steam and other apps from setting you to away while you are
//! at the computer, by sending a harmless input event at a fixed interval.

pub mod client;
pub mod config;
pub mod daemon;
pub mod engine;
pub mod impulse;
pub mod integration;
pub mod live;
pub mod paths;
pub mod schedule;
pub mod service;
