//! Types, traits and constants to abstract over the different
//! implementations of the firebird client

mod connection;
#[cfg(feature = "date_time")]
mod date_time;
pub(crate) mod error;
pub mod ibase;
mod params;
mod row;

pub use connection::*;
pub use error::FbError;
pub use params::*;
pub use row::*;
