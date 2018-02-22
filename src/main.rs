extern crate csv;
extern crate failure;
extern crate futures;
#[macro_use]
extern crate telegram_bot;
#[macro_use]
extern crate serde_derive;
extern crate tokio_core;

use std::env;

use failure::{err_msg, Error};
use futures::{Future, IntoFuture, Sink, Stream};
use futures::sync::mpsc;
use futures::future::Map;
use futures::finished;
use tokio_core::reactor::{Core, Timeout};

use telegram_bot::{Api, InlineKeyboardMarkup, InlineKeyboardButton, MessageKind};
use telegram_bot::{SendMessage, Update, UpdateKind, UpdatesStream};

use std::process::Command;

mod gamestate;
mod messages;
mod player;
mod timeout_stream;
mod question;
mod telegram_config;
mod questionsstorage;

use messages::*;
use questionsstorage::{CsvQuestionsStorage, QuestionsStorage};

const TOKEN_VAR: &str = "TELEGRAM_BOT_TOKEN";
const CONFIG_VAR: &str = "GAME_CONFIG";

const ANSWER_YES: &str = "AnswerYes";
const ANSWER_NO: &str = "AnswerNo";

fn send_photo_via_curl() -> Box<Future<Item = (), Error = Error>> {
    // curl -F chat_id="-303858504" -F photo="@result.png"
    // https://api.telegram.org/bot521483445:AAEUuf-U2xTkKxjPtUawncpyEGZXEGHaddI/sendPhoto
    Command::new("curl").arg("-F").arg("chat_id=-303858504").arg("-F").arg("photo=@tmp.png").arg("https://api.telegram.org/bot521483445:AAEUuf-U2xTkKxjPtUawncpyEGZXEGHaddI/sendPhoto").status().expect("Failed to send table score");
    finished(()).boxed()
}

fn topics_inline_keyboard(topics: Vec<String>) -> InlineKeyboardMarkup {
    let mut inline_markup = InlineKeyboardMarkup::new();
    {
        for topic in topics {
            let data = format!("/topic{}", topic);
            let row = inline_markup.add_empty_row();
            row.push(InlineKeyboardButton::callback(format!("{}", topic), data));
        }
    }
    inline_markup
}

fn questioncosts_inline_keyboard(topic: String, costs: Vec<usize>) -> InlineKeyboardMarkup {
    let mut inline_markup = InlineKeyboardMarkup::new();
    {
        for cost in costs {
            let data = format!("/question{}_{}", topic, cost);
            let row = inline_markup.add_empty_row();
            row.push(InlineKeyboardButton::callback(format!("{}", cost), data));
        }
    }
    inline_markup
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
    NextTour,
}

enum CallbackMessage {
    SelectedTopic(String),
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

    if data == "/nexttour" {
        return TextMessage::NextTour;
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

    if data.starts_with("/topic") {
        let data = data.trim_left_matches("/topic");
        return CallbackMessage::SelectedTopic(data.into());
    }

    if data == ANSWER_YES {
        return CallbackMessage::AnswerYes;
    }

    if data == ANSWER_NO {
        return CallbackMessage::AnswerNo;
    }

    CallbackMessage::Unknown
}


fn convert_future<I, E, F>(future: F) -> Box<Future<Item = (), Error = Error>>
where
    F: Future<Item = I, Error = E> + 'static,
    E: std::fmt::Display,
{
    Box::new(future.map(|_| ()).map_err(|err| {
        let msg = format!("error happened: {}", err);
        err_msg(msg)
    }))
}

fn main() {
    let mut core = Core::new().unwrap();
    let token = env::var(TOKEN_VAR).unwrap();
    let api = Api::configure(token.clone()).build(core.handle()).unwrap();

    let config = telegram_config::Config::new(env::var(CONFIG_VAR).ok());

    // Fetch new updates via long poll method
    let (sender, receiver) = mpsc::channel::<Option<Box<Future<Item = (), Error = Error>>>>(1);

    let handle = core.handle();
    let timeout_stream = timeout_stream::TimeoutStream::new(receiver);
    let updates_stream = api.stream();
    let requests_stream = merge_updates_and_timeouts(updates_stream, timeout_stream);

    eprintln!("Game is ready to start!");
    let question_storage: Box<QuestionsStorage> = Box::new(
        CsvQuestionsStorage::new(config.questions_storage_path.clone()).expect("cannot open questions storage")
    );
    let mut gamestate = gamestate::GameState::new(
        config.admin_user,
        question_storage,
        config.questions_per_topic,
        config.tours.clone(),
    ).expect("failed to create gamestate");

    let fut = requests_stream.for_each(move |request| {
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
                                TextMessage::NextTour => {
                                    gamestate.next_tour(message.from.id)
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
                            CallbackMessage::SelectedTopic(topic) => {
                                gamestate.select_topic(topic, callback.from.id)
                            }
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
            let fut = match r {
                gamestate::UiRequest::SendTextToMainChat(msg) => {
                    let msg = SendMessage::new(config.game_chat, msg);
                    convert_future(api.send(msg))
                }
                gamestate::UiRequest::SendTextToMainChatWithDelay(msg, delay) => {
                    let msg = SendMessage::new(config.game_chat, msg);
                    let timeout = Timeout::new(delay, &handle).expect("cannot create timer");
                    let sendfut = api.send(msg).map_err(|_| err_msg("send failed"));
                    let fut = timeout.map_err(|_| err_msg("timeout error")).and_then(|_| sendfut);
                    convert_future(fut)
                }
                gamestate::UiRequest::Timeout(msg, duration) => {
                    let timer = Timeout::new(duration, &handle).expect("cannot create timer");
                    let timer = timer.map_err(|_err| err_msg("timer error happened"));
                    let timer_and_msg = match msg {
                        Some(msg) => {
                            let msg = SendMessage::new(config.game_chat, msg);
                            let sendfut = api.send(msg).map_err(
                                |err| {
                                    let msg = format!("send msg after timeout failed {:?}", err);
                                    err_msg(msg)
                                }
                            ).map(|_| ());
                            let res: Box<Future<Item = (), Error = Error>> = Box::new(
                                timer.and_then(|_| sendfut)
                            );
                            res
                        }
                        None => {
                            let res: Box<Future<Item = (), Error = Error>> = Box::new(timer);
                            res
                        }
                    };

                    convert_future(sender.clone().send(Some(timer_and_msg)))
                }
                gamestate::UiRequest::ChooseTopic(current_player_name, topics) => {
                    let mut msg = SendMessage::new(
                        config.game_chat,
                        format!("{}, выберите тему", current_player_name)
                    );
                    let inline_keyboard = topics_inline_keyboard(topics);
                    msg.reply_markup(inline_keyboard);
                    let fut = api.send(msg);
                    convert_future(fut)
                }
                gamestate::UiRequest::ChooseQuestion(topic, costs) => {
                    let mut msg = SendMessage::new(
                        config.game_chat,
                        "Выберите цену".to_string(),
                    );
                    let inline_keyboard = questioncosts_inline_keyboard(topic, costs);
                    msg.reply_markup(inline_keyboard);
                    let fut = api.send(msg);
                    convert_future(fut)
                }
                gamestate::UiRequest::AskAdminYesNo(question) => {
                    let inline_keyboard = reply_markup!(inline_keyboard,
                        ["Yes" callback ANSWER_YES, "No" callback ANSWER_NO]
                    );
                    let mut msg = SendMessage::new(config.admin_chat, question);
                    let msg = msg.reply_markup(inline_keyboard);
                    convert_future(api.send(msg))
                }
                gamestate::UiRequest::SendToAdmin(msg) => {
                    let msg = SendMessage::new(config.admin_chat, msg);
                    convert_future(api.send(msg))
                }
                gamestate::UiRequest::StopTimer => {
                    convert_future(sender.clone().send(None))
                }
                gamestate::UiRequest::SendPhoto(filename) => {
                    send_photo_via_curl()
                }
            };
            res_future = convert_future(res_future.and_then(|_| fut));
        }

        res_future
    });
    core.run(fut).expect("unexpected error");
}
