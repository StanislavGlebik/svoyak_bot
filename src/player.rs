use telegram_bot::UserId;

use std::cmp::{Eq, PartialEq};
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug)]
pub struct Player {
    name: String,
    id: UserId,
}

impl Player {
    pub fn new(name: String, id: UserId) -> Player {
        Player { name, id }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn id(&self) -> UserId {
        self.id
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
