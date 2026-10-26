//! Bridge module for communicating with the Go wa-bridge subprocess.

pub mod process;
pub mod protocol;
#[cfg(test)]
mod protocol_fixtures;

pub use process::{default_data_dir, find_bridge_binary, BridgeConfig, BridgeProcess, BridgeStop};
pub use protocol::{
    BridgeCommand, BridgeEvent, BridgeImage, Chat, ChatPresenceState, ConnectionState, Contact,
    Message, MessageContent,
};
