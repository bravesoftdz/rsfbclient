//! Firebird client implementation in pure rust

mod arc4;
mod blr;
mod client;
mod consts;
mod srp;
mod wire;
mod xsqlda;

pub use client::{DbHandle, RustFbClient, StmtHandle, TrHandle};