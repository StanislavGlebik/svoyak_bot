use serde_derive::{Deserialize, Serialize};
use std::fs::File;
use telegram_bot;

#[derive(Clone, Serialize, Deserialize)]
pub struct Question {
    topic: String,
    cost: usize,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CatInBag {
    pub old_topic: String,
    pub cost: usize,
    pub new_topic: String,
    pub question: String,
    pub answer: String,
    pub comments: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct RawConfig {
    pub admin_id: i64,
    pub game_chat_id: Option<i64>,
    pub questions_storage_path: String,
    pub questions_per_topic: usize,
    pub manual_questions: Option<Vec<Question>>,
    pub cats_in_bags: Option<Vec<CatInBag>>,
}

pub struct Config {
    pub token: String,
    pub admin_user: telegram_bot::UserId,
    pub admin_chat: telegram_bot::ChatId,
    pub game_chat: Option<telegram_bot::ChatId>,
    pub questions_storage_path: String,
    pub questions_per_topic: usize,
    pub manual_questions: Vec<(String, usize)>,
    pub cats_in_bags: Vec<CatInBag>,
}

const DEFAULT_ADMIN_ID: i64 = 125732128;

impl RawConfig {
    fn new(filename: Option<String>) -> Self {
        match filename {
            Some(ref fname) => {
                eprintln!("Loading configuration from '{}'", fname);
                let file = File::open(fname)
                    .unwrap_or_else(|_| panic!("Can't open file '{}' with configuration", fname));
                let config: Self = serde_json::from_reader(file).unwrap_or_else(|_| {
                    panic!(
                        "Content of '{}' is not a valid InstanceConfig object",
                        fname
                    )
                });
                config
            }
            None => {
                eprintln!("Loading default configuration");
                Self {
                    admin_id: DEFAULT_ADMIN_ID,
                    game_chat_id: None,
                    questions_storage_path: "storage.csv".into(),
                    questions_per_topic: 5,
                    manual_questions: None,
                    cats_in_bags: None,
                }
            }
        }
    }
}

impl Config {
    /// Read configuration from JSON-file or return
    /// the default one
    pub fn new(filename: Option<String>, token: String) -> Self {
        let config = RawConfig::new(filename);
        Config {
            token,
            admin_user: telegram_bot::UserId::from(config.admin_id),
            admin_chat: telegram_bot::ChatId::from(config.admin_id),
            game_chat: config.game_chat_id.map(telegram_bot::ChatId::from),
            questions_storage_path: config.questions_storage_path,
            questions_per_topic: config.questions_per_topic,
            manual_questions: config
                .manual_questions
                .unwrap_or(vec![])
                .into_iter()
                .map(|question| (question.topic, question.cost))
                .collect(),
            cats_in_bags: config.cats_in_bags.unwrap_or(vec![]),
        }
    }
}
