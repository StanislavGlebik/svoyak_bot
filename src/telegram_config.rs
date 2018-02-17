extern crate serde;
extern crate serde_json;

use std::fs::File;
use telegram_bot;

#[derive(Serialize, Deserialize)]
struct RawConfig {
    pub admin_id: i64,
    pub game_chat_id: i64
}

pub struct Config {
    pub admin_user : telegram_bot::UserId,  
    pub admin_chat : telegram_bot::ChatId,
    pub game_chat : telegram_bot::ChatId 
}

const DEFAULT_ADMIN_ID: i64 = 125732128;
const DEFAULT_GAME_CHAT_ID: i64 = -272387150;

impl RawConfig {
    fn new(filename: Option<String>) -> Self {
        match filename {
            Some(ref fname) => {
                eprintln!("Loading configuration from '{}'", fname);
                let file = File::open(fname).unwrap_or_else(
                    |_| panic!("Can't open file '{}' with configuration", fname)
                );
                let config: Self = serde_json::from_reader(file).unwrap_or_else(
                    |_| panic!("Content of '{}' is not a valid InstanceConfig object", fname)
                );
                config
            }
            None => { 
                eprintln!("Loading default configuration");
                Self {
                    admin_id: DEFAULT_ADMIN_ID,
                    game_chat_id: DEFAULT_GAME_CHAT_ID
                }
            }
        }
    }
}

impl Config {
    /// Read configuration from JSON-file or return
    /// the default one
    pub fn new(filename: Option<String>) -> Self {
        let config = RawConfig::new(filename);
        Config {
            admin_user: telegram_bot::UserId::from(config.admin_id),
            admin_chat: telegram_bot::ChatId::from(config.admin_id),
            game_chat: telegram_bot::ChatId::from(config.game_chat_id)
        }
    }
}
