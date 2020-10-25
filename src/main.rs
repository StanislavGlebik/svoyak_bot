extern crate csv;
extern crate failure;
extern crate futures;
#[macro_use]
extern crate telegram_bot;
#[macro_use]
extern crate serde_derive;
extern crate futures_cpupool;
extern crate serde;
extern crate serde_json;
extern crate tokio_core;

use std::env;

use failure::{err_msg, Error};
use futures::sync::mpsc;
use futures::{Future, IntoFuture, Sink, Stream};
use futures_cpupool::CpuPool;
use std::fs::File;
use std::io::prelude::*;
use std::process::Command;
use std::time::Duration;
use tokio_core::reactor::{Core, Timeout};

use telegram_bot::{Api, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, MessageKind};
use telegram_bot::{SendMessage, Update, UpdateKind, UpdatesStream};

mod gamestate;
mod messages;
mod player;
mod question;
mod questionsstorage;
mod telegram_config;
mod timeout_stream;

use messages::*;
use questionsstorage::{CsvQuestionsStorage, QuestionsStorage};

const TOKEN_VAR: &str = "TELEGRAM_BOT_TOKEN";
const CONFIG_VAR: &str = "GAME_CONFIG";

const ANSWER_YES: &str = "AnswerYes";
const ANSWER_NO: &str = "AnswerNo";

const SCORE_TABLE_JSON_FILE: &str = "score_table.json";
const SCORE_TABLE_PNG_FILE: &str = "score_table.png";

fn dump_score_table_file(table: gamestate::ScoreTable, filename: &str) -> Result<(), Error> {
    let mut file = File::create(filename).map_err(|error| {
        err_msg(format!(
            "Can't create file to dump score table ({:?})",
            error
        ))
    })?;
    let data = serde_json::to_string(&table).map_err(|error| {
        err_msg(format!(
            "Failed while serializing score table ({:?})",
            error
        ))
    })?;
    file.write_all(data.as_bytes()).map_err(|error| {
        err_msg(format!(
            "Can't write to file while dumping score table ({:?})",
            error
        ))
    })
}

fn make_score_table_image(table_filename: &str, image_filename: &str) -> Result<(), Error> {
    let status = Command::new("python3")
        .arg("external/draw_table.py")
        .arg(table_filename)
        .arg(image_filename)
        .status()
        .map_err(|error| {
            err_msg(format!(
                "Can't execute process to draw score table ({:?})",
                error
            ))
        })?;
    if !status.success() {
        Err(err_msg(
            "Process drawing score table finished unsucessfully",
        ))
    } else {
        Ok(())
    }
}

fn send_photo_via_curl(game_chat: ChatId, token: &str, filename: &str) -> Result<(), Error> {
    let status = Command::new("curl")
        .arg("-F")
        .arg(format!("chat_id={}", game_chat))
        .arg("-F")
        .arg(format!("photo=@{}", filename))
        .arg(format!("https://api.telegram.org/bot{}/sendPhoto", token))
        .status()
        .map_err(|error| {
            err_msg(format!(
                "Can't execute curl to send score table ({:?})",
                error
            ))
        })?;
    if !status.success() {
        Err(err_msg("Curl sending score table finished unsucessfully"))
    } else {
        Ok(())
    }
}

fn send_score_table(
    pool: &CpuPool,
    table: gamestate::ScoreTable,
    game_chat: ChatId,
    token: String,
) -> Box<dyn Future<Item = (), Error = Error>> {
    Box::new(pool.spawn_fn(move || {
        dump_score_table_file(table, SCORE_TABLE_JSON_FILE)?;
        make_score_table_image(SCORE_TABLE_JSON_FILE, SCORE_TABLE_PNG_FILE)?;
        send_photo_via_curl(game_chat, &token, SCORE_TABLE_PNG_FILE)?;
        Ok(())
    }))
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
) -> Box<dyn Stream<Item = Result<Update, ()>, Error = Error>> {
    let updates_stream = Box::new(
        updates_stream
            .map(|update| Ok(update))
            .map_err(|err| err_msg(format!("{}", err))),
    );

    let timeouts = Box::new(
        timeouts
            .map(|timeout| Err(timeout))
            .map_err(|err| err_msg(format!("{}", err))),
    );
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
    UpdateScore(String, i64),
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

    if data.starts_with("/updatescore ") {
        let data = data.trim_start_matches("/updatescore ");
        let split: Vec<_> = data.rsplitn(2, ' ').collect();
        if split.len() == 2 {
            let name = split.get(1).unwrap();
            let newscore = split.get(0).unwrap();
            let score = newscore.parse();
            if let Ok(score) = score {
                return TextMessage::UpdateScore((*name).into(), score);
            }
        }
    }

    if data == BEGIN_CMD {
        return TextMessage::StartGame;
    }

    return TextMessage::JustMessage(data.clone());
}

fn parse_callback(data: &String) -> CallbackMessage {
    if data.starts_with("/question") {
        let data = data.trim_start_matches("/question");
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
        let data = data.trim_start_matches("/topic");
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

fn convert_future<I, E, F>(future: F) -> Box<dyn Future<Item = (), Error = Error>>
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
    let config = telegram_config::Config::new(env::var(CONFIG_VAR).ok(), token);
    let api = Api::configure(&config.token).build(core.handle()).unwrap();

    let thread_pool = CpuPool::new(2);

    // Fetch new updates via long poll method
    let (sender, receiver) = mpsc::channel::<Option<Box<dyn Future<Item = (), Error = Error>>>>(1);

    let handle = core.handle();
    let timeout_stream = timeout_stream::TimeoutStream::new(receiver);
    let updates_stream = api.stream();
    let requests_stream = merge_updates_and_timeouts(updates_stream, timeout_stream);

    eprintln!("Game is ready to start!");
    let question_storage: Box<dyn QuestionsStorage> = Box::new(
        CsvQuestionsStorage::new(config.questions_storage_path.clone())
            .expect("cannot open questions storage"),
    );
    let mut gamestate = gamestate::GameState::new(
        config.admin_user,
        question_storage,
        config.questions_per_topic,
        config.tours.clone(),
        config.manual_questions.clone(),
    )
    .expect("failed to create gamestate");

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
                                TextMessage::StartGame => gamestate.start(message.from.id),
                                TextMessage::GetScore => gamestate.get_score(message.from.id),
                                TextMessage::CurrentPlayer => {
                                    gamestate.current_player(message.from.id)
                                }
                                TextMessage::NextTour => gamestate.next_tour(message.from.id),
                                TextMessage::UpdateScore(name, newscore) => {
                                    gamestate.update_score(name, newscore, message.from.id)
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
                            CallbackMessage::AnswerYes => gamestate.yes_reply(callback.from.id),
                            CallbackMessage::AnswerNo => gamestate.no_reply(callback.from.id),
                            CallbackMessage::Unknown => vec![],
                        }
                    }
                    _ => vec![],
                }
            }
            Err(_timeout) => gamestate.timeout(),
        };

        let res_future: Result<_, Error> = Ok(());
        let mut res_future: Box<dyn Future<Item = (), Error = Error>> =
            Box::new(res_future.into_future());
        for r in res {
            let fut = match r {
                gamestate::UiRequest::SendTextToMainChat(msg) => {
                    let msg = SendMessage::new(config.game_chat, msg);
                    convert_future(api.send(msg))
                }
                gamestate::UiRequest::Timeout(msg, delay) => {
                    let duration = match delay {
                        gamestate::Delay::Short => Duration::new(3, 0),
                        gamestate::Delay::Medium => Duration::new(5, 0),
                        gamestate::Delay::Long => Duration::new(10, 0),
                    };
                    let timer = Timeout::new(duration, &handle).expect("cannot create timer");
                    let timer = timer.map_err(|_err| err_msg("timer error happened"));
                    let timer_and_msg = match msg {
                        Some(msg) => {
                            let msg = SendMessage::new(config.game_chat, msg);
                            let sendfut = api
                                .send(msg)
                                .map_err(|err| {
                                    let msg = format!("send msg after timeout failed {:?}", err);
                                    err_msg(msg)
                                })
                                .map(|_| ());
                            let res: Box<dyn Future<Item = (), Error = Error>> =
                                Box::new(timer.and_then(|_| sendfut));
                            res
                        }
                        None => {
                            let res: Box<dyn Future<Item = (), Error = Error>> = Box::new(timer);
                            res
                        }
                    };

                    convert_future(sender.clone().send(Some(timer_and_msg)))
                }
                gamestate::UiRequest::ChooseTopic(current_player_name, topics) => {
                    let mut msg = SendMessage::new(
                        config.game_chat,
                        format!("{}, выберите тему", current_player_name),
                    );
                    let inline_keyboard = topics_inline_keyboard(topics);
                    msg.reply_markup(inline_keyboard);
                    let fut = api.send(msg);
                    convert_future(fut)
                }
                gamestate::UiRequest::ChooseQuestion(topic, costs) => {
                    let mut msg = SendMessage::new(config.game_chat, "Выберите цену".to_string());
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
                gamestate::UiRequest::StopTimer => convert_future(sender.clone().send(None)),
                gamestate::UiRequest::SendScoreTable(score_table) => {
                    let mut msg = SendMessage::new(
                        config.game_chat,
                        String::from("```\n") + &score_table.to_string() + "```",
                    );
                    let msg = msg.parse_mode(telegram_bot::ParseMode::Markdown);
                    let text_fallback: Box<dyn Future<Item = (), Error = Error>> =
                        convert_future(api.send(msg).map_err(|_| err_msg("send failed")));
                    convert_future(
                        send_score_table(
                            &thread_pool,
                            score_table,
                            config.game_chat,
                            config.token.clone(),
                        )
                        .then(|future| match future {
                            Ok(_) => Box::new(futures::done(Ok(()))),
                            Err(errmsg) => {
                                eprintln!("Couldn't send score table image: '{:?}'", errmsg);
                                text_fallback
                            }
                        }),
                    )
                }
            };
            res_future = convert_future(res_future.and_then(|_| fut));
        }

        res_future
    });
    core.run(fut).expect("unexpected error");
}
