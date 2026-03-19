// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use std::collections::HashMap;

#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 2]
pub struct Config {
    pub username: String,
    pub pin: String,
    pub gym_uuid: String,
    pub gym_name: String,
    /// Notes keyed by "{class_name}_{day_of_week}_{HH:MM}" for recurring class persistence.
    pub class_notes: HashMap<String, String>,
}

impl Config {
    pub fn has_credentials(&self) -> bool {
        !self.username.is_empty() && !self.pin.is_empty()
    }
}
