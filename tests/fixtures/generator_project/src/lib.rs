// SPDX-License-Identifier: AGPL-3.0-only
/// @mvs-feature("rust_surface")
/// @mvs-protocol("rust_api")
pub fn handshake(version: u32) -> bool {
    version > 0
}

pub struct HostAdapter;

impl HostAdapter {
    pub fn connect(&self, target: &str) -> bool {
        !target.is_empty()
    }
}
