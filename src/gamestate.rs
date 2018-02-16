use std::collections::HashMap;
use std::time::Duration;

use telegram_bot::UserId;

use messages::*;
use player::Player;
use question::Question;

#[derive(Clone, Debug, Eq, PartialEq)]
enum State {
    WaitingForPlayersToJoin,
    WaitingForQuestion,
    Falsestart(Question, i64),
    CanAnswer(Question, i64),
    Answering(Question, i64),
    Pause,
}

pub struct GameState {
    admin_user: UserId,
    state: State,
    players: HashMap<Player, i64>,
    current_player: Option<Player>,
    questions: HashMap<String, Vec<usize>>,
    question_storage: HashMap<(String, usize), Question>,
}

pub enum UiRequest {
    SendTextToMainChat(String),
    Timeout(Duration),
    ChooseQuestion(String, HashMap<String, Vec<usize>>),
    AskAdminYesNo(String),
    SendToAdmin(String),
    StopTimer,
}

impl GameState {
    pub fn new(admin_user: UserId) -> Self {
        let mut questions = HashMap::new();
        questions.insert(String::from("Sport"), vec![100, 200, 300, 400, 500]);
        questions.insert(String::from("Movies"), vec![100, 200, 300, 400, 500]);

        let mut question_storage = HashMap::new();
        question_storage.insert((String::from("Sport"), 100), Question::new("2 * 2 = ?", "4"));
        question_storage.insert((String::from("Sport"), 200), Question::new("3 * 2 = ?", "6"));
        question_storage.insert((String::from("Sport"), 300), Question::new("4 * 2 = ?", "8"));
        question_storage.insert((String::from("Sport"), 400), Question::new("5 * 2 = ?", "10"));
        question_storage.insert((String::from("Sport"), 500), Question::new("6 * 2 = ?", "12"));

        question_storage.insert((String::from("Movies"), 100), Question::new("2 * 2 = ?", "4"));
        question_storage.insert((String::from("Movies"), 200), Question::new("3 * 2 = ?", "6"));
        question_storage.insert((String::from("Movies"), 300), Question::new("4 * 2 = ?", "8"));
        question_storage.insert((String::from("Movies"), 400), Question::new("5 * 2 = ?", "10"));
        question_storage.insert((String::from("Movies"), 500), Question::new("6 * 2 = ?", "12"));

        Self {
            admin_user,
            state: State::WaitingForPlayersToJoin,
            players: HashMap::new(),
            current_player: None,
            questions,
            question_storage,
        }
    }

    pub fn add_player(&mut self, new_user: UserId, name: String) -> Vec<UiRequest> {
        if self.state != State::WaitingForPlayersToJoin {
            println!("{} tried to join, but the game has already started", name);
            return vec![];
        }

        if !self.find_player(new_user).is_none() {
            vec![
                UiRequest::SendTextToMainChat(String::from(
                    "Такой игрок уже существует",
                )),
            ]
        } else if !self.find_player_by_name(&name).is_none() {
            vec![
                UiRequest::SendTextToMainChat(String::from(
                    "Игрок с таким именем уже существует",
                )),
            ]
        } else {
            self.players.insert(Player::new(name.clone(), new_user), 0);
            vec![
                UiRequest::SendTextToMainChat(format!("Привет {}", name)),
            ]
        }
    }

    pub fn start(&mut self, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            println!("non admin user attempted to start a game");
            return vec![];
        }

        self.current_player = self.players.keys().next().cloned();
        let current_player_name = match self.current_player {
            Some(ref player) => {
                player.name()
            }
            None => {
                return vec![
                    UiRequest::SendTextToMainChat(String::from("Ни одного игрока не зарегистрировалось!")),
                ];
            }
        };

        if self.state != State::WaitingForPlayersToJoin {
            println!("attempt to start the game twice");
            vec![]
        } else {
            self.state = State::Pause;
            vec![
                UiRequest::SendTextToMainChat(format!("Игру начинает {}", current_player_name)),
            ]
        }
    }

    pub fn message(&mut self, user: UserId, _message: String) -> Vec<UiRequest> {
        println!("{} {}", user, _message);
        if let State::CanAnswer(question, cost) = self.state.clone() {
            let player = self.find_player(user).cloned();
            match player {
                Some(player) => {
                    self.current_player = Some(player.clone());
                    self.state = State::Answering(question, cost);
                    vec![
                        UiRequest::StopTimer,
                        UiRequest::SendTextToMainChat(format!("Отвечает {}", player.name())),
                        UiRequest::AskAdminYesNo("Correct answer?".to_string()),
                    ]
                }
                None => {
                   vec![]
                }
            }
        } else {
            vec![]
        }
    }

    pub fn next_question(&mut self, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            println!("non-admin user tried to select next question");
            return vec![];
        }
        let current_player_name = match self.current_player {
            Some(ref player) => {
                player.name().clone()
            }
            None => {
                println!("internal error: no current player!");
                return vec![];
            }
        };

        self.state = State::WaitingForQuestion;
        vec![
            UiRequest::ChooseQuestion(current_player_name, self.questions.clone())
        ]
    }

    pub fn yes_reply(&mut self, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            println!("non-admin yes reply");
            return vec![];
        }
        if let State::Answering(_, cost) = self.state {
            match self.update_current_player_score(cost) {
                Ok(_) => {
                    self.state = State::WaitingForQuestion;
                    let current_player_name = match self.current_player {
                        Some(ref player) => {
                            player.name()
                        }
                        None => {
                            return vec![];
                        }
                    };
                    let msg = format!("{}\nИгру продолжает {}", CORRECT_ANSWER, current_player_name);
                    vec![
                        UiRequest::SendTextToMainChat(msg),
                    ]
                }
                Err(err_msg) => {
                    println!("{}", err_msg);
                    vec![]
                }
            }
        } else {
            println!("unexpected yes answer");
            vec![]
        }
    }

    pub fn no_reply(&mut self, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            println!("non-admin yes reply");
            return vec![];
        }

        if let State::Answering(question, cost) = self.state.clone() {
            match self.update_current_player_score(-cost) {
                Ok(_) => {
                    self.state = State::CanAnswer(question, cost);
                    vec![
                        UiRequest::SendTextToMainChat(INCORRECT_ANSWER.to_string()),
                        UiRequest::Timeout(Duration::new(3, 0)),
                    ]
                }
                Err(err_msg) => {
                    println!("{}", err_msg);
                    vec![]
                }
            }
        } else {
            println!("unexpected yes answer");
            vec![]
        }
    }

    pub fn timeout(&mut self) -> Vec<UiRequest> {
        if let State::Falsestart(question, cost) = self.state.clone() {
            self.state = State::CanAnswer(question.clone(), cost);
            return vec![
                UiRequest::SendTextToMainChat(String::from("!")),
                UiRequest::Timeout(Duration::new(3, 0)),
            ];
        };

        if let State::CanAnswer(question, _) = self.state.clone() {
            self.state = State::Pause;
            let current_player_name = match self.current_player {
                Some(ref player) => {
                    player.name()
                }
                None => {
                    return vec![]
                }
            };
            let msg = format!("Время вышло!\nПравильный ответ: {}\nСледующий вопрос выбирает {}", question.question(), current_player_name);
            vec![
                UiRequest::SendTextToMainChat(msg),
            ]
        } else {
            println!("unexpected timeout");
            vec![]
        }
    }

    pub fn select_question<T: ToString>(&mut self, topic: T, cost: usize, user: UserId) -> Vec<UiRequest> {
        if self.state != State::WaitingForQuestion {
            println!("unexpected question selection");
            return vec![];
        }

        if !self.is_current_player(user) {
            println!("only current player can select questions");
            return vec![];
        }

        let topic = topic.to_string();
        match self.questions.get_mut(&topic) {
            Some(costs) => {
                if costs.contains(&cost) {
                    costs.retain(|elem| elem != &cost);
                    match self.question_storage.get(&(topic.clone(), cost)) {
                        Some(question) => {
                            self.state = State::Falsestart(question.clone(), cost as i64);
                            let main_chat_message = format!("Играем тему {}, вопрос за {}\n{}", topic, cost, question.question());
                            vec![
                                UiRequest::SendToAdmin(format!("question: {}\nanswer: {}", question.question(), question.answer())),
                                UiRequest::SendTextToMainChat(main_chat_message),
                                UiRequest::Timeout(Duration::new(1, 0)),
                            ]
                        }
                        None => {
                            println!("internal error: question is not found");
                            vec![]
                        }
                    }
                } else {
                    println!("question was already used");
                    vec![]
                }
            }
            None => {
                println!("unknown topic");
                vec![]
            }
        }
    }

    pub fn get_score(&mut self, _user: UserId) -> Vec<UiRequest> {
        let mut res = String::new();
        for (player, score) in self.players.iter() {
            res += &format!("{}: {}\n", player.name(), score);
        }
        vec![
            UiRequest::SendTextToMainChat(format!("{}", res))
        ]
    }

    pub fn current_player(&mut self, _user: UserId) -> Vec<UiRequest> {
        let mut res = String::new();
        match self.current_player {
            Some(ref player) => {
                res += &player.name()
            }
            None => {
                res += "No current player!";
            }
        }

        vec![
            UiRequest::SendTextToMainChat(format!("{}", res))
        ]
    }

    fn find_player(&self, id: UserId) -> Option<&Player> {
        self.players.keys().find(|player| player.id() == id)
    }

    fn find_player_by_name(&mut self, name: &String) -> Option<&Player> {
        self.players.keys().find(|player| player.name() == name)
    }

    fn update_current_player_score(&mut self, cost: i64) -> Result<(), String> {
        match self.current_player {
            Some(ref player) => {
                let val = self.players.get_mut(player);
                match val {
                    Some(val) => {
                        *val += cost;
                        Ok(())
                    }
                    None => {
                        Err("current player is not in list of players".to_string())
                    }
                }
            }
            None => {
                Err("internal error: current player is None!".to_string())
            }
        }
    }

    fn is_current_player(&self, id: UserId) -> bool {
        match self.current_player {
            Some(ref p) => {
                p.id() == id
            }
            None => {
                false
            }
        }
    }

    // Test only methods
    fn get_players(&self) -> Vec<Player> {
        let mut v = vec![];
        for k in self.players.keys() {
            v.push(k.clone());
        }
        v
    }

    fn get_player_score(&self, id: UserId) -> Option<i64> {
        let player = self.players.keys().find(|player| player.id() == id);
        player.and_then(|player| self.players.get(player).cloned())
    }

    fn get_current_player(&self) -> Option<Player> {
        self.current_player.clone()
    }

    fn set_current_player(&mut self, id: UserId) -> Result<(), String> {
        let player = self.players.keys().find(|player| player.id() == id);
        let player = player.ok_or(String::from("does not exist"))?;
        self.current_player = Some(player).cloned();
        Ok(())
    }

    fn get_state(&self) -> &State {
        &self.state
    }
}


#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_add_player() {
        let mut game_state = GameState::new(UserId::from(1));
        game_state.add_player(UserId::from(1), String::from("new"));
        game_state.add_player(UserId::from(1), String::from("new"));
        assert_eq!(game_state.get_players().len(), 1);
    }

    #[test]
    fn test_start_game() {
        let mut game_state = GameState::new(UserId::from(1));
        assert_eq!(game_state.get_state(), &State::WaitingForPlayersToJoin);

        game_state.start(UserId::from(2));
        assert_eq!(game_state.get_state(), &State::WaitingForPlayersToJoin);

        game_state.start(UserId::from(1));
        assert_eq!(game_state.get_state(), &State::WaitingForPlayersToJoin);

        game_state.add_player(UserId::from(1), String::from("new"));
        game_state.start(UserId::from(1));
        assert_eq!(game_state.get_state(), &State::Pause);

        game_state.start(UserId::from(1));
        assert_eq!(game_state.get_state(), &State::Pause);
    }

    #[test]
    fn test_score_simple() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let p2 = UserId::from(3);
        let mut game_state = GameState::new(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.add_player(p2, String::from("new_2"));
        game_state.start(admin);

        assert_eq!(game_state.get_player_score(p1), Some(0));
        assert_eq!(game_state.get_player_score(p2), Some(0));
        game_state.set_current_player(p1).unwrap();

        game_state.next_question(admin);
        game_state.select_question("Sport", 100, p1);
        match game_state.get_state() {
            &State::Falsestart(_, _) => {

            }
            _ => {
                assert!(false);
            }
        }

        // Can click button
        game_state.timeout();
        game_state.message(p1, String::from("1"));
        game_state.yes_reply(admin);

        assert_eq!(game_state.get_player_score(p1), Some(100));
        assert_eq!(game_state.get_player_score(p2), Some(0));
        assert_eq!(game_state.get_current_player().map(|p| p.id()), Some(p1));

        game_state.next_question(admin);
        game_state.select_question("Sport", 100, p1);
        // Cannot select already selected question
        assert_eq!(game_state.get_state(), &State::WaitingForQuestion);

        game_state.select_question("Rock'n'roll", 100, p1);
        // Cannot select non-existing topic
        assert_eq!(game_state.get_state(), &State::WaitingForQuestion);

        game_state.select_question("Sport", 200, p2);
        // Only current player can select next question
        assert_eq!(game_state.get_state(), &State::WaitingForQuestion);
    }
}
