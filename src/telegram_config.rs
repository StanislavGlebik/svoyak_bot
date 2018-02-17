extern crate serde;
extern crate serde_json;


use std::fs::File;

#[derive(Serialize, Deserialize)]
pub struct InstanceConfig {
    pub admin_id: i64,
    pub game_chat_id: i64
}

const DEFAULT_ADMIN_ID: i64 = 125732128;
const DEFAULT_GAME_CHAT_ID: i64 = -272387150;

impl InstanceConfig {
    /// Read configuration from JSON-file or return
    /// the default one
    pub fn new(filename: Option<String>) -> Self {
        match filename {
            Some(ref fname) => {
                let file = File::open(fname).unwrap_or_else(
                    |_| panic!("Can't open file '{}' with configuration", fname)
                );
                let config: InstanceConfig = serde_json::from_reader(file).unwrap_or_else(
                    |_| panic!("Content of '{}' is not a valid InstanceConfig object", fname)
                );
                config
            }
            None => Self {
                admin_id: DEFAULT_ADMIN_ID,
                game_chat_id: DEFAULT_GAME_CHAT_ID
            }
        }
    }
}

