use std::collections::HashMap;
use std::time::Duration;
use std::collections::HashSet;

use telegram_bot::UserId;

use failure::{Error, err_msg};
use messages::*;
use player::Player;
use telegram_config::TourDescription;
use question::Question;
use questionsstorage::QuestionsStorage;

#[derive(Clone, Debug, Eq, PartialEq)]
enum State {
    WaitingForPlayersToJoin,
    WaitingForTopic,
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
    questions_storage: Box<QuestionsStorage>,
    players_falsestarted: HashSet<Player>,
    players_answered_current_question: HashSet<Player>,
    questions_per_topic: usize,
    tours: Vec<TourDescription>,
    current_tour: usize,
    current_multiplier: usize,
}

pub enum UiRequest {
    SendTextToMainChat(String),
    SendTextToMainChatWithDelay(String, Duration),
    Timeout(Option<String>, Duration),
    ChooseTopic(String, Vec<String>),
    ChooseQuestion(String, Vec<usize>),
    AskAdminYesNo(String),
    SendToAdmin(String),
    SendScoreTable(ScoreTable),
    StopTimer,
}

#[derive(Serialize)]
struct ScoreTableItem {
    name: String,
    questions: Vec<usize>
}

#[derive(Serialize)]
pub struct ScoreTable {
    scores: Vec<usize>,
    data: Vec<ScoreTableItem>
}

impl GameState {
    pub fn new(
            admin_user: UserId,
            questions_storage: Box<QuestionsStorage>,
            questions_per_topic: usize,
            tours: Vec<TourDescription>,
    ) -> Result<Self, Error> {
        if questions_per_topic == 0 {
            return Err(err_msg(String::from("questions per topic can't be zero")));
        }
        for tour in tours.iter() {
            for topic in tour.topics.iter() {
                for i in 0..questions_per_topic {
                  let question_num = i + 1;
                  let topic_name = &topic.name;
                  if questions_storage.get(topic_name.clone(), i+1).is_none() {
                    return Err(err_msg(format!("{} is not found in {}", topic_name, question_num)));
                  }
                }
            }
        }

        Ok(Self {
            admin_user,
            state: State::WaitingForPlayersToJoin,
            players: HashMap::new(),
            current_player: None,
            questions: HashMap::new(),
            questions_storage,
            players_falsestarted: HashSet::new(),
            players_answered_current_question: HashSet::new(),
            questions_per_topic,
            tours,
            current_tour: 0,
            current_multiplier: 0,
        })
    }

    fn set_state(&mut self, state: State) {
        self.state = state;
        match self.state {
            State::WaitingForQuestion => {
                eprintln!("/question command was executed");
                self.players_falsestarted.clear();
                self.players_answered_current_question.clear();
            }
            State::Answering(_, _) => {
                eprintln!("Now waiting for player '{:?}' to answer", self.current_player.as_ref());
            }
            State::WaitingForPlayersToJoin => {
                eprintln!("Now waiting for players to join the game");
            }
            State::Falsestart(_, _) => {
                eprintln!("Now it would be a falsestart to answer the question");
            }
            State::CanAnswer(_, _) => {
                eprintln!("Now it is ok to answer the question");
            }
            State::Pause => {
                eprintln!("The game is paused");
            }
            State::WaitingForTopic => {
                eprintln!("Waiting for the choice of topic");
            }
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

        if self.state != State::WaitingForPlayersToJoin {
            println!("attempt to start the game twice");
            vec![]
        } else {
            self.current_player = self.players.keys().next().cloned();
            if self.current_player.is_none() {
                return vec![
                    UiRequest::SendTextToMainChat(String::from(
                        "Ни одного игрока не зарегистрировалось!",
                    )),
                ];
            }

            self.current_tour = 0;
            self.reload_available_questions();
            self.set_state(State::Pause);
            vec![
                UiRequest::SendTextToMainChat(
                    format!("Игру начинает {}", self.current_player.clone().unwrap().name())
                ),
            ]
        }
    }

    pub fn next_tour(&mut self, user: UserId) -> Vec<UiRequest> {
        eprintln!("User {} asking for the next tour", user);
        if user != self.admin_user {
            println!("non-admin user tried to select next question");
            return vec![];
        }

        if self.state != State::Pause && self.state != State::WaitingForTopic {
            println!("incorrect state to move to the next tour");
            return vec![];
        }

        self.current_tour += 1;
        self.reload_available_questions();
        vec![
            UiRequest::SendTextToMainChat("Переходим к следующему туру".to_string()),
        ]
    }

    pub fn message(&mut self, user: UserId, _message: String) -> Vec<UiRequest> {
        eprintln!("User {} sent a message '{}'", user, _message);
        if let State::Falsestart(_, _) = self.state.clone() {
            let player = self.find_player(user).cloned();
            match player {
                Some(player) => {
                    self.players_falsestarted.insert(player.clone());
                    return vec![
                        UiRequest::SendTextToMainChat(format!("Фальшстарт {}", player.name())),
                    ];
                }
                None => {
                    return vec![];
                }
            }
        }

        if let State::CanAnswer(question, cost) = self.state.clone() {
            let player = self.find_player(user).cloned();
            match player {
                Some(player) => {
                    if self.players_answered_current_question.contains(&player) {
                        eprintln!("Player '{:?}' already answered this question", player);
                        return vec![];
                    } else if self.players_falsestarted.contains(&player) {
                        eprintln!("Player {} falsestarted", player.name());
                        return vec![];
                    } else {
                        eprintln!("{:?}", self.players_answered_current_question);
                    }
                    self.current_player = Some(player.clone());
                    self.players_answered_current_question.insert(player.clone());
                    self.set_state(State::Answering(question, cost));
                    vec![
                        UiRequest::StopTimer,
                        UiRequest::SendTextToMainChat(format!("Отвечает {}", player.name())),
                        UiRequest::AskAdminYesNo("Correct answer?".to_string()),
                    ]
                }
                None => vec![],
            }
        } else {
            println!("bad state");
            vec![]
        }
    }

    fn make_score_table(&self) -> ScoreTable {
        let mut scores = Vec::new();
        for i in 1..self.questions_per_topic + 1 {
            scores.push(i * self.current_multiplier);
        }
        let mut data = Vec::new();
        for topic in self.tours[self.current_tour].topics.iter() {
            let topic_name = topic.name.clone();
            let question_scores = self.questions.get(&topic_name).unwrap().clone();


            data.push(ScoreTableItem{
                name: topic_name,
                questions: question_scores
            })
        }

        ScoreTable {
            scores,
            data
        }
    }


    pub fn next_question(&mut self, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            println!("non-admin user tried to select next question");
            return vec![];
        }
        let current_player_name = match self.current_player {
            Some(ref player) => player.name().clone(),
            None => {
                println!("internal error: no current player!");
                return vec![];
            }
        };

        self.set_state(State::WaitingForTopic);
        let topics: Vec<_> = self.questions.iter()
            .filter(|&(_, costs)| !costs.is_empty())
            .map(|(topic, _)| topic.clone()).collect();
        vec![
            UiRequest::SendScoreTable(self.make_score_table()),
            UiRequest::ChooseTopic(current_player_name, topics),
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
                    self.set_state(State::WaitingForTopic);
                    let current_player_name = match self.current_player {
                        Some(ref player) => player.name(),
                        None => {
                            return vec![];
                        }
                    };
                    let msg = format!(
                        "{}\nИгру продолжает {}",
                        CORRECT_ANSWER,
                        current_player_name
                    );
                    vec![UiRequest::SendTextToMainChat(msg)]
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
                    if self.players_answered_current_question.len() != self.players.len() {
                        self.set_state(State::CanAnswer(question, cost));
                        self.players_falsestarted.clear();
                        vec![
                            UiRequest::Timeout(Some(INCORRECT_ANSWER.to_string()), Duration::new(3, 0)),
                        ]
                    } else {
                        self.set_state(State::Pause);
                        vec![
                            UiRequest::SendTextToMainChat(INCORRECT_ANSWER.to_string()),
                            UiRequest::SendTextToMainChat(
                                format!(
                                    "Все игроки не смогли ответить на такой простой вопрос.\nПравильный ответ: '{}'",
                                    question.answer()
                                )
                            )
                        ]
                    }
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
        println!("timeout");
        if let State::Falsestart(question, cost) = self.state.clone() {
            println!("falsestart");
            self.set_state(State::CanAnswer(question.clone(), cost));
            return vec![
                UiRequest::Timeout(None, Duration::new(8, 0)),
            ];
        };

        if let State::CanAnswer(question, _) = self.state.clone() {
            self.set_state(State::Pause);
            let current_player_name = match self.current_player {
                Some(ref player) => player.name(),
                None => return vec![],
            };
            let msg = format!(
                "Время вышло!\nПравильный ответ: {}\nСледующий вопрос выбирает {}",
                question.answer(),
                current_player_name
            );
            vec![UiRequest::SendTextToMainChat(msg)]
        } else {
            println!("unexpected timeout");
            vec![]
        }
    }

    pub fn select_topic<T: ToString>(
        &mut self,
        topic: T,
        user: UserId,
    ) -> Vec<UiRequest> {
        // TODO(stas): make it possible to deselect the topic
        if self.state != State::WaitingForTopic {
            println!("unexpected topic selection");
            return vec![];
        }

        if !self.is_current_player(user) {
            println!("only current player can select questions");
            return vec![];
        }

        let topic = topic.to_string();
        match self.questions.clone().get(&topic) {
            Some(costs) => {
                if !costs.is_empty() {
                    self.set_state(State::WaitingForQuestion);
                    vec![
                        UiRequest::ChooseQuestion(topic.clone(), costs.clone())
                    ]
                } else {
                    vec![]
                }
            }
            None => {
                println!("unknown topic");
                return vec![];
            }
        }
    }

    pub fn select_question<T: ToString>(
        &mut self,
        topic: T,
        cost: usize,
        user: UserId,
    ) -> Vec<UiRequest> {
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
                    eprintln!("Question in topic '{}' and cost {} was selected", topic, cost);
                } else {
                    eprintln!("Question in topic '{}' and cost {} was already used!", topic, cost);
                    return vec![];
                }
            }
            None => {
                println!("unknown topic");
                return vec![];
            }
        }

        match self.questions_storage.get(topic.clone(), cost / self.current_multiplier) {
            Some(question) => {
                self.set_state(State::Falsestart(question.clone(), cost as i64));
                let main_chat_message = format!(
                    "Играем тему {}, вопрос за {}",
                    topic,
                    cost
                );
                let question_msg = format!("{}", question.question());
                let delay_before_question_secs = 5;
                let delay_falsestart_secs = delay_before_question_secs + 1;
                vec![
                    UiRequest::SendToAdmin(format!(
                        "question: {}\nanswer: {}",
                        question.question(),
                        question.answer()
                    )),
                    UiRequest::SendTextToMainChat(main_chat_message),
                    UiRequest::SendTextToMainChatWithDelay(
                        question_msg,
                        Duration::from_secs(delay_before_question_secs)
                    ),
                    UiRequest::Timeout(Some("!".to_string()), Duration::from_secs(delay_falsestart_secs)),
                ]
            }
            None => {
                println!("internal error: question is not found");
                vec![]
            }
        }
    }

    pub fn get_score(&mut self, _user: UserId) -> Vec<UiRequest> {
        let mut res = String::new();
        for (player, score) in self.players.iter() {
            res += &format!("{}: {}\n", player.name(), score);
        }
        vec![UiRequest::SendTextToMainChat(format!("{}", res))]
    }

    pub fn current_player(&mut self, _user: UserId) -> Vec<UiRequest> {
        let mut res = String::new();
        match self.current_player {
            Some(ref player) => res += &player.name(),
            None => {
                res += "No current player!";
            }
        }

        vec![UiRequest::SendTextToMainChat(format!("{}", res))]
    }


    fn reload_available_questions(&mut self) {
        self.questions.clear();
        match self.tours.get(self.current_tour) {
            Some(ref tour) => {
                self.current_multiplier = tour.multiplier;
                for topic in &tour.topics {
                    let mut costs = vec![];
                    for i in 0..self.questions_per_topic {
                        costs.push((i + 1) * self.current_multiplier);
                    }
                    self.questions.insert(topic.name.clone(), costs);
                }
            }
            None => {
                eprintln!("current tour is not available!");
            }
        }
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
                    None => Err("current player is not in list of players".to_string()),
                }
            }
            None => Err("internal error: current player is None!".to_string()),
        }
    }

    fn is_current_player(&self, id: UserId) -> bool {
        match self.current_player {
            Some(ref p) => p.id() == id,
            None => false,
        }
    }

    #[cfg(test)]
    fn get_players(&self) -> Vec<Player> {
        let mut v = vec![];
        for k in self.players.keys() {
            v.push(k.clone());
        }
        v
    }

    #[cfg(test)]
    fn get_player_score(&self, id: UserId) -> Option<i64> {
        let player = self.players.keys().find(|player| player.id() == id);
        player.and_then(|player| self.players.get(player).cloned())
    }

    #[cfg(test)]
    fn get_current_player(&self) -> Option<Player> {
        self.current_player.clone()
    }

    #[cfg(test)]
    fn set_current_player(&mut self, id: UserId) -> Result<(), String> {
        let player = self.players.keys().find(|player| player.id() == id);
        let player = player.ok_or(String::from("does not exist"))?;
        self.current_player = Some(player).cloned();
        Ok(())
    }

    #[cfg(test)]
    fn get_state(&self) -> &State {
        &self.state
    }
}


#[cfg(test)]
mod test {
    use super::*;
    use telegram_config::Topic;
    use questionsstorage::QuestionsStorage;

    pub struct FakeQuestionsStorage {
        questions: HashMap<(String, usize), Question>
    }

    impl FakeQuestionsStorage {
        pub fn new() -> Self {
            let mut question_storage = HashMap::new();
            question_storage.insert(
                (String::from("Sport"), 1),
                Question::new("2 * 2 = ?", "4"),
            );
            question_storage.insert(
                (String::from("Sport"), 2),
                Question::new("3 * 2 = ?", "6"),
            );
            question_storage.insert(
                (String::from("Sport"), 3),
                Question::new("4 * 2 = ?", "8"),
            );
            question_storage.insert(
                (String::from("Sport"), 4),
                Question::new("5 * 2 = ?", "10"),
            );
            question_storage.insert(
                (String::from("Sport"), 5),
                Question::new("6 * 2 = ?", "12"),
            );

            question_storage.insert(
                (String::from("Movies"), 1),
                Question::new("2 * 2 = ?", "4"),
            );
            question_storage.insert(
                (String::from("Movies"), 2),
                Question::new("3 * 2 = ?", "6"),
            );
            question_storage.insert(
                (String::from("Movies"), 3),
                Question::new("4 * 2 = ?", "8"),
            );
            question_storage.insert(
                (String::from("Movies"), 4),
                Question::new("5 * 2 = ?", "10"),
            );
            question_storage.insert(
                (String::from("Movies"), 5),
                Question::new("6 * 2 = ?", "12"),
            );

            Self {
                questions: question_storage
            }
        }
    }

    impl QuestionsStorage for FakeQuestionsStorage {
        fn get(&self, topic_name: String, difficulty: usize) -> Option<Question> {
            self.questions.get(&(topic_name, difficulty)).cloned()
        }
    }

    fn create_game_state(user: UserId) -> GameState {
        let questions_storage: Box<QuestionsStorage> = Box::new(FakeQuestionsStorage::new());
        let tours = vec![
            TourDescription {
                multiplier: 100,
                topics: vec![
                    Topic {
                        name: "Sport".to_string(),
                    }
                ]
            },
            TourDescription {
                multiplier: 200,
                topics: vec![
                    Topic {
                        name: "Movies".to_string(),
                    }
                ]
            },
        ];
        GameState::new(
            user,
            questions_storage,
            5,
            tours,
        ).unwrap()
    }

    #[test]
    fn test_add_player() {
        let mut game_state = create_game_state(UserId::from(1));
        game_state.add_player(UserId::from(1), String::from("new"));
        game_state.add_player(UserId::from(1), String::from("new"));
        assert_eq!(game_state.get_players().len(), 1);
    }

    #[test]
    fn test_start_game() {
        let mut game_state = create_game_state(UserId::from(1));
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
        let mut game_state = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.add_player(p2, String::from("new_2"));
        game_state.start(admin);
        match game_state.get_state() {
            &State::Pause => {}
            _ => {
                assert!(false);
            }
        }

        assert_eq!(game_state.get_player_score(p1), Some(0));
        assert_eq!(game_state.get_player_score(p2), Some(0));
        game_state.set_current_player(p1).unwrap();

        game_state.next_question(admin);
        game_state.select_topic("Sport", p1);
        match game_state.get_state() {
            &State::WaitingForQuestion => {}
            _ => {
                assert!(false);
            }
        }

        game_state.select_question("Sport", 100, p1);
        match game_state.get_state() {
            &State::Falsestart(_, _) => {}
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

        game_state.select_topic("Rock'n'roll", p1);
        // Cannot select non-existing topic
        assert_eq!(game_state.get_state(), &State::WaitingForTopic);

        game_state.select_topic("Sport", p1);
        game_state.select_question("Sport", 1, p1);
        // Cannot select already selected question
        assert_eq!(game_state.get_state(), &State::WaitingForQuestion);

        game_state.select_question("Sport", 200, p2);
        // Only current player can select next question
        assert_eq!(game_state.get_state(), &State::WaitingForQuestion);
    }

    #[test]
    fn test_game_state_creation() {
        let admin = UserId::from(1);
        let questions_storage: Box<QuestionsStorage> = Box::new(FakeQuestionsStorage::new());
        let tours = vec![
            TourDescription {
                multiplier: 100,
                topics: vec![
                    Topic {
                        name: "Nonexisting topic".to_string(),
                    }
                ]
            }
        ];

        // 0 question number
        assert!(GameState::new(
            admin,
            questions_storage,
            0,
            tours.clone(),
        ).is_err());

        // Non existing topic
        let questions_storage: Box<QuestionsStorage> = Box::new(FakeQuestionsStorage::new());
        assert!(GameState::new(
            admin,
            questions_storage,
            5,
            tours,
        ).is_err());


        // Incorrect question number
        let tours = vec![
            TourDescription {
                multiplier: 100,
                topics: vec![
                    Topic {
                        name: "Sport".to_string(),
                    }
                ]
            }
        ];

        let questions_storage: Box<QuestionsStorage> = Box::new(FakeQuestionsStorage::new());
        assert!(GameState::new(
            admin,
            questions_storage,
            6,
            tours,
        ).is_err());
    }

    #[test]
    fn test_tours_simple() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let mut game_state = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.start(admin);
        game_state.next_tour(admin);
        game_state.next_question(admin);

        game_state.select_topic("Movies", p1);
        game_state.select_question("Movies", 200, p1);

        game_state.timeout();
        game_state.message(p1, String::from("1"));
        game_state.yes_reply(admin);

        assert_eq!(game_state.get_player_score(p1), Some(200));
    }

    #[test]
    fn test_falsestarts_simple() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let mut game_state = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.start(admin);
        game_state.next_question(admin);

        game_state.select_topic("Sport", p1);
        game_state.select_question("Sport", 200, p1);
        game_state.message(p1, String::from("1"));
        game_state.timeout();
        game_state.message(p1, String::from("1"));
        match game_state.get_state() {
            &State::Answering(_, _) => {
                assert!(false);
            }
            _ => {
            }
        }
    }

    #[test]
    fn test_falsestarts_second_can_answer() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let p2 = UserId::from(3);
        let mut game_state = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.add_player(p2, String::from("new_2"));
        game_state.start(admin);
        game_state.next_question(admin);

        game_state.set_current_player(p1).unwrap();
        game_state.select_topic("Sport", p1);
        game_state.select_question("Sport", 100, p1);
        game_state.message(p1, String::from("1"));
        game_state.timeout();
        game_state.message(p2, String::from("1"));
        game_state.yes_reply(admin);

        assert_eq!(game_state.get_player_score(p1), Some(0));
        assert_eq!(game_state.get_player_score(p2), Some(100));
    }

    #[test]
    fn test_falsestarts_can_answer_after_no() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let p2 = UserId::from(3);
        let mut game_state = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.add_player(p2, String::from("new_2"));
        game_state.start(admin);
        game_state.next_question(admin);

        game_state.set_current_player(p1).unwrap();
        game_state.select_topic("Sport", p1);
        game_state.select_question("Sport", 100, p1);
        game_state.message(p1, String::from("1"));
        game_state.timeout();
        game_state.message(p2, String::from("1"));
        game_state.no_reply(admin);
        game_state.message(p1, String::from("1"));
        game_state.yes_reply(admin);

        assert_eq!(game_state.get_player_score(p1), Some(100));
        assert_eq!(game_state.get_player_score(p2), Some(-100));
    }
}
