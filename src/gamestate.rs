use std::collections::HashMap;
use std::collections::HashSet;

use telegram_bot::UserId;

use failure::{Error, err_msg};
use messages::*;
use player::Player;
use telegram_config::TourDescription;
use question::Question;
use questionsstorage::QuestionsStorage;
use std;

/// The state, when players are bidding for the auction
/// and winner is no decided yet. In this state players can pass
///
/// current_player is the player, who has highest bid so far
/// so if all other players passed, (s)he will play
#[derive(Clone, Debug, Eq, PartialEq)]
struct BiddingState {
    question: Question,
    current_player: Player,
    bid: u64,
    passed: HashSet<Player>
}

/// In this state auction is won by player, but the final
/// bid is no decided yet. Here player can't pass
#[derive(Clone, Debug, Eq, PartialEq)]
struct FinishingBidState {
    question: Question,
    player: Player,
    bid: u64
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum State {
    WaitingForPlayersToJoin,
    WaitingForTopic,
    WaitingForQuestion,
    BeforeQuestionAsked(Question, i64),
    Falsestart(Question, i64),
    CanAnswer(Question, i64),
    Answering(Question, i64),
    Bidding(BiddingState),
    FinishingBid(FinishingBidState),
    Pause,
}

pub struct GameState {
    admin_user: UserId,
    state: State,
    players: HashMap<Player, i64>,
    current_player: Option<Player>,
    player_which_chose_question: Option<Player>,
    questions: HashMap<String, Vec<usize>>,
    questions_storage: Box<QuestionsStorage>,
    players_falsestarted: HashSet<Player>,
    players_answered_current_question: HashSet<Player>,
    questions_per_topic: usize,
    tours: Vec<TourDescription>,
    current_tour: usize,
    current_multiplier: usize,
    manual_questions: Vec<(String, usize)>,
}

pub enum UiRequest {
    SendTextToMainChat(String),
    Timeout(Option<String>, Delay),
    ChooseTopic(String, Vec<String>),
    ChooseQuestion(String, Vec<usize>),
    AskAdminYesNo(String),
    SendToAdmin(String),
    SendScoreTable(ScoreTable),
    StopTimer,
}

pub enum Delay {
    Short,
    Medium,
    Long
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

impl ScoreTable {
    pub fn to_string(&self) -> String {
        let mut rows : Vec<String> = Vec::new();

        let mut topic_length : usize = 0;
        for ref item in self.data.iter() {
            let this_length = item.name.chars().count();
            if this_length > topic_length {
                topic_length = this_length;
            }
        }

        for ref item in self.data.iter() {
            let mut row = String::from("|");
            row.push_str(&item.name);
            while row.chars().count() < topic_length + 1 {
                row.push_str(" ");
            }
            row.push_str("|");

            for score in self.scores.iter() {
                let mut found = false;
                for this_score in item.questions.iter() {
                    if this_score == score {
                        found = true;
                        break;
                    }
                }
                if found {
                    row.push_str("x");
                } else {
                    row.push_str(" ");
                }
                row.push_str("|");
            }

            rows.push(row);
        }

        rows.join("\n")
    }
}

impl GameState {
    pub fn new(
            admin_user: UserId,
            questions_storage: Box<QuestionsStorage>,
            questions_per_topic: usize,
            tours: Vec<TourDescription>,
            manual_questions: Vec<(String, usize)>,
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
            player_which_chose_question: None,
            current_player: None,
            questions: HashMap::new(),
            questions_storage,
            players_falsestarted: HashSet::new(),
            players_answered_current_question: HashSet::new(),
            questions_per_topic,
            tours,
            current_tour: 0,
            current_multiplier: 0,
            manual_questions,
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
            State::BeforeQuestionAsked(_, _) => {
                eprintln!("Now waiting for the question to be sent to the main chat");
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
            State::Bidding(_) => {
                eprintln!("Now bidding is on");
            }
            State::FinishingBid(ref bid_state) => {
                eprintln!("Auction was won by {:?}, now it's time to finalize the bid", &bid_state.player);
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
        for (topic, scores) in self.questions.iter() {
            let topic_name = topic.clone();
            let question_scores = scores.clone();


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

    fn close_unanswered_question(&mut self, question: Question, reason: Option<String>) -> Vec<UiRequest> {
        self.set_state(State::Pause);
        // Haven't received correct answer, so current player is which
        // asked the question (http://vladimirkhil.com/tv/game/10)
        self.current_player = self.player_which_chose_question.clone();

        let current_player_name = match self.current_player {
            Some(ref player) => player.name(),
            None => {panic!("Trying to process question, but no current player set")}
        };
        let msg = format!(
            "Правильный ответ: {}\nСледующий вопрос выбирает {}",
            question.answer(),
            current_player_name
        );

        if let Some(reason_message) = reason {
            vec![
                UiRequest::SendTextToMainChat(reason_message),
                UiRequest::SendTextToMainChat(msg)
            ]
        } else {
            vec![
                UiRequest::SendTextToMainChat(msg)
            ]
        }
    }

    fn close_answered_question(&mut self, reason: Option<String>) -> Vec<UiRequest> {
        self.set_state(State::Pause);
        self.player_which_chose_question = None;

        let current_player_name = match self.current_player {
            Some(ref player) => player.name(),
            None => {panic!("Trying to process question, but no current player set")}
        };
        let msg = format!(
            "Игру продолжает {}",
            current_player_name
        );

        if let Some(reason_message) = reason {
            vec![
                UiRequest::SendTextToMainChat(reason_message),
                UiRequest::SendTextToMainChat(msg)
            ]
        } else {
            vec![
                UiRequest::SendTextToMainChat(msg)
            ]
        }
    }

    fn check_bid_while_bidding(&self, player: &Player, bid: i64) -> Result<bool, String> {
        Err(String::from("Not implemented yet"))
    }

    fn parse_bid(bid: &str, score: i64) -> Result<i64, std::num::ParseIntError> {
        let all_in = vec!["ва-банк", "вабанк", "ва банк"];
        let pass = vec!["пас"];
        let bid = bid.to_lowercase();

        for opt in all_in.iter() {
            if bid == *opt {
                return Ok(score);
            }
        }
        for opt in pass.iter() {
            if bid == *opt {
                return Ok(-1);
            }
        }

        bid.parse::<i64>()
    }

    fn next_player_to_bid(current_player: &Player, bid: u64, scores: &HashMap<Player, i64>, passed_players: &HashSet<Player>) -> Option<Player> {
        let bid = bid as i64;
        let mut players = Vec::new();
        for (player, score) in scores.iter() {
            players.push((player.clone(), *score));
        }
        let comparator = |a : &(Player, i64), b : &(Player, i64)| {
            if a.1 != b.1 {
                a.1.cmp(&b.1)
            } else {
                a.0.name().cmp(b.0.name())
            }
        };
        players.sort_unstable_by(comparator);

        let all_in = *scores.get(&current_player).expect("Can't get bid for the current player") == bid;
        let can_bid = |player| {
            if passed_players.contains(player) {
                return false;
            }
            let score = scores[player];
            if score < bid {
                return false;
            }
            if score > bid {
                return true;
            }
            score == bid && !all_in
        };

        let mut found_player = false;
        for &(ref player, _) in players.iter() {
            if player == current_player {
                found_player = true;
                continue;
            }
            if !found_player {
                continue;
            }

            if can_bid(player) {
                return Some(player.clone());
            }
        }

        for &(ref player, _) in players.iter() {
            if player == current_player {
                break;
            }

            if can_bid(player) {
                return Some(player.clone());
            }
        }

        None
    }

    pub fn yes_reply(&mut self, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            println!("non-admin yes reply");
            return vec![];
        }
        if let State::Answering(_, cost) = self.state {
            match self.update_current_player_score(cost) {
                Ok(_) => {
                    self.close_answered_question(Some(String::from(CORRECT_ANSWER)))
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
                            UiRequest::Timeout(Some(INCORRECT_ANSWER.to_string()), Delay::Long),
                        ]
                    } else {
                        self.close_unanswered_question(question, Some(String::from("Все попытались, но ни у кого не получилось")))
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
        eprintln!("Scheduled timeout occurred");
        if let State::BeforeQuestionAsked(question, cost) = self.state.clone() {
            eprintln!("Falsestart section is about to start");
            self.set_state(State::Falsestart(question.clone(), cost));
            return vec![
                UiRequest::SendTextToMainChat(question.question()),
                UiRequest::Timeout(Some("!".into()), Delay::Short),
            ];
        }

        if let State::Falsestart(question, cost) = self.state.clone() {
            eprintln!("Falsestart section if finished, accepting answer now");
            self.set_state(State::CanAnswer(question.clone(), cost));
            return vec![
                UiRequest::Timeout(None, Delay::Long),
            ];
        };

        if let State::CanAnswer(question, _) = self.state.clone() {
            self.close_unanswered_question(question, Some(String::from("Время на ответ вышло!")))
        } else {
            eprintln!("unexpected timeout");
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
                if self.is_manual(&topic, &cost) {
                    eprintln!("manual question");
                    self.set_state(State::Pause);
                    vec![
                        UiRequest::SendToAdmin(format!(
                            "question: {}\nanswer: {}",
                            question.question(),
                            question.answer()
                        )),
                        UiRequest::SendTextToMainChat("Вопрос играется вручную".into()),
                    ]
                } else {
                    eprintln!("automatic question");
                    self.set_state(State::BeforeQuestionAsked(question.clone(), cost as i64));
                    self.player_which_chose_question = self.current_player.clone();
                    let main_chat_message = format!(
                        "Играем тему {}, вопрос за {}",
                        topic,
                        cost
                    );
                    vec![
                        UiRequest::SendToAdmin(format!(
                            "question: {}\nanswer: {}",
                            question.question(),
                            question.answer()
                        )),
                        UiRequest::SendTextToMainChat(main_chat_message),
                        UiRequest::Timeout(None, Delay::Medium),
                    ]
                }
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

    pub fn update_score(&mut self, name: String, newscore: i64, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            eprintln!("non admin user tried to update the score");
            return vec![];
        }

        let player = match self.find_player_by_name(&name) {
            Some(player) => {
                player.clone()
            }
            None => {
                eprintln!("{} not found", name);
                return vec![];
            }
        };

        if let Some(score) = self.players.get_mut(&player) {
            eprintln!("{} score updated", name);
            *score = newscore;
        } else {
            eprintln!("internal error: {} not found", name);
        }
        vec![]
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

    fn is_manual(&self, cur_topic: &String, cur_cost: &usize) -> bool {
        self.manual_questions.iter()
            .find(|&&(ref topic, ref cost)| cur_topic == topic && cur_cost == cost).is_some()
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
            vec![],
        ).unwrap()
    }

    fn select_question<T: ToString>(game_state: &mut GameState, topic: T, player: UserId, cost: usize) {
        let topic = topic.to_string();
        game_state.set_current_player(player).unwrap();
        game_state.select_topic(topic.clone(), player);
        game_state.select_question(topic, cost, player);
        game_state.timeout();
        game_state.timeout();
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
        game_state.timeout();
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
            vec![],
        ).is_err());

        // Non existing topic
        let questions_storage: Box<QuestionsStorage> = Box::new(FakeQuestionsStorage::new());
        assert!(GameState::new(
            admin,
            questions_storage,
            5,
            tours,
            vec![],
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
            vec![],
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

        select_question(&mut game_state, "Movies", p1, 200);
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
        game_state.timeout();
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
        game_state.timeout();
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
        game_state.timeout();
        game_state.message(p1, String::from("1"));
        game_state.timeout();
        game_state.message(p2, String::from("1"));
        game_state.no_reply(admin);
        game_state.message(p1, String::from("1"));
        game_state.yes_reply(admin);

        assert_eq!(game_state.get_player_score(p1), Some(100));
        assert_eq!(game_state.get_player_score(p2), Some(-100));
    }

    #[test]
    fn test_score_table_to_string() {
        let table = ScoreTable {
            scores: vec![10, 30, 20],
            data: vec![
                ScoreTableItem{
                    name: String::from("a"),
                    questions: vec![10, 20]
                }
            ]
        };

        assert_eq!(table.to_string(), "|a|x| |x|");

        let table = ScoreTable {
            scores: vec![10, 30, 20],
            data: vec![
                ScoreTableItem{
                    name: String::from("a"),
                    questions: vec![10, 20]
                },
                ScoreTableItem{
                    name: String::from("привет"),
                    questions: vec![30]
                }
            ]
        };

        assert_eq!(table.to_string(), "|a     |x| |x|\n|привет| |x| |");
    }

    #[test]
    fn test_players_turns() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let p2 = UserId::from(3);
        let mut game_state = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.add_player(p2, String::from("new_2"));
        game_state.start(admin);

        // first no, second no
        game_state.next_question(admin);
        select_question(&mut game_state, "Sport", p1, 100);
        game_state.message(p1, String::from("1"));
        game_state.no_reply(admin);
        game_state.message(p2, String::from("1"));
        game_state.no_reply(admin);
        // no correct answer, so question is closed
        assert_eq!(game_state.get_state(), &State::Pause);
        // checking, that despite the second player answered last
        // the current player is the first one
        assert_eq!(game_state.get_current_player().map(|p| p.id()), Some(p1));

        // first no, second yes
        game_state.next_question(admin);
        select_question(&mut game_state, "Sport", p1, 200);
        game_state.message(p1, String::from("1"));
        game_state.no_reply(admin);
        game_state.message(p2, String::from("1"));
        game_state.yes_reply(admin);
        // correct answer, so question is closed
        assert_eq!(game_state.get_state(), &State::Pause);
        // checking, that the second player caught turn by correct answer
        assert_eq!(game_state.get_current_player().map(|p| p.id()), Some(p2));
    }

    #[test]
    fn test_closing_questions() {
        let admin_id = UserId::from(1);
        let p1_id = UserId::from(2);
        let p2_id = UserId::from(3);
        let mut game_state = create_game_state(admin_id);
        game_state.add_player(p1_id, String::from("new_1"));
        game_state.add_player(p2_id, String::from("new_2"));
        game_state.start(admin_id);

        let p1 = Player::new(String::from("new_1"), p1_id);
        let p2 = Player::new(String::from("new_2"), p2_id);
        let mut players_answered = HashSet::new();

        // first question asked
        game_state.next_question(admin_id);
        select_question(&mut game_state, "Sport", p1_id, 100);
        match game_state.get_state() {
            &State::CanAnswer(_, _) => {}
            _ => {
                panic!("Must be in CanAnswer state now: no players answered");
            }
        }

        assert_eq!(game_state.players_answered_current_question, players_answered);

        // first player answers wrongly
        game_state.message(p1_id, String::from("1"));
        game_state.no_reply(admin_id);
        players_answered.insert(p1.clone());
        assert_eq!(game_state.players_answered_current_question, players_answered);
        match game_state.get_state() {
            &State::CanAnswer(_, _) => {}
            _ => {
                panic!("Must be in CanAnswer state now: first player answered, but the second is up");
            }
        }

        // second player answers wrongly
        game_state.message(p2_id, String::from("2"));
        game_state.no_reply(admin_id);
        players_answered.insert(p2.clone());
        assert_eq!(game_state.players_answered_current_question, players_answered);

        // question must be closed by now
        assert_eq!(game_state.get_state(), &State::Pause);

        game_state.next_question(admin_id);
        select_question(&mut game_state, "Sport", p1_id, 200);
        match game_state.get_state() {
            &State::CanAnswer(_, _) => {}
            _ => {
                panic!(format!("Must be in CanAnswer state now: no players answered; but in {:?}", game_state.get_state()));
            }
        }
        players_answered.clear();
        // this is the next question, so no players answered yet
        assert_eq!(game_state.players_answered_current_question, players_answered);

        // second player answers wrongly
        game_state.message(p2_id, String::from("1"));
        game_state.no_reply(admin_id);
        players_answered.insert(p2.clone());
        assert_eq!(game_state.players_answered_current_question, players_answered);
        match game_state.get_state() {
            &State::CanAnswer(_, _) => {}
            _ => {
                panic!("Must be in CanAnswer state now: second player answered, but the first is up");
            }
        }

    }

    #[test]
    fn test_manual_questions() {
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
        ];

        let admin_id = UserId::from(1);
        let p1_id = UserId::from(2);

        let mut game_state = GameState::new(
            admin_id,
            questions_storage,
            5,
            tours,
            vec![("Sport".to_string(), 100)],
        ).unwrap();

        game_state.add_player(p1_id, String::from("new_1"));
        game_state.start(admin_id);

        game_state.next_question(admin_id);
        game_state.set_current_player(p1_id).unwrap();
        game_state.select_topic("Sport", p1_id);
        game_state.select_question("Sport", 100, p1_id);

        match game_state.get_state() {
            &State::Pause => {}
            _ => {
                panic!("Manual question should set game state to pause");
            }
        }
    }

    #[test]
    fn test_parse_bid() {
        assert_eq!(GameState::parse_bid("вабанк", 100), Ok(100));
        assert_eq!(GameState::parse_bid("Вабанк", 100), Ok(100));
        assert_eq!(GameState::parse_bid("ва-банк", 100), Ok(100));
        assert_eq!(GameState::parse_bid("Ва-банк", 100), Ok(100));
        assert_eq!(GameState::parse_bid("пас", 100), Ok(-1));
        assert_eq!(GameState::parse_bid("Пас", 100), Ok(-1));
        // This method won't check, that the bid is higher than score
        assert_eq!(GameState::parse_bid("123", 100), Ok(123));
        // This method won't check, that the bid is negative
        assert_eq!(GameState::parse_bid("-50", 100), Ok(-50));
        assert!(GameState::parse_bid("М?", 100).is_err(), "Should fail");
        assert!(GameState::parse_bid("x23", 100).is_err(), "Should fail");
    }

    #[test]
    fn test_check_bid_while_bidding() {
        let admin = UserId::from(1);
        let p1 = Player::new(String::from("Stas"), UserId::from(2));
        let p2 = Player::new(String::from("Sasha"), UserId::from(3));
        let mut game_state = create_game_state(admin);
        game_state.add_player(p1.id(), p1.name().clone());
        game_state.add_player(p2.id(), p2.name().clone());
        game_state.start(admin);

        let mut passed = HashSet::new();

        game_state.players.insert(p1.clone(), 100);
        game_state.players.insert(p2.clone(), 80);

        let state = BiddingState {
            question : Question::new("?", "!"),
            passed: passed,
            bid: 80,
            current_player: p1.clone()
        };

        assert!(game_state.check_bid_while_bidding(&p2, 80), "Must be able to beat with all-in");
        assert!(!game_state.check_bid_while_bidding(&p2, 81), "Can't bid more than have");
        assert!(!game_state.check_bid_while_bidding(&p2, 79), "Can't bid less than current bid");
        assert!(game_state.check_bid_while_bidding(&p2, -1), "Must be able to pass");
    }

    #[test]
    fn test_next_player_to_bid() {
        let p1 = Player::new(String::from("Stas"), UserId::from(1));
        let p2 = Player::new(String::from("Sasha"), UserId::from(2));
        let p3 = Player::new(String::from("Masha"), UserId::from(3));

        let mut score = HashMap::new();
        score.insert(p1.clone(), 1000);
        score.insert(p2.clone(), 700);
        score.insert(p3.clone(), 10000);
        let mut passed = HashSet::new();
        passed.insert(p2.clone());
        assert_eq!(
            GameState::next_player_to_bid(
                &p1, 800, &score, &passed
            ),
            Some(p3.clone())
        );

        let mut score = HashMap::new();
        score.insert(p1.clone(), 1000);
        score.insert(p2.clone(), 700);
        score.insert(p3.clone(), 10000);
        let mut passed = HashSet::new();
        passed.insert(p2.clone());
        assert_eq!(
            GameState::next_player_to_bid(
                &p3, 800, &score, &passed
            ),
            Some(p1.clone())
        );

        // Sasha got auction, has nominal bid
        // so Stas is the next bidder, because he
        // obviously has lesser score, than Masha
        let mut score = HashMap::new();
        score.insert(p1.clone(), 1000);
        score.insert(p2.clone(), 700);
        score.insert(p3.clone(), 10000);
        let passed = HashSet::new();
        assert_eq!(
            GameState::next_player_to_bid(
                &p2, 800, &score, &passed
            ),
            Some(p1.clone())
        );

        // Masha got bid that no one can meet
        let mut score = HashMap::new();
        score.insert(p1.clone(), 1000);
        score.insert(p2.clone(), 700);
        score.insert(p3.clone(), 10000);
        let passed = HashSet::new();
        assert_eq!(
            GameState::next_player_to_bid(
                &p3, 1500, &score, &passed
            ),
            None
        );


        // scenario: 0
        // Sasha got nominal of 600, Stas is the next bidder
        // because he has less score than Masha (again, obviosly)
        let mut score = HashMap::new();
        score.insert(p1.clone(), 1000);
        score.insert(p2.clone(), 700);
        score.insert(p3.clone(), 10000);
        let mut passed = HashSet::new();
        assert_eq!(
            GameState::next_player_to_bid(
                &p2, 600, &score, &passed
            ),
            Some(p1.clone())
        );
        // Stas takes risk of having 601 bid
        // the next bidder is Masha
        assert_eq!(
            GameState::next_player_to_bid(
                &p1, 601, &score, &passed
            ),
            Some(p3.clone())
        );
        // Masha reads BBC news, so she don't want
        // to play such an easy game and passes
        // Sasha is the next bidder
        passed.insert(p3.clone());
        assert_eq!(
            GameState::next_player_to_bid(
                &p1, 601, &score, &passed
            ),
            Some(p2.clone())
        );
        // Sasha tries to trick Stas and go all-in,
        // so Stas will lose all in a presumably difficult question
        // What would Stas do?
        assert_eq!(
            GameState::next_player_to_bid(
                &p2, 700, &score, &passed
            ),
            Some(p1.clone())
        );
        // he passes, so Sasha has played herself
        passed.insert(p1.clone());
        assert_eq!(
            GameState::next_player_to_bid(
                &p2, 700, &score, &passed
            ),
            None
        );

        // scenario: 1
        // Masha got nominal of 800, Sasha is out before she's in
        // Stas is the next bidder
        let mut score = HashMap::new();
        score.insert(p1.clone(), 1000);
        score.insert(p2.clone(), 700);
        score.insert(p3.clone(), 10000);
        let mut passed = HashSet::new();
        assert_eq!(
            GameState::next_player_to_bid(
                &p3, 800, &score, &passed
            ),
            Some(p1.clone())
        );
        // Stas plays 5D tic-tac-toe, and takes a bid of 813
        assert_eq!(
            GameState::next_player_to_bid(
                &p1, 813, &score, &passed
            ),
            Some(p3.clone())
        );
        // Masha doesn't get Stas's complex logic and tries him
        // with the bid of 1000
        assert_eq!(
            GameState::next_player_to_bid(
                &p3, 1000, &score, &passed
            ),
            Some(p1.clone())
        );
        // Stas isn't bitchmade and goes all in
        assert_eq!(
            GameState::next_player_to_bid(
                &p1, 1000, &score, &passed
            ),
            Some(p3.clone())
        );
        // Masha is off to reading bbc, so Stas is playing
        passed.insert(p3.clone());
        assert_eq!(
            GameState::next_player_to_bid(
                &p1, 1000, &score, &passed
            ),
            None
        );
    }
}
