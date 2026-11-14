//! Friends: a mutual relationship between two endpoint ids, agreed directly
//! between the two peers on an ALPN of its own, with no server keeping a list.
//!
//! - `wire`: the messages.
//! - `state`: the state machine both peers run.
//! - `store`: the list on disk.

pub mod state;
pub mod store;
pub mod wire;
