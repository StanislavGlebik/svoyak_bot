use telegram_bot::UserId;

use std::cmp::{Eq, PartialEq};
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug)]
pub struct Player {
    name: String,
    id: UserId,
    username: Option<String>,
}

impl Player {
    pub fn new(name: String, id: UserId, username: Option<String>) -> Player {
        Player { name, id, username }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn id(&self) -> UserId {
        self.id
    }

    pub fn username(&self) -> &Option<String> {
        &self.username
    }
}

impl PartialEq for Player {
    fn eq(&self, other: &Player) -> bool {
        self.id == other.id
    }
}

impl Eq for Player {}

impl Hash for Player {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
