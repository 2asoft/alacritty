use serde::{Deserialize, Deserializer, Serialize, de};
use toml::Value;

use alacritty_config_derive::{ConfigDeserialize, SerdeReplace};
use alacritty_terminal::term::{GraphicsConfig, Osc52};

use crate::config::ui_config::{Program, StringVisitor};

#[derive(ConfigDeserialize, Serialize, Default, Clone, Debug, PartialEq)]
pub struct Terminal {
    /// OSC52 support mode.
    pub osc52: SerdeOsc52,
    /// Path to a shell program to run on startup.
    pub shell: Option<Program>,
    /// Kitty graphics protocol options.
    pub graphics: Graphics,
}

#[derive(ConfigDeserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Graphics {
    pub enabled: bool,
    pub storage_limit: usize,
    pub local_transmission: bool,
}

impl Default for Graphics {
    fn default() -> Self {
        Self { enabled: false, storage_limit: 320_000_000, local_transmission: true }
    }
}

impl From<&Graphics> for GraphicsConfig {
    fn from(graphics: &Graphics) -> Self {
        Self {
            enabled: graphics.enabled,
            storage_limit: graphics.storage_limit,
            local_transmission: graphics.local_transmission,
        }
    }
}

#[derive(SerdeReplace, Serialize, Default, Copy, Clone, Debug, PartialEq)]
pub struct SerdeOsc52(pub Osc52);

impl<'de> Deserialize<'de> for SerdeOsc52 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserializer.deserialize_str(StringVisitor)?;
        Osc52::deserialize(Value::String(value)).map(SerdeOsc52).map_err(de::Error::custom)
    }
}
