// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};

#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 2]
pub struct Config {
    pub username: String,
    pub pin: String,
    pub gym_uuid: String,
    pub gym_name: String,
}

impl Config {
    pub fn has_credentials(&self) -> bool {
        !self.username.is_empty() && !self.pin.is_empty()
    }
}
