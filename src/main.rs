extern crate failure;
extern crate futures;
#[macro_use]
extern crate telegram_bot;
extern crate tokio_core;

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::env;
use std::thread;
use std::time::Duration;

use failure::{err_msg, Error};
use futures::{Future, IntoFuture, Sink, Stream};
use futures::sync::mpsc;
use futures::future::Either;
use tokio_core::reactor::{Core, Timeout};

use telegram_bot::{Api, CanReplySendMessage, ChatId, InlineKeyboardMarkup, InlineKeyboardButton,
                   MessageKind};
use telegram_bot::{SendMessage, Update, UpdateKind, UpdatesStream, UserId};
use std::sync::{Arc, Mutex};

mod gamestate;
mod messages;
mod player;
mod timeout_stream;
mod question;

use messages::*;
use player::Player;

const ANSWER_YES: &str = "AnswerYes";
const ANSWER_NO: &str = "AnswerNo";


#[derive(Clone, Eq, PartialEq)]
enum State {
    WaitingForPlayersToJoin,
    WaitingForQuestion,
    Falsestart,
    CanAnswer(usize), // timer id
    Answering,
}

fn question_inline_keyboard(
    questions: &HashMap<String, Vec<usize>>,
) -> InlineKeyboardMarkup {
    let mut markup = InlineKeyboardMarkup::new();
    {
        for (topic, costs) in questions {
            let row = markup.add_empty_row();
            row.push(InlineKeyboardButton::callback(topic, "fakedata"));
            for cost in costs {
                let data = format!("/question{}_{}", topic, cost);
                row.push(InlineKeyboardButton::callback(format!("{}", cost), data));
            }
        }
    }
    markup
}

fn question_inline_keyboard_old(
    questions: &BTreeSet<usize>,
    multiplier: usize,
) -> InlineKeyboardMarkup {
    let mut markup = InlineKeyboardMarkup::new();
    {
        let row = markup.add_empty_row();
        for question_num in questions {
            let user_friendly_value = format!("{}", question_num * multiplier);
            let data = format!("question{}", question_num);
            row.push(InlineKeyboardButton::callback(user_friendly_value, data));
        }
    }
    markup
}

// if timer_id != global_timer_id, then this timer was "deleted", and it should just exit
fn timer_full_question(state: Arc<Mutex<State>>, timer_id: usize, chat_id: i64) {
    let mut core = Core::new().unwrap();
    let token = env::var("TELEGRAM_BOT_TOKEN").unwrap();
    let api = Api::configure(token).build(core.handle()).unwrap();
    let chat = ChatId::from(chat_id);

    {
        let mut val = state.lock().unwrap();
        *val = State::Falsestart;
    }
    thread::sleep(Duration::from_secs(1));

    let msg = SendMessage::new(chat, "!");
    let fut = api.send(msg).and_then(|_| {
        {
            let mut val = state.lock().expect("poisoned lock");
            *val = State::CanAnswer(timer_id);
        }
        // TODO(stash): tokio Timeout didn't work as expected!
        thread::sleep(Duration::from_secs(3));
        let msg = SendMessage::new(chat, TIMES_UP_MSG);

        let val = {
            state.lock().expect("poisoned lock").clone()
        };
        // Nobody replied - send message and move on to the next question
        if val == State::CanAnswer(timer_id) {
            let fut = api.send(msg).map({
                let state = state.clone();
                move |_| {
                    let mut val = state.lock().unwrap();
                    // TODO(stash): fix
                    *val = State::WaitingForQuestion;
                }
            });
            Either::A(fut)
        } else {
            Either::B(Ok(()).into_future())
        }
    });

    core.run(fut).unwrap();
}

fn timer_question_after_incorrect_answer(state: Arc<Mutex<State>>, timer_id: usize, chat_id: i64) {
    let mut core = Core::new().unwrap();
    let token = env::var("TELEGRAM_BOT_TOKEN").unwrap();
    let api = Api::configure(token).build(core.handle()).unwrap();
    let chat = ChatId::from(chat_id);

    {
        let mut state = state.lock().expect("poisoned lock");
        *state = State::CanAnswer(timer_id);
    }

    let msg = SendMessage::new(chat, INCORRECT_ANSWER);
    let fut = api.send(msg).and_then(|_| {
        // TODO(stash): tokio Timeout didn't work as expected!
        thread::sleep(Duration::from_secs(3));
        let msg = SendMessage::new(chat, TIMES_UP_MSG);

        let val = {
            state.lock().expect("poisoned lock").clone()
        };
        // Nobody replied - send message and move on to the next question
        if val == State::CanAnswer(timer_id) {
            let fut = api.send(msg).map({
                let state = state.clone();
                move |_| {
                    let mut val = state.lock().unwrap();
                    // TODO(stash): fix
                    *val = State::WaitingForQuestion;
                }
            });
            Either::A(fut)
        } else {
            Either::B(Ok(()).into_future())
        }
    });

    core.run(fut).unwrap();
}

fn start_timer(
    state: Arc<Mutex<State>>,
    global_timer_id: &mut usize,
    chat_id: i64,
    timer_func: fn(Arc<Mutex<State>>, usize, i64) -> (),
) {
    let current_id = *global_timer_id;
    *global_timer_id += 1;

    thread::spawn({
        let state = state.clone();
        move || timer_func(state, current_id, chat_id)
    });
}

fn find_player(players: &mut Vec<Player>, id: UserId) -> Option<&mut Player> {
    players.iter_mut().find(|player| player.id() == id)
}

fn merge_updates_and_timeouts(
    updates_stream: UpdatesStream,
    timeouts: timeout_stream::TimeoutStream,
) -> Box<Stream<Item = Result<Update, ()>, Error = Error>> {
    let updates_stream = Box::new(updates_stream.map(|update| Ok(update)).map_err(|err| {
        err_msg(format!("{}", err))
    }));

    let timeouts = Box::new(timeouts.map(|timeout| Err(timeout)).map_err(|err| {
        err_msg(format!("{}", err))
    }));
    Box::new(updates_stream.select(timeouts))
}

enum TextMessage {
    Join(String),
    JustMessage(String),
    NextQuestion,
    GetScore,
    StartGame,
    CurrentPlayer,
}

enum CallbackMessage {
    SelectedQuestion(String, usize),
    AnswerYes,
    AnswerNo,
    Unknown,
}

fn parse_text_message(data: &String) -> TextMessage {
    if data.starts_with("/join") {
        let split: Vec<_> = data.splitn(2, ' ').collect();
        if split.len() == 2 {
            return TextMessage::Join((*split.get(1).expect("should not happen")).to_string());
        }
    }

    if data == "/question" {
        return TextMessage::NextQuestion;
    }

    if data == "/score" {
        return TextMessage::GetScore;
    }

    if data == "/currentplayer" {
        return TextMessage::CurrentPlayer;
    }

    if data == BEGIN_CMD {
        return TextMessage::StartGame;
    }

    return TextMessage::JustMessage(data.clone());
}

fn parse_callback(data: &String) -> CallbackMessage {
    if data.starts_with("/question") {
        let data = data.trim_left_matches("/question");
        let split: Vec<_> = data.rsplitn(2, '_').collect();
        if split.len() == 2 {
            let cost = split.get(0).expect("should not happen");
            let topic = split.get(1).expect("should not happen");
            if let Ok(cost) = cost.parse::<usize>() {
                return CallbackMessage::SelectedQuestion(topic.to_string(), cost);
            } else {
                return CallbackMessage::Unknown;
            }
        } else {
            return CallbackMessage::Unknown;
        }
    }
    if data == ANSWER_YES {
        return CallbackMessage::AnswerYes;
    }

    if data == ANSWER_NO {
        return CallbackMessage::AnswerNo;
    }

    CallbackMessage::Unknown
}


fn main() {
    let mut core = Core::new().unwrap();
    let state = Arc::new(Mutex::new(State::WaitingForPlayersToJoin));
    let token = env::var("TELEGRAM_BOT_TOKEN").unwrap();
    let api = Api::configure(token.clone()).build(core.handle()).unwrap();

    let admin_user: UserId = UserId::from(ADMIN_ID);

    // Fetch new updates via long poll method
    let (sender, receiver) = mpsc::channel::<Option<Timeout>>(1);

    let handle = core.handle();
    let timeout_stream = timeout_stream::TimeoutStream::new(receiver);
    let updates_stream = api.stream();
    let requests_stream = merge_updates_and_timeouts(updates_stream, timeout_stream);

    let mut gamestate = gamestate::GameState::new(admin_user);

    println!("res");
    let fut = requests_stream.then(|res| {
        println!("udpate");
        match res {
            Ok(res) => {
                Ok(res)
            }
            Err(err) => {
                println!("err: {}", err);
                Ok(Err(()))
            }
        }
    }).for_each(move |request| {
        println!("here");
        let res = match request {
            Ok(telegram_update) => {
                match telegram_update.kind {
                    UpdateKind::Message(message) => {
                        if let MessageKind::Text { ref data, .. } = message.kind {
                            match parse_text_message(data) {
                                TextMessage::Join(name) => {
                                    gamestate.add_player(message.from.id, name)
                                }
                                TextMessage::JustMessage(text_msg) => {
                                    gamestate.message(message.from.id, text_msg)
                                }
                                TextMessage::NextQuestion => {
                                    gamestate.next_question(message.from.id)
                                }
                                TextMessage::StartGame => {
                                    gamestate.start(message.from.id)
                                }
                                TextMessage::GetScore => {
                                    gamestate.get_score(message.from.id)
                                }
                                TextMessage::CurrentPlayer => {
                                    gamestate.current_player(message.from.id)
                                }
                            }
                        } else {
                            vec![]
                        }
                    }
                    // TODO(stash): better matching
                    UpdateKind::CallbackQuery(callback) => {
                        let data = callback.data;
                        match parse_callback(&data) {
                            CallbackMessage::SelectedQuestion(topic, cost) => {
                                gamestate.select_question(topic, cost, callback.from.id)
                            }
                            CallbackMessage::AnswerYes => {
                                gamestate.yes_reply(callback.from.id)
                            }
                            CallbackMessage::AnswerNo => {
                                gamestate.no_reply(callback.from.id)
                            }
                            CallbackMessage::Unknown => {
                                vec![]
                            }
                        }
                    }
                    _ => vec![],
                }
            }
            Err(_timeout) => {
                gamestate.timeout()
            },
        };

        let res_future: Result<_, Error> = Ok(());
        let mut res_future: Box<Future<Item=(), Error=Error>> = Box::new(res_future.into_future());
        for r in res {
            match r {
                gamestate::UiRequest::SendTextToMainChat(msg) => {
                    let msg = SendMessage::new(ChatId::from(GAME_CHAT_ID), msg);
                    api.spawn(msg);
                }
                gamestate::UiRequest::Timeout(duration) => {
                    let timer = Timeout::new(duration, &handle).expect("cannot create timer");
                    let send_fut = sender.clone().send(Some(timer))
                        .map(|_| ())
                        .map_err(|err| err_msg(format!("{}", err)));

                    res_future = Box::new(res_future.and_then(|_| send_fut));
                }
                gamestate::UiRequest::ChooseQuestion(player_name, available_questions) => {
                    let msg = format!("{} {}", player_name, CHOOSE_QUESTION);
                    let mut msg = SendMessage::new(ChatId::from(GAME_CHAT_ID), msg);
                    let inline_keyboard = question_inline_keyboard(&available_questions);
                    let msg = msg.reply_markup(inline_keyboard);
                    api.spawn(msg);
                }
                gamestate::UiRequest::AskAdminYesNo(question) => {
                    let chat = ChatId::from(ADMIN_ID);
                    let inline_keyboard = reply_markup!(inline_keyboard,
                        ["Yes" callback ANSWER_YES, "No" callback ANSWER_NO]
                    );
                    let mut msg = SendMessage::new(chat, question);
                    let msg = msg.reply_markup(inline_keyboard);
                    api.spawn(msg);
                }
                gamestate::UiRequest::SendToAdmin(msg) => {
                    let msg = SendMessage::new(ChatId::from(ADMIN_ID), msg);
                    api.spawn(msg);
                }
                gamestate::UiRequest::StopTimer => {
                    let send_fut = sender.clone().send(None).map(|_| ())
                        .map_err(|err| err_msg(format!("{}", err)));
                    res_future = Box::new(res_future.and_then(|_| send_fut));
                }
            }
        }

        res_future
    });
    core.run(fut).expect("unexpected error");

    let api = Api::configure(token).build(core.handle()).unwrap();
    let future = api.stream().for_each({
        let mut players: Vec<Player> = vec![];
        // Incrementing global_timer_id means that all other timers should exit and have no side effects
        let mut global_timer_id: usize = 0;
        let mut global_yes_no_button_id: usize = 0;
        let mut current_player: Option<UserId> = None;
        let mut available_questions: BTreeSet<usize> = [1, 2, 3, 4, 5].iter().cloned().collect();
        let mut players_already_answered: BTreeSet<UserId> = BTreeSet::new();
 
        move |update| {
            match update.kind {
                UpdateKind::Message(message) => if let MessageKind::Text { ref data, .. } =
                    message.kind
                {
                    // TODO(stash): disallow same player answer twice
                    // TODO(stash): What if we are in the middle of the question?
                    if data == "/score" {
                        let mut res = String::new();
                        for player in players.iter() {
                            res.push_str(&format!("{}: {}\n", player.name(), player.score()));
                        }
                        api.spawn(message.text_reply(res));
                        return Ok(());
                    } else if data == "/currentplayer" {
                        let name = match current_player {
                            Some(id) => {
                                match find_player(&mut players, id) {
                                    Some(player) => player.name().clone(),
                                    None => "current player not found".to_string(),
                                }
                            },
                            None => "no current player".to_string()
                        };
                        api.spawn(message.text_reply(name));
                        return Ok(());
                    } else if data == "/checkbutton" {
                        let chat = ChatId::from(GAME_CHAT_ID);
                        let msg = SendMessage::new(chat, CHECK_BUTTON_MSG);
                        api.spawn(msg);
                        start_timer(state.clone(), &mut global_timer_id, GAME_CHAT_ID, timer_full_question);
                        return Ok(());
                    }

                    let state_val = { state.lock().unwrap().clone() };
                    match state_val {
                        State::WaitingForPlayersToJoin => if data.starts_with("/join") {
                            let split: Vec<_> = data.splitn(2, ' ').collect();
                            if split.len() != 2 {
                                api.spawn(message.text_reply(BAD_NAME));
                            } else {
                                if find_player(&mut players, message.from.id).is_some() {
                                    api.spawn(message.text_reply(PLAYER_ALREADY_ADDED));
                                } else {
                                    let name = (*split.get(1).expect("should not happen")).to_string();
                                    let name = name.trim().to_string();
                                    if players.iter().find(|player| player.name() == &name).is_some() {
                                        api.spawn(message.text_reply(NAME_TAKEN));
                                    } else {
                                        players.push(Player::new(name.clone(), message.from.id));
                                        api.spawn(message.text_reply(format!("Привет {}", name)));
                                    }
                                }
                            }
                        } else if message.from.id == admin_user && data == BEGIN_CMD {
                            let chat = ChatId::from(GAME_CHAT_ID);
                            if players.is_empty() {
                                let msg = SendMessage::new(chat, NO_PLAYERS);
                                api.spawn(msg);
                            } else {
                                let mut state = state.lock().expect("poisoned lock");
                                current_player = Some(players.get(0).expect("should be non-empty").id());
                                *state = State::WaitingForQuestion;
                            }
                        },
                        State::Falsestart => {
                            api.spawn(message.text_reply(FALSESTART_MSG));
                        }
                        State::CanAnswer(_) => {
                            // TODO(stash): check that only registered players can answer
                            if players_already_answered.contains(&message.from.id) {
                                println!("player that already answered tried to answer");
                                return Ok(());
                            }

                            {
                                let mut state = state.lock().expect("poisoned lock");
                                *state = State::Answering;
                            }
                            current_player = Some(message.from.id);
                            players_already_answered.insert(message.from.id);

                            {
                                let callback_yes = format!("{}{}", ANSWER_YES, global_yes_no_button_id);
                                let callback_no = format!("{}{}", ANSWER_NO, global_yes_no_button_id);
                                let inline_keyboard = reply_markup!(inline_keyboard,
                                    ["Yes" callback callback_yes, "No" callback callback_no]
                                );

                                let chat = ChatId::from(ADMIN_ID);
                                let mut msg = SendMessage::new(chat, format!("Correct answer?"));
                                let msg = msg.reply_markup(inline_keyboard);
                                api.spawn(msg);
                            }

                            api.spawn(message.text_reply(
                                format!("Ваш ответ {}?", message.from.first_name),
                            ));
                        }
                        State::WaitingForQuestion => {
                            if message.from.id == admin_user && data == "/question" {
                                let chat = ChatId::from(GAME_CHAT_ID);

                                let mut msg = SendMessage::new(chat, CHOOSE_QUESTION);
                                let inline_keyboard = question_inline_keyboard_old(&available_questions, 100);
                                let msg = msg.reply_markup(inline_keyboard);
                                api.spawn(msg);

                            }
                        }
                        State::Answering => {}
                    }
                },
                // TODO(stash): better matching
                UpdateKind::CallbackQuery(callback) => {

                    let state_val = { state.lock().unwrap().clone() };
                    let game_chat = ChatId::from(GAME_CHAT_ID);
                    let data = callback.data;
                    // TODO(stash): check that only new button is used
                    // TODO(stash): check that state is answering
                    // TODO(stash): check that admin send the message
                    // TODO(stash): check that current player exist
                    if current_player.is_none() {
                        println!("current player is not set!");
                        return Ok(());
                    }
                    let current_user_id = current_player.clone().expect("current user is None");
                    if data.starts_with(ANSWER_YES) {
                        let button_id = data.trim_left_matches(ANSWER_YES);
                        match button_id.parse::<usize>() {
                            Ok(button_id) => {
                                if button_id != global_yes_no_button_id {
                                    println!("old button was clicked");
                                    return Ok(());
                                }
                            }
                            Err(err) => {
                                println!("failed to parse button id: {}", err);
                                return Ok(());
                            }
                        };

                        match find_player(&mut players, current_user_id) {
                            Some(player) => {
                                player.update_score(10);
                                global_yes_no_button_id += 1;
                                let msg = SendMessage::new(game_chat, CORRECT_ANSWER);
                                api.spawn(msg);
                                players_already_answered = BTreeSet::new();
                                let mut state = state.lock().expect("poisoned lock");
                                *state = State::WaitingForQuestion;
                            }
                            None => {
                                println!("player not found");
                            }
                        }
                    } else if data.starts_with(ANSWER_NO) {
                        let button_id = data.trim_left_matches(ANSWER_NO);
                        match button_id.parse::<usize>() {
                            Ok(button_id) => {
                                if button_id != global_yes_no_button_id {
                                    println!("old button was clicked");
                                    return Ok(());
                                }
                            }
                            Err(err) => {
                                println!("failed to parse button id: {}", err);
                                return Ok(());
                            }
                        };

                        match find_player(&mut players, current_user_id) {
                            Some(player) => {
                                player.update_score(-10);
                                global_yes_no_button_id += 1;
                            }
                            None => {
                                println!("player not found");
                            }
                        }
                        start_timer(state.clone(), &mut global_timer_id, GAME_CHAT_ID, timer_question_after_incorrect_answer);

                    // TODO(stash): check question id
                    } else if data.starts_with("question") {
                        if callback.from.id == current_user_id && state_val == State::WaitingForQuestion {
                            let val = data.trim_left_matches("question").parse::<usize>()
                                .map_err(|err| format!("cannot parse question num{}", err))
                                .and_then(|question_num| {
                                    if available_questions.remove(&question_num) {
                                        Ok(())
                                    } else {
                                        Err("bad question num".to_string())
                                    }
                                })
                                .map(|_| {
                                    let chat = ChatId::from(GAME_CHAT_ID);
                                    let msg = SendMessage::new(chat, "2*2 = ?");
                                    api.spawn(msg);
                                    start_timer(state.clone(), &mut global_timer_id, GAME_CHAT_ID, timer_full_question);
                                });
                            match val {
                                Ok(_) => {}
                                Err(err) => {
                                    println!("{}", err);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }

            Ok(())
        }
    });

    core.run(future).unwrap();
}
