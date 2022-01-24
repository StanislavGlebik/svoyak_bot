use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::TryInto;
use std::path::PathBuf;

use serde_derive::Serialize;
use telegram_bot::UserId;

use failure::{err_msg, Error};

use crate::messages::*;
use crate::player::Player;
use crate::question::Question;
use crate::questionsstorage::{CatInBag, TourDescription, QuestionsStorage};

#[derive(Clone, Debug, Eq, PartialEq)]
enum State {
    WaitingForPlayersToJoin,
    WaitingForTopic,
    WaitingForQuestion,
    BeforeQuestionAsked(Question, i64),
    Falsestart(Question, i64),
    CanAnswer(Question, i64),
    WaitingForAuction(Question),
    // question, cost, anyone can answer
    Answering(Question, i64, bool),

    CatInBagChoosingPlayer(String, Question),
    CatInBagChoosingCost(Question),

    Pause,
}

pub struct GameState {
    admin_user: UserId,
    state: State,
    players: HashMap<Player, i64>,
    current_player: Option<Player>,
    player_which_chose_question: Option<Player>,
    questions: Vec<(String, Vec<usize>)>,
    players_falsestarted: HashSet<Player>,
    players_answered_current_question: HashSet<Player>,
    questions_per_topic: usize,
    tours: Vec<TourDescription>,
    current_tour: usize,
    current_multiplier: usize,
    manual_questions: Vec<(String, usize)>,
    cats_in_bags: Vec<CatInBag>,
    auctions: Vec<(String, usize)>,
}

pub enum UiRequest {
    SendTextToMainChat(String),
    SendHtmlToMainChat(String),
    SendImage(PathBuf),
    SendAudio(PathBuf),
    Timeout(Option<String>, Delay),
    ChooseTopic(String, Vec<String>),
    ChooseQuestion(String, Vec<usize>),
    AskAdminYesNo(String),
    SendToAdmin(String),
    SendScoreTable(ScoreTable),
    StopTimer,
    CatInBagChoosePlayer(Vec<Player>),
    CatInBagChooseCost(Vec<usize>),
}

pub enum Delay {
    Short,
    Medium,
    Long,
}

#[derive(Serialize)]
struct ScoreTableItem {
    name: String,
    questions: Vec<usize>,
}

#[derive(Serialize)]
pub struct ScoreTable {
    scores: Vec<usize>,
    data: Vec<ScoreTableItem>,
}

impl ScoreTable {
    pub fn to_string(&self) -> String {
        let mut rows: Vec<String> = Vec::new();

        let mut topic_length: usize = 0;
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
        questions_storage: &Box<dyn QuestionsStorage>,
        questions_per_topic: usize,
    ) -> Result<Self, Error> {
        if questions_per_topic == 0 {
            return Err(err_msg(String::from("questions per topic can't be zero")));
        }
        let tours = questions_storage.get_tours();
        for tour in tours.iter() {
            for topic in tour.topics.iter() {
                for i in 0..questions_per_topic {
                    let question_num = i + 1;
                    let topic_name = &topic.name;
                    if questions_storage.get(topic_name.clone(), i + 1).is_none() {
                        return Err(err_msg(format!(
                            "{} is not found in {}",
                            topic_name, question_num
                        )));
                    }
                }
            }
        }

        let manual_questions = questions_storage.get_manual_questions();

        Ok(Self {
            admin_user,
            state: State::WaitingForPlayersToJoin,
            players: HashMap::new(),
            player_which_chose_question: None,
            current_player: None,
            questions: Vec::new(),
            players_falsestarted: HashSet::new(),
            players_answered_current_question: HashSet::new(),
            questions_per_topic,
            tours,
            current_tour: 0,
            current_multiplier: 0,
            manual_questions,
            cats_in_bags: questions_storage.get_cats_in_bags(),
            auctions: questions_storage.get_auctions(),
        })
    }

    fn set_state(&mut self, state: State) {
        self.state = state;
        match self.state {
            State::WaitingForQuestion => {
                eprintln!("/question command was executed");

                for (player, score) in self.players.iter() {
                    eprintln!("{}: {}\n", player.name(), score);
                }

                self.players_falsestarted.clear();
                self.players_answered_current_question.clear();
            }
            State::Answering(_, _, _) => {
                eprintln!(
                    "Now waiting for player '{:?}' to answer",
                    self.current_player.as_ref()
                );
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
            State::WaitingForAuction(_) => {
                eprintln!("Waiting for an auction cost to be decided");
            }
            State::Pause => {
                eprintln!("The game is paused");
            }
            State::WaitingForTopic => {
                eprintln!("Waiting for the choice of topic");
            }
            State::CatInBagChoosingPlayer(..) => {
                eprintln!("Waiting while cat in bag player is chosen");
            }
            State::CatInBagChoosingCost(..) => {
                eprintln!("Waiting while cat in bag cost is chosen");
            }
        }
    }

    pub fn update_auction_cost(&mut self, maybe_admin: UserId, name: String, cost: usize) -> Vec<UiRequest> {
        if maybe_admin != self.admin_user {
            println!("non admin user attempted to update auction cost");
            return vec![];
        }

        let question = match &self.state {
            State::WaitingForAuction(question) => {
                question.clone()
            }
            _ => {
                eprintln!("Cannot update auction, wrong state");
                return vec![];
            }
        };

        if let Some(player) = self.find_player_by_name(&name) {
            self.current_player = Some(player.clone());
        } else {
            eprintln!("user {} not found", name);
            return vec![];
        }

        self.player_which_chose_question = self.current_player.clone();

        // Only this player can answer
        self.set_state(State::Answering(question.clone(), cost.try_into().unwrap(), false));

        let mut res = vec![
            UiRequest::SendTextToMainChat(format!("Играем аукцион с {}, стоимость {}", name, cost)),
        ];
        res.extend(self.format_question(&question));
        res.push(UiRequest::AskAdminYesNo("Correct answer?".to_string()));
        res
    }

    fn format_question(&self, question: &Question) -> Vec<UiRequest> {
        let mut res = vec![];
        if let Some(image) = question.image() {
            res.push(UiRequest::SendImage(image.to_path_buf()));
        }
        if let Some(audio) = question.audio() {
            res.push(UiRequest::SendAudio(audio.to_path_buf()));
        }
        let question_msg = question.question();
        res.push(UiRequest::SendTextToMainChat(question_msg));
        res
    }

    pub fn add_player(&mut self, new_user: UserId, name: String) -> Vec<UiRequest> {
        if self.state != State::WaitingForPlayersToJoin {
            println!("{} tried to join, but the game has already started", name);
            return vec![];
        }

        if !self.find_player(new_user).is_none() {
            vec![UiRequest::SendTextToMainChat(String::from(
                "Такой игрок уже существует",
            ))]
        } else if !self.find_player_by_name(&name).is_none() {
            vec![UiRequest::SendTextToMainChat(String::from(
                "Игрок с таким именем уже существует",
            ))]
        } else {
            self.players.insert(Player::new(name.clone(), new_user), 0);
            vec![UiRequest::SendTextToMainChat(format!("Привет {}", name))]
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
                return vec![UiRequest::SendTextToMainChat(String::from(
                    "Ни одного игрока не зарегистрировалось!",
                ))];
            }

            self.current_tour = 0;
            self.reload_available_questions();
            self.set_state(State::Pause);

            // let mut topics = String::from("Вот темы сегодняшней игры, они как всегда прекрасны:\n");
            // for (id, tour) in self.tours.iter().enumerate() {
            //     topics += &format!("<b>Тур {}</b>\n", id + 1);
            //     for topic in &tour.topics {
            //         topics += &format!("{}\n", topic.name);
            //     }
            // }

            vec![
                UiRequest::SendTextToMainChat(format!("Здравствуйте, здравствуйте, добрый день! Это своя игра!")),
                // UiRequest::SendHtmlToMainChat(topics),
                UiRequest::SendTextToMainChat(format!(
                    "Игру начинает {}",
                    self.current_player.clone().unwrap().name()
                ))
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
        vec![UiRequest::SendTextToMainChat(
            "Переходим к следующему туру".to_string(),
        )]
    }

    pub fn message(&mut self, user: UserId, _message: String) -> Vec<UiRequest> {
        eprintln!("User {} sent a message '{}'", user, _message);
        if let State::Falsestart(_, _) = self.state.clone() {
            let player = self.find_player(user).cloned();
            match player {
                Some(player) => {
                    self.players_falsestarted.insert(player.clone());
                    return vec![UiRequest::SendTextToMainChat(format!(
                        "Фальшстарт {}",
                        player.name()
                    ))];
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
                    self.players_answered_current_question
                        .insert(player.clone());
                    // Anyone can answer
                    self.set_state(State::Answering(question, cost, true));
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

            data.push(ScoreTableItem {
                name: topic_name,
                questions: question_scores,
            })
        }

        ScoreTable { scores, data }
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
        let topics: Vec<_> = self
            .questions
            .iter()
            .filter(|&(_, costs)| !costs.is_empty())
            .map(|(topic, _)| topic.clone())
            .collect();
        vec![
            UiRequest::SendScoreTable(self.make_score_table()),
            UiRequest::ChooseTopic(current_player_name, topics),
        ]
    }

    fn close_unanswered_question(
        &mut self,
        question: Question,
        reason: Option<String>,
    ) -> Vec<UiRequest> {
        self.set_state(State::Pause);
        // Haven't received correct answer, so current player is which
        // asked the question (http://vladimirkhil.com/tv/game/10)
        self.current_player = self.player_which_chose_question.clone();

        let score_msg = self.get_score_str();
        let current_player_name = match self.current_player {
            Some(ref player) => player.name(),
            None => panic!("Trying to process question, but no current player set"),
        };

        let mut msg = format!("Правильный ответ: {}\n", question.answer());
        if let Some(comments) = question.comments() {
            if comments.len() > 0 {
                msg.push_str(&format!("Комментарий:{}\n", comments));
            }
        }

        msg.push_str(&format!("{}\nСледующий вопрос выбирает {}", score_msg, current_player_name));

        if let Some(reason_message) = reason {
            vec![
                UiRequest::SendTextToMainChat(reason_message),
                UiRequest::SendTextToMainChat(msg),
            ]
        } else {
            vec![UiRequest::SendTextToMainChat(msg)]
        }
    }

    fn close_answered_question(&mut self, reason: Option<String>) -> Vec<UiRequest> {
        self.set_state(State::Pause);
        self.player_which_chose_question = None;

        let mut msg = self.get_score_str();
        let current_player_name = match self.current_player {
            Some(ref player) => player.name(),
            None => panic!("Trying to process question, but no current player set"),
        };
        msg += "\n";
        msg += &format!("Игру продолжает {}", current_player_name);

        if let Some(reason_message) = reason {
            vec![
                UiRequest::SendTextToMainChat(format!("{}\n{}", reason_message, msg))
            ]
        } else {
            vec![UiRequest::SendTextToMainChat(msg)]
        }
    }

    pub fn yes_reply(&mut self, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            println!("non-admin yes reply");
            return vec![];
        }
        if let State::Answering(question, cost, _) = &self.state {

            let message = match question.comments() {
                Some(comments) if comments.len() > 0 => {
                    format!("{}\nКомментарий: {}", CORRECT_ANSWER, comments)
                }
                _ => {
                    String::from(CORRECT_ANSWER)
                }
            };
            let res = match self.update_current_player_score(*cost) {
                Ok(_) => self.close_answered_question(Some(message)),
                Err(err_msg) => {
                    println!("{}", err_msg);
                    vec![]
                }
            };

            let mut res_score = String::new();
            for (player, score) in self.players.iter() {
                res_score += &format!("{}: {}\n", player.name(), score);
            }
            println!("score: {}", res_score);

            res
        } else {
            println!("unexpected yes answer");
            vec![]
        }
    }

    pub fn no_reply(&mut self, user: UserId) -> Vec<UiRequest> {
        println!("no reply");
        if user != self.admin_user {
            println!("non-admin no reply");
            return vec![];
        }

        if let State::Answering(question, cost, anyone_can_answer) = self.state.clone() {

            let res = match self.update_current_player_score(-cost) {
                Ok(_) => {
                    if anyone_can_answer {
                        if self.players_answered_current_question.len() != self.players.len() {
                            self.set_state(State::CanAnswer(question, cost));
                            self.players_falsestarted.clear();
                            vec![
                                UiRequest::SendTextToMainChat(INCORRECT_ANSWER.to_string()),
                                UiRequest::Timeout(
                                    None,
                                    Delay::Long,
                                )
                            ]
                        } else {
                            self.close_unanswered_question(
                                question,
                                Some(String::from("Все попытались, но ни у кого не получилось")),
                            )
                        }
                    } else {
                        self.close_unanswered_question(
                            question,
                            Some(String::from("Нет")),
                        )
                    }
                }
                Err(err_msg) => {
                    println!("{}", err_msg);
                    vec![]
                }
            };

            let mut res_score = String::new();
            for (player, score) in self.players.iter() {
                res_score += &format!("{}: {}\n", player.name(), score);
            }
            println!("score: {}", res_score);
            res
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

            let delay = if question.image().is_some() {
                Delay::Long
            } else if question.question().len() <= 100 {
                Delay::Short
            } else if question.question().len() <= 230 {
                Delay::Medium
            } else {
                Delay::Long
            };

            let mut res = vec![];
            res.extend(self.format_question(&question));
            res.push(UiRequest::Timeout(Some("!".into()), delay));
            return res;
        }

        if let State::Falsestart(question, cost) = self.state.clone() {
            eprintln!("Falsestart section if finished, accepting answer now");
            self.set_state(State::CanAnswer(question.clone(), cost));
            return vec![UiRequest::Timeout(None, Delay::Long)];
        };

        if let State::CanAnswer(question, _) = self.state.clone() {
            self.close_unanswered_question(question, Some(String::from("Время на ответ вышло!")))
        } else {
            eprintln!("unexpected timeout");
            vec![]
        }
    }

    pub fn select_topic<T: ToString>(&mut self, topic: T, user: UserId) -> Vec<UiRequest> {
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
        match self.questions.iter().find(|(t, _)| t == &topic).cloned() {
            Some((_, costs)) => {
                if !costs.is_empty() {
                    self.set_state(State::WaitingForQuestion);
                    vec![UiRequest::ChooseQuestion(topic.clone(), costs.clone())]
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
        questions_storage: &Box<dyn QuestionsStorage>,
    ) -> Vec<UiRequest> {
        if self.state != State::WaitingForQuestion {
            println!("unexpected question selection");
            return vec![];
        }

        if !self.is_current_player(user) {
            println!("only current player can select questions");
            return vec![];
        }

        let mut found = false;
        let topic = topic.to_string();
        for (cur_topic, costs) in &mut self.questions {
            if cur_topic == &topic {
                found = true;
                if costs.contains(&cost) {
                    costs.retain(|elem| elem != &cost);
                    eprintln!(
                        "Question in topic '{}' and cost {} was selected",
                        topic, cost
                    );
                } else {
                    eprintln!(
                        "Question in topic '{}' and cost {} was already used!",
                        topic, cost
                    );
                    return vec![];
                }
            }
        }

        if !found {
            println!("unknown topic");
            return vec![];
        }

        let mut reply = vec![];
        reply.push(
            UiRequest::SendTextToMainChat(format!("Играем тему {}, вопрос за {}", topic, cost))
        );

        let maybe_cat_in_bag = self.is_cat_in_bag(&topic, &cost);
        if let Some((new_topic, question)) = maybe_cat_in_bag {
            self.set_state(State::CatInBagChoosingPlayer(new_topic, question.clone()));
            reply.push(
                UiRequest::SendToAdmin(format!(
                    "question: {}\nanswer: {}",
                    question.question(),
                    question.answer(),
                ))
            );
            reply.push(UiRequest::SendTextToMainChat("Кот в мешке!".into()));
            reply.push(
                UiRequest::CatInBagChoosePlayer(
                    self.players
                        .keys()
                        .map(|player| player.clone())
                        .filter(|player| Some(player) != self.current_player.as_ref())
                        .collect::<Vec<_>>()
                )
            );
            return reply;
        }

        match questions_storage
            .get(topic.clone(), cost / self.current_multiplier)
        {
            Some(question) => {
                reply.push(
                    UiRequest::SendToAdmin(format!(
                        "question: {}\nanswer: {}",
                        question.question(),
                        question.answer(),
                    ))
                );

                if self.is_manual(&topic, &cost) {
                    eprintln!("manual question");
                    self.set_state(State::Pause);
                    reply.push(
                        UiRequest::SendTextToMainChat("Вопрос играется вручную".into()),
                    );
                    reply
                } else if self.is_auction(&topic, &cost) {
                    eprintln!("auction");
                    self.set_state(State::WaitingForAuction(question.clone()));
                    let score = self.get_score_str();
                    reply.push(
                       UiRequest::SendTextToMainChat(format!("Аукцион!\n{}", score))
                    );
                    reply
                } else {
                    eprintln!("automatic question");
                    self.set_state(State::BeforeQuestionAsked(question.clone(), cost as i64));
                    self.player_which_chose_question = self.current_player.clone();
                    reply.push(
                        UiRequest::Timeout(None, Delay::Medium),
                    );
                    reply
                }
            }
            None => {
                println!("internal error: question is not found");
                vec![]
            }
        }
    }

    pub fn select_cat_in_bag_player(&mut self, user: UserId, selected_player: String) -> Vec<UiRequest> {
        let cur_state = self.state.clone();
        match cur_state {
            State::CatInBagChoosingPlayer(topic, question) => {
                if Some(user) != self.current_player.clone().map(|x| x.id()) {
                    eprintln!("invalid user {} tried to select cat in bag player", user);
                    return vec![];
                }

                let players = self.players.clone();
                for (player, _) in players {
                    // Can't select themselves
                    if player.id() == user {
                        continue;
                    }
                    if player.name() == &selected_player {
                        self.current_player = Some(player.clone());
                        self.player_which_chose_question = Some(player.clone());
                        self.set_state(State::CatInBagChoosingCost(question));
                        return vec![
                            UiRequest::SendTextToMainChat(format!(
                                "Играем с {}. Тема: {}", player.name(), topic,
                            )),
                            UiRequest::CatInBagChooseCost(vec![
                                self.current_multiplier, self.current_multiplier * self.questions_per_topic
                            ])
                        ];
                    }
                }

                eprintln!("unknown player {} for cat in bag", selected_player);
                vec![]

            }
            _ => {
                eprintln!("not in cat in bag");
                vec![]
            }
        }
    }

    pub fn select_cat_in_bag_cost(&mut self, user: UserId, cost: usize) -> Vec<UiRequest> {
        let cur_state = self.state.clone();
        match cur_state {
            State::CatInBagChoosingCost(question) => {
                if Some(user) != self.current_player.clone().map(|x| x.id()) {
                    eprintln!("invalid user {} tried to select cat in bag cost", user);
                    return vec![];
                }
                if cost != self.current_multiplier && cost != self.current_multiplier * self.questions_per_topic {
                    eprintln!("invalid cost {}", cost);
                    return vec![];
                }

                // Only one person can answer
                self.set_state(State::Answering(question.clone(), cost as i64, false));

                let mut res = vec![
                    UiRequest::SendTextToMainChat(format!("Выбрана стоимость {}", cost)),
                ];
                res.extend(self.format_question(&question));
                res.push(UiRequest::AskAdminYesNo("Correct answer?".to_string()));
                res
            }
            _ => {
                eprintln!("not in cat in bag");
                vec![]
            }
        }
    }

    pub fn get_score(&mut self, _user: UserId) -> Vec<UiRequest> {
        vec![UiRequest::SendTextToMainChat(self.get_score_str())]
    }

    pub fn get_score_str(&self) -> String {
        let mut res = String::from("Счет:\n");
        for (player, score) in self.players.iter() {
            res += &format!("{}: {}\n", player.name(), score);
        }
        res
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

    pub fn change_player(&mut self, user: UserId, change_player: String) -> Vec<UiRequest> {
        if user != self.admin_user {
            eprintln!("non admin user tried to change player");
            return vec![];
        }

        if let Some(player) = self.find_player_by_name(&change_player) {
            self.current_player = Some(player.clone());
            vec![UiRequest::SendTextToMainChat(format!("Играет {}", change_player))]
        } else {
            vec![UiRequest::SendTextToMainChat(format!("Игрок {} не найден", change_player))]
        }
    }

    pub fn update_score(&mut self, name: String, newscore: i64, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            eprintln!("non admin user tried to update the score");
            return vec![];
        }

        let player = match self.find_player_by_name(&name) {
            Some(player) => player.clone(),
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

    pub fn hide_question(&mut self, topic: String, cost: usize, user: UserId) -> Vec<UiRequest> {
        if user != self.admin_user {
            eprintln!("non admin user tried to hide question");
            return vec![];
        }

        let mut found = false;
        for (cur_topic, costs) in &mut self.questions {
            if cur_topic == &topic {
                if costs.contains(&cost) {
                    found = true;
                    costs.retain(|elem| elem != &cost);
                } else {
                    break;
                }
            }
        }

        if found {
            eprintln!("hidden question");
        } else {
            eprintln!("question and topic to hide not found");
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
                    self.questions.push((topic.name.clone(), costs));
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
        self.manual_questions
            .iter()
            .find(|&&(ref topic, ref cost)| cur_topic == topic && cur_cost == cost)
            .is_some()
    }

    fn is_auction(&self, cur_topic: &String, cur_cost: &usize) -> bool {
        self.auctions
            .iter()
            .find(|&&(ref topic, ref cost)| cur_topic == topic && cur_cost == cost)
            .is_some()
    }

    fn is_cat_in_bag(&mut self, cur_topic: &String, cur_cost: &usize) -> Option<(String, Question)> {
        for cat_in_bag in &self.cats_in_bags {
            if &cat_in_bag.old_topic == cur_topic && &cat_in_bag.cost == cur_cost {
                return Some(
                    (
                        cat_in_bag.new_topic.clone(),
                        Question::new(
                            cat_in_bag.question.clone(),
                            cat_in_bag.answer.clone(),
                            None,
                        )
                    )
                );
            }
        }

        None
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
    use crate::questionsstorage::QuestionsStorage;
    use crate::questionsstorage::Topic;

    pub struct FakeQuestionsStorage {
        questions: HashMap<(String, usize), Question>,
        tours: Vec<TourDescription>,
        cats_in_bags: Vec<CatInBag>,
        manual_questions: Vec<(String, usize)>,
        auctions: Vec<(String, usize)>,
    }

    impl FakeQuestionsStorage {
        pub fn new(tours: Vec<TourDescription>) -> Self {
            let mut question_storage = HashMap::new();
            question_storage.insert((String::from("Sport"), 1), Question::new("2 * 2 = ?", "4", None));
            question_storage.insert((String::from("Sport"), 2), Question::new("3 * 2 = ?", "6", None));
            question_storage.insert((String::from("Sport"), 3), Question::new("4 * 2 = ?", "8", None));
            question_storage.insert((String::from("Sport"), 4), Question::new("5 * 2 = ?", "10", None));
            question_storage.insert((String::from("Sport"), 5), Question::new("6 * 2 = ?", "12", None));

            question_storage.insert((String::from("Movies"), 1), Question::new("2 * 2 = ?", "4", None));
            question_storage.insert((String::from("Movies"), 2), Question::new("3 * 2 = ?", "6", None));
            question_storage.insert((String::from("Movies"), 3), Question::new("4 * 2 = ?", "8", None));
            question_storage.insert(
                (String::from("Movies"), 4),
                Question::new("5 * 2 = ?", "10", None),
            );
            question_storage.insert(
                (String::from("Movies"), 5),
                Question::new("6 * 2 = ?", "12", None),
            );

            Self {
                questions: question_storage,
                tours,
                cats_in_bags: vec![],
                manual_questions: vec![],
                auctions: vec![],
            }
        }
    }

    impl QuestionsStorage for FakeQuestionsStorage {
        fn get(&self, topic_name: String, difficulty: usize) -> Option<Question> {
            self.questions.get(&(topic_name, difficulty)).cloned()
        }

        fn get_tours(&self) -> Vec<TourDescription> {
            self.tours.clone()
        }

        fn get_cats_in_bags(&self) -> Vec<CatInBag> {
            self.cats_in_bags.clone()
        }

        fn get_manual_questions(&self) -> Vec<(String, usize)> {
            self.manual_questions.clone()
        }

        fn get_auctions(&self) -> Vec<(String, usize)> {
            self.auctions.clone()
        }
    }

    fn create_game_state(user: UserId) -> (GameState, Box<dyn QuestionsStorage>) {
        let tours = vec![
            TourDescription {
                multiplier: 100,
                topics: vec![Topic {
                    name: "Sport".to_string(),
                }],
            },
            TourDescription {
                multiplier: 200,
                topics: vec![Topic {
                    name: "Movies".to_string(),
                }],
            },
        ];
        let questions_storage: Box<dyn QuestionsStorage> = Box::new(FakeQuestionsStorage::new(tours));
        (GameState::new(user, &questions_storage, 5).unwrap(), questions_storage)
    }

    fn select_question<T: ToString>(
        game_state: &mut GameState,
        questions_storage: &Box<dyn QuestionsStorage>,
        topic: T,
        player: UserId,
        cost: usize,
    ) {
        let topic = topic.to_string();
        game_state.set_current_player(player).unwrap();
        game_state.select_topic(topic.clone(), player);
        game_state.select_question(topic, cost, player, questions_storage);
        game_state.timeout();
        game_state.timeout();
    }

    #[test]
    fn test_add_player() {
        let (mut game_state, _) = create_game_state(UserId::from(1));
        game_state.add_player(UserId::from(1), String::from("new"));
        game_state.add_player(UserId::from(1), String::from("new"));
        assert_eq!(game_state.get_players().len(), 1);
    }

    #[test]
    fn test_start_game() {
        let (mut game_state, _) = create_game_state(UserId::from(1));
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
        let (mut game_state, questions_storage) = create_game_state(admin);
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

        game_state.select_question("Sport", 100, p1, &questions_storage);
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
        game_state.select_question("Sport", 1, p1, &questions_storage);
        // Cannot select already selected question
        assert_eq!(game_state.get_state(), &State::WaitingForQuestion);

        game_state.select_question("Sport", 200, p2, &questions_storage);
        // Only current player can select next question
        assert_eq!(game_state.get_state(), &State::WaitingForQuestion);
    }

    #[test]
    fn test_game_state_creation() {
        let admin = UserId::from(1);
        let tours = vec![TourDescription {
            multiplier: 100,
            topics: vec![Topic {
                name: "Nonexisting topic".to_string(),
            }],
        }];
        let questions_storage: Box<dyn QuestionsStorage> = Box::new(FakeQuestionsStorage::new(tours.clone()));

        // 0 question number
        assert!(GameState::new(admin, &questions_storage, 0).is_err());

        // Non existing topic
        let questions_storage: Box<dyn QuestionsStorage> = Box::new(FakeQuestionsStorage::new(tours.clone()));
        assert!(GameState::new(admin, &questions_storage, 5).is_err());

        // Incorrect question number
        let tours = vec![TourDescription {
            multiplier: 100,
            topics: vec![Topic {
                name: "Sport".to_string(),
            }],
        }];

        let questions_storage: Box<dyn QuestionsStorage> = Box::new(FakeQuestionsStorage::new(tours.clone()));
        assert!(GameState::new(admin, &questions_storage, 6).is_err());
    }

    #[test]
    fn test_tours_simple() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let (mut game_state, questions_storage) = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.start(admin);
        game_state.next_tour(admin);
        game_state.next_question(admin);

        select_question(&mut game_state, &questions_storage, "Movies", p1, 200);
        game_state.message(p1, String::from("1"));
        game_state.yes_reply(admin);

        assert_eq!(game_state.get_player_score(p1), Some(200));
    }

    #[test]
    fn test_falsestarts_simple() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let (mut game_state, questions_storage) = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.start(admin);
        game_state.next_question(admin);

        game_state.select_topic("Sport", p1);
        game_state.select_question("Sport", 200, p1, &questions_storage);
        game_state.timeout();
        game_state.message(p1, String::from("1"));
        game_state.timeout();
        game_state.message(p1, String::from("1"));
        match game_state.get_state() {
            &State::Answering(..) => {
                assert!(false);
            }
            _ => {}
        }
    }

    #[test]
    fn test_falsestarts_second_can_answer() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let p2 = UserId::from(3);
        let (mut game_state, questions_storage) = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.add_player(p2, String::from("new_2"));
        game_state.start(admin);
        game_state.next_question(admin);

        game_state.set_current_player(p1).unwrap();
        game_state.select_topic("Sport", p1);
        game_state.select_question("Sport", 100, p1, &questions_storage);
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
        let (mut game_state, questions_storage) = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.add_player(p2, String::from("new_2"));
        game_state.start(admin);
        game_state.next_question(admin);

        game_state.set_current_player(p1).unwrap();
        game_state.select_topic("Sport", p1);
        game_state.select_question("Sport", 100, p1, &questions_storage);
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
            data: vec![ScoreTableItem {
                name: String::from("a"),
                questions: vec![10, 20],
            }],
        };

        assert_eq!(table.to_string(), "|a|x| |x|");

        let table = ScoreTable {
            scores: vec![10, 30, 20],
            data: vec![
                ScoreTableItem {
                    name: String::from("a"),
                    questions: vec![10, 20],
                },
                ScoreTableItem {
                    name: String::from("привет"),
                    questions: vec![30],
                },
            ],
        };

        assert_eq!(table.to_string(), "|a     |x| |x|\n|привет| |x| |");
    }

    #[test]
    fn test_players_turns() {
        let admin = UserId::from(1);
        let p1 = UserId::from(2);
        let p2 = UserId::from(3);
        let (mut game_state, questions_storage) = create_game_state(admin);
        game_state.add_player(p1, String::from("new_1"));
        game_state.add_player(p2, String::from("new_2"));
        game_state.start(admin);

        // first no, second no
        game_state.next_question(admin);
        select_question(&mut game_state, &questions_storage, "Sport", p1, 100);
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
        select_question(&mut game_state, &questions_storage, "Sport", p1, 200);
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
        let (mut game_state, questions_storage) = create_game_state(admin_id);
        game_state.add_player(p1_id, String::from("new_1"));
        game_state.add_player(p2_id, String::from("new_2"));
        game_state.start(admin_id);

        let p1 = Player::new(String::from("new_1"), p1_id);
        let p2 = Player::new(String::from("new_2"), p2_id);
        let mut players_answered = HashSet::new();

        // first question asked
        game_state.next_question(admin_id);
        select_question(&mut game_state, &questions_storage, "Sport", p1_id, 100);
        match game_state.get_state() {
            &State::CanAnswer(_, _) => {}
            _ => {
                panic!("Must be in CanAnswer state now: no players answered");
            }
        }

        assert_eq!(
            game_state.players_answered_current_question,
            players_answered
        );

        // first player answers wrongly
        game_state.message(p1_id, String::from("1"));
        game_state.no_reply(admin_id);
        players_answered.insert(p1.clone());
        assert_eq!(
            game_state.players_answered_current_question,
            players_answered
        );
        match game_state.get_state() {
            &State::CanAnswer(_, _) => {}
            _ => {
                panic!(
                    "Must be in CanAnswer state now: first player answered, but the second is up"
                );
            }
        }

        // second player answers wrongly
        game_state.message(p2_id, String::from("2"));
        game_state.no_reply(admin_id);
        players_answered.insert(p2.clone());
        assert_eq!(
            game_state.players_answered_current_question,
            players_answered
        );

        // question must be closed by now
        assert_eq!(game_state.get_state(), &State::Pause);

        game_state.next_question(admin_id);
        select_question(&mut game_state, &questions_storage, "Sport", p1_id, 200);
        match game_state.get_state() {
            &State::CanAnswer(_, _) => {}
            _ => {
                panic!(format!(
                    "Must be in CanAnswer state now: no players answered; but in {:?}",
                    game_state.get_state()
                ));
            }
        }
        players_answered.clear();
        // this is the next question, so no players answered yet
        assert_eq!(
            game_state.players_answered_current_question,
            players_answered
        );

        // second player answers wrongly
        game_state.message(p2_id, String::from("1"));
        game_state.no_reply(admin_id);
        players_answered.insert(p2.clone());
        assert_eq!(
            game_state.players_answered_current_question,
            players_answered
        );
        match game_state.get_state() {
            &State::CanAnswer(_, _) => {}
            _ => {
                panic!(
                    "Must be in CanAnswer state now: second player answered, but the first is up"
                );
            }
        }
    }

    #[test]
    fn test_manual_questions() {
        let tours = vec![TourDescription {
            multiplier: 100,
            topics: vec![Topic {
                name: "Sport".to_string(),
            }],
        }];

        let mut questions_storage = FakeQuestionsStorage::new(tours);
        questions_storage.manual_questions = vec![("Sport".to_string(), 100)];
        let questions_storage: Box<dyn QuestionsStorage> = Box::new(questions_storage);

        let admin_id = UserId::from(1);
        let p1_id = UserId::from(2);

        let mut game_state = GameState::new(
            admin_id,
            &questions_storage,
            5,
        )
        .unwrap();

        game_state.add_player(p1_id, String::from("new_1"));
        game_state.start(admin_id);

        game_state.next_question(admin_id);
        game_state.set_current_player(p1_id).unwrap();
        game_state.select_topic("Sport", p1_id);
        game_state.select_question("Sport", 100, p1_id, &questions_storage);

        match game_state.get_state() {
            &State::Pause => {}
            _ => {
                panic!("Manual question should set game state to pause");
            }
        }
    }

    #[test]
    fn test_cats_in_bags_questions() {
        let tours = vec![TourDescription {
            multiplier: 100,
            topics: vec![Topic {
                name: "Sport".to_string(),
            }],
        }];
        let mut questions_storage = FakeQuestionsStorage::new(tours);
        questions_storage.cats_in_bags = vec![
                CatInBag {
                    old_topic: "Sport".to_string(),
                    cost: 100,
                    new_topic: "CatInBag".to_string(),
                    question: "question".to_string(),
                    answer: "answer".to_string(),
                }
            ];

        let questions_storage: Box<dyn QuestionsStorage> = Box::new(questions_storage);

        let admin_id = UserId::from(1);

        let mut game_state = GameState::new(
            admin_id,
            &questions_storage,
            5,
        )
        .unwrap();

        let p1_id = UserId::from(2);
        let p2_id = UserId::from(3);
        game_state.add_player(p1_id, String::from("new_1"));
        game_state.add_player(p2_id, String::from("new_2"));
        game_state.start(admin_id);

        game_state.next_question(admin_id);
        game_state.set_current_player(p1_id).unwrap();
        game_state.select_topic("Sport", p1_id);
        game_state.select_question("Sport", 100, p1_id, &questions_storage);

        // Wrong choices
        assert!(matches!(game_state.get_state(), State::CatInBagChoosingPlayer(_, _)));
        game_state.select_cat_in_bag_player(p2_id, "new_1".to_string());
        assert!(matches!(game_state.get_state(), State::CatInBagChoosingPlayer(_, _)));
        game_state.select_cat_in_bag_player(p2_id, "new_2".to_string());
        assert!(matches!(game_state.get_state(), State::CatInBagChoosingPlayer(_, _)));

        game_state.select_cat_in_bag_player(p1_id, "new_1".to_string());
        assert!(matches!(game_state.get_state(), State::CatInBagChoosingPlayer(_, _)));

        // Right choice
        game_state.select_cat_in_bag_player(p1_id, "new_2".to_string());
        assert!(matches!(game_state.get_state(), State::CatInBagChoosingCost(_)));

        // Select cost - wrong cost
        game_state.select_cat_in_bag_cost(p2_id, 200);
        assert!(matches!(game_state.get_state(), State::CatInBagChoosingCost(_)));
        // Select cost - wrong user id
        game_state.select_cat_in_bag_cost(p1_id, 500);
        assert!(matches!(game_state.get_state(), State::CatInBagChoosingCost(_)));

        // Select cost - right choice
        game_state.select_cat_in_bag_cost(p2_id, 500);
        assert!(matches!(game_state.get_state(), State::Answering(_, _, false)));

        assert_eq!(game_state.current_player.map(|x| x.id()), Some(p2_id));
    }

    #[test]
    fn test_auctions() {
        let tours = vec![TourDescription {
            multiplier: 100,
            topics: vec![Topic {
                name: "Sport".to_string(),
            }],
        }];
        let mut questions_storage = FakeQuestionsStorage::new(tours);
        questions_storage.auctions = vec![("Sport".to_string(), 100)];

        let questions_storage: Box<dyn QuestionsStorage> = Box::new(questions_storage);

        let admin_id = UserId::from(1);

        let mut game_state = GameState::new(
            admin_id,
            &questions_storage,
            5,
        )
        .unwrap();

        let p1_id = UserId::from(2);
        let p2_id = UserId::from(3);
        game_state.add_player(p1_id, String::from("new_1"));
        game_state.add_player(p2_id, String::from("new_2"));
        game_state.start(admin_id);

        game_state.next_question(admin_id);
        game_state.set_current_player(p1_id).unwrap();
        game_state.select_topic("Sport", p1_id);
        game_state.select_question("Sport", 100, p1_id, &questions_storage);

        assert!(matches!(game_state.get_state(), State::WaitingForAuction(_)));

        // non-admin user
        game_state.update_auction_cost(p1_id, "new_1".to_string(), 100);
        assert!(matches!(game_state.get_state(), State::WaitingForAuction(_)));

        game_state.update_auction_cost(admin_id, "new_1".to_string(), 100);
        assert!(matches!(game_state.get_state(), State::Answering(_, _, _)));
        assert_eq!(game_state.get_current_player().map(|p| p.id()), Some(p1_id));
    }
}
