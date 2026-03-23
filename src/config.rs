// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Default, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct PlusOneConfig {
    pub name: String,
    pub username: String,
    pub pin: String,
}

#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 2]
pub struct Config {
    pub username: String,
    pub pin: String,
    pub gym_uuid: String,
    pub gym_name: String,
    /// Notes keyed by "{class_name}_{day_of_week}_{HH:MM}" for recurring class persistence.
    pub class_notes: HashMap<String, String>,
    /// Family members / friends who also have Gym Group accounts.
    pub plus_ones: Vec<PlusOneConfig>,
}

impl Config {
    pub fn has_credentials(&self) -> bool {
        !self.username.is_empty() && !self.pin.is_empty()
    }
}
