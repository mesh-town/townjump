//! `pluginRequest` codec. JS: hyperdht/lib/messages.js:pluginRequest
use crate::compact_encoding::{
    self, State, decode_string, decode_uint, encode_string, encode_uint, preencode_string,
    preencode_uint,
};
use crate::hyperdht_messages::Result;

/// Body of a PLUGIN (command 10) request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRequest {
    /// Plugin name.
    pub plugin: String,
    /// Plugin protocol version.
    pub version: u64,
    /// Plugin-specific command.
    pub command: u64,
    /// Optional payload; `Some(empty)` is distinct from `None` (flag bit 1 is set).
    pub value: Option<Vec<u8>>,
}

/// Serializes a [`PluginRequest`]: string, uint version, uint command, uint flags, value if flag 1.
pub fn encode_plugin_request(m: &PluginRequest) -> Vec<u8> {
    let mut state = State::new();
    preencode_string(&mut state, &m.plugin);
    preencode_uint(&mut state, m.version);
    preencode_uint(&mut state, m.command);
    preencode_uint(&mut state, 0);
    if let Some(v) = &m.value {
        compact_encoding::preencode_buffer(&mut state, Some(v));
    }
    state.alloc();
    encode_string(&mut state, &m.plugin);
    encode_uint(&mut state, m.version);
    encode_uint(&mut state, m.command);
    encode_uint(&mut state, u64::from(m.value.is_some()));
    if let Some(v) = &m.value {
        compact_encoding::encode_buffer(&mut state, Some(v));
    }
    state.buffer
}

/// Decodes a [`PluginRequest`]; strict like compact-encoding 3.x (`Out of bounds` on truncation).
pub fn decode_plugin_request(bytes: &[u8]) -> Result<PluginRequest> {
    let mut state = State::from_buffer(bytes);
    let plugin = decode_string(&mut state)?;
    let version = decode_uint(&mut state)?;
    let command = decode_uint(&mut state)?;
    let flags = decode_uint(&mut state)?;
    let value = if flags & 1 != 0 {
        compact_encoding::decode_buffer(&mut state)?
    } else {
        None
    };
    Ok(PluginRequest {
        plugin,
        version,
        command,
        value,
    })
}

#[cfg(test)]
#[path = "hyperdht_plugin_tests.rs"]
mod tests;
