use std::env;

use failure::{err_msg, Error};
use futures::sync::mpsc;
use futures::{Future, Sink, Stream};
use futures_03::{
    compat::{Future01CompatExt, Stream01CompatExt},
    FutureExt, StreamExt, TryFutureExt, TryStreamExt,
};
use std::fs::File;
use std::io::prelude::*;
use std::process::Command;
use std::time::{Duration, Instant};
use telegram_bot::{ParseMode, reply_markup};
use tokio as tokio_01;
use tokio_compat::runtime::Runtime;

use telegram_bot::{Api, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, MessageKind};
use telegram_bot::{SendMessage, Update, UpdateKind, UpdatesStream};

mod gamestate;
mod messages;
mod player;
mod question;
mod questionsstorage;
mod stickers;
mod telegram_config;
mod timeout_stream;

use gamestate::TopicIdx;
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

fn send_audio_via_curl(game_chat: ChatId, token: &str, filename: &str) -> Result<(), Error> {
    let status = Command::new("curl")
        .arg("-F")
        .arg(format!("chat_id={}", game_chat))
        .arg("-F")
        .arg(format!("audio=@{}", filename))
        .arg(format!("https://api.telegram.org/bot{}/sendAudio", token))
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

fn send_sticker_via_curl(game_chat: ChatId, token: &str, file_id: &str) -> Result<(), Error> {
    let status = Command::new("curl")
        .arg("-F")
        .arg(format!("chat_id={}", game_chat))
        .arg("-F")
        .arg(format!("sticker={}", file_id))
        .arg(format!("https://api.telegram.org/bot{}/sendSticker", token))
        .status()
        .map_err(|error| {
            err_msg(format!(
                "Can't execute curl to send sticker ({:?})",
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
    table: gamestate::ScoreTable,
    game_chat: ChatId,
    token: String,
) -> Result<(), Error> {
    dump_score_table_file(table, SCORE_TABLE_JSON_FILE)?;
    make_score_table_image(SCORE_TABLE_JSON_FILE, SCORE_TABLE_PNG_FILE)?;
    send_photo_via_curl(game_chat, &token, SCORE_TABLE_PNG_FILE)?;
    Ok(())
}

fn topics_inline_keyboard(topics: Vec<(TopicIdx, String)>) -> InlineKeyboardMarkup {
    let mut inline_markup = InlineKeyboardMarkup::new();
    {
        for (idx, topic) in topics {
            let data = format!("/topic{}", idx.0);
            let row = inline_markup.add_empty_row();
            row.push(InlineKeyboardButton::callback(format!("{}", topic), data));
        }
    }
    inline_markup
}

fn questioncosts_inline_keyboard(topic_idx: TopicIdx, costs: Vec<usize>) -> InlineKeyboardMarkup {
    let mut inline_markup = InlineKeyboardMarkup::new();
    {
        for cost in costs {
            let data = format!("/question{}_{}", topic_idx.0, cost);
            let row = inline_markup.add_empty_row();
            row.push(InlineKeyboardButton::callback(format!("{}", cost), data));
        }
    }
    inline_markup
}

fn cat_in_bag_player_inline_keyboard(players: Vec<player::Player>) -> InlineKeyboardMarkup {
    let mut inline_markup = InlineKeyboardMarkup::new();
    for player in players {
        let data = format!("/cat_in_bag_choose_player_{}", player.name());
        let row = inline_markup.add_empty_row();
        row.push(InlineKeyboardButton::callback(player.name().to_string(), data))
    }
    inline_markup
}

fn cat_in_bag_cost_inline_keyboard(costs: Vec<usize>) -> InlineKeyboardMarkup {
    let mut inline_markup = InlineKeyboardMarkup::new();
    for cost in costs {
        let data = format!("/cat_in_bag_choose_cost_{}", cost);
        let row = inline_markup.add_empty_row();
        row.push(InlineKeyboardButton::callback(format!("{}", cost), data))
    }
    inline_markup
}

fn merge_updates_and_timeouts(
    updates_stream: UpdatesStream,
    timeouts: timeout_stream::TimeoutStream,
) -> Box<dyn Stream<Item = Result<Update, ()>, Error = Error>> {
    let updates_stream = Box::new(
        updates_stream
            .compat()
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
    ChangePlayer(String),
    NextTour,
    UpdateScore(String, i64),
    HideQuestion(String, usize),
    UpdateAuctionCost(String, usize),
}

enum CallbackMessage {
    SelectedTopic(TopicIdx),
    SelectedQuestion(TopicIdx, usize),
    AnswerYes,
    AnswerNo,
    Unknown,
    CatInBagPlayerChosen(String),
    CatInBagCostChosen(usize),
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

    if data.starts_with("/changeplayer") {
        let split: Vec<_> = data.splitn(2, ' ').collect();
        if split.len() == 2 {
            return TextMessage::ChangePlayer((*split.get(1).expect("should not happen")).to_string());
        }
    }

    if data.starts_with("/auction") {
        let split: Vec<_> = data.splitn(3, ' ').collect();
        if split.len() == 3 {
            if let Ok(cost) = split[2].parse() {
                return TextMessage::UpdateAuctionCost(split[1].to_string(), cost);
            }
        }
    }

    if data.starts_with("/hidequestion") {
        let data = data.trim_start_matches("/hidequestion ");
        let split: Vec<_> = data.splitn(2, ' ').collect();

        if split.len() == 2 {
            let cost = split.get(0).unwrap();
            let topic = split.get(1).unwrap();
            let cost = cost.parse();
            if let Ok(cost) = cost {
                return TextMessage::HideQuestion(topic.to_string(), cost);
            }
        }
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

fn parse_callback(data: &Option<String>) -> CallbackMessage {
    let data = match data {
        Some(data) => data,
        None => {
            return CallbackMessage::Unknown;
        }
    };
    if data.starts_with("/question") {
        let data = data.trim_start_matches("/question");
        let split: Vec<_> = data.rsplitn(2, '_').collect();
        if split.len() == 2 {
            let cost = split.get(0).expect("should not happen");
            let topic_idx = split.get(1).expect("should not happen");
            let topic_idx = match topic_idx.parse::<usize>() {
                Ok(topic_idx) => topic_idx,
                Err(_) => {
                    return CallbackMessage::Unknown;
                }
            };
            if let Ok(cost) = cost.parse::<usize>() {
                return CallbackMessage::SelectedQuestion(TopicIdx(topic_idx), cost);
            } else {
                return CallbackMessage::Unknown;
            }
        } else {
            return CallbackMessage::Unknown;
        }
    }

    if data.starts_with("/topic") {
        let data = data.trim_start_matches("/topic");
        let maybe_topic_idx = data.parse::<usize>();
        if let Ok(idx) = maybe_topic_idx {
            return CallbackMessage::SelectedTopic(TopicIdx(idx));
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

    if data.starts_with("/cat_in_bag_choose_player_") {
        let data = data.trim_start_matches("/cat_in_bag_choose_player_");
        return CallbackMessage::CatInBagPlayerChosen(data.to_string());
    }

    if data.starts_with("/cat_in_bag_choose_cost_") {
        let data = data.trim_start_matches("/cat_in_bag_choose_cost_");
        let maybe_cost = data.parse::<usize>();
        match maybe_cost {
            Ok(cost) => {
                return CallbackMessage::CatInBagCostChosen(cost);
            }
            Err(_) => {
                return CallbackMessage::Unknown;
            }
        }
    }

    CallbackMessage::Unknown
}

fn main() -> Result<(), Error> {
    let mut runtime = Runtime::new()?;
    let token = env::var(TOKEN_VAR).unwrap();
    let config = telegram_config::Config::new(env::var(CONFIG_VAR).ok(), token);
    let api = Api::new(&config.token);

    let game_chat = match config.game_chat {
        Some(game_chat) => {
            game_chat
        }
        None => {
            eprintln!("waiting to select a game chat");
            let mut s = api.stream();
            runtime.block_on_std(
                async {
                    while let Some(telegram_update) = s.try_next().await? {
                        if let UpdateKind::Message(message) = telegram_update.kind {
                            if let MessageKind::Text { ref data, .. } = message.kind {
                                if data == "/thischat" && message.from.id == config.admin_user {
                                    return Ok(message.chat.id());
                                }
                            }
                        }
                    }
                    Err(err_msg("unexpected exit"))
                }
            )?
        }
    };

    runtime.block_on_std(
        async {
            let msg = SendMessage::new(game_chat, "Для регистрации в игре введите '/join ИМЯ' без кавычек".to_string());
            api.send(msg).await?;
            Result::<_, Error>::Ok(())
        }
    )?;

    // Fetch new updates via long poll method
    let (sender, receiver) = mpsc::channel::<Option<Box<dyn Future<Item = (), Error = Error>>>>(1);

    let timeout_stream = timeout_stream::TimeoutStream::new(receiver);
    let updates_stream = api.stream();
    let requests_stream = merge_updates_and_timeouts(updates_stream, timeout_stream);

    eprintln!("Game is ready to start!");
    let question_storage = runtime.block_on_std(
        CsvQuestionsStorage::new(
            config.questions_storage_path.clone(),
        )
    )?;
    let question_storage: Box<dyn QuestionsStorage> = Box::new(question_storage);

    eprintln!("loaded questions");
    let mut gamestate = gamestate::GameState::new(
        config.admin_user,
        &question_storage,
        config.questions_per_topic,
    )?;
    eprintln!("created gamestate");

    let fut = async move {
        let mut s = requests_stream.compat();
        while let Some(request) = s.next().await {
            let request = match request {
                Ok(request) => request,
                Err(err) => {
                    println!("{}", err);
                    continue;
                }
            };
            let res = match request {
                Ok(telegram_update) => {
                    match telegram_update.kind {
                        UpdateKind::Message(message) => {
                            println!("message chat id {}", message.chat.id());
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
                                    TextMessage::ChangePlayer(player) => {
                                        gamestate.change_player(message.from.id, player)
                                    }
                                    TextMessage::NextTour => gamestate.next_tour(message.from.id),
                                    TextMessage::UpdateScore(name, newscore) => {
                                        gamestate.update_score(name, newscore, message.from.id)
                                    }
                                    TextMessage::HideQuestion(topic, cost) => {
                                        gamestate.hide_question(topic, cost, message.from.id)
                                    }
                                    TextMessage::UpdateAuctionCost(user, cost) => {
                                        gamestate.update_auction_cost(message.from.id, user, cost)
                                    }
                                }
                            } else if let  MessageKind::Sticker { ref data } = message.kind {
                                eprintln!("sticker: {}", data.file_id);
                                vec![]
                            } else {
                                vec![]
                            }
                        }
                        // TODO(stash): better matching
                        UpdateKind::CallbackQuery(callback) => {
                            let data = callback.data;
                            match parse_callback(&data) {
                                CallbackMessage::SelectedTopic(topic_id) => {
                                    gamestate.select_topic(topic_id, callback.from.id)
                                }
                                CallbackMessage::SelectedQuestion(topic_idx, cost) => {
                                    gamestate.select_question(topic_idx, cost, callback.from.id, &question_storage)
                                }
                                CallbackMessage::AnswerYes => gamestate.yes_reply(callback.from.id),
                                CallbackMessage::AnswerNo => gamestate.no_reply(callback.from.id),
                                CallbackMessage::CatInBagPlayerChosen(player) => {
                                    gamestate.select_cat_in_bag_player(callback.from.id, player)
                                }
                                CallbackMessage::CatInBagCostChosen(cost) => {
                                    gamestate.select_cat_in_bag_cost(callback.from.id, cost)
                                }
                                CallbackMessage::Unknown => vec![],
                            }
                        }
                        _ => vec![],
                    }
                }
                Err(_timeout) => gamestate.timeout(),
            };

            for r in res {
                match r {
                    gamestate::UiRequest::SendTextToMainChat(msg) => {
                        let msg = SendMessage::new(game_chat, msg);
                        api.send(msg).await?;
                    }
                    gamestate::UiRequest::SendHtmlToMainChat(msg) => {
                        let mut msg = SendMessage::new(game_chat, msg);
                        msg.parse_mode(ParseMode::Html);
                        api.send(msg).await?;
                    }
                    gamestate::UiRequest::SendSticker(sticker) => {
                        let r = send_sticker_via_curl(game_chat, &config.token, &sticker);
                        if let Err(e) = r {
                            eprintln!("was not able to send sticker {}!", e);
                        }
                    }
                    gamestate::UiRequest::SendImage(image) => {
                        let r = send_photo_via_curl(game_chat, &config.token, &image.to_string_lossy());
                        if let Err(e) = r {
                            eprintln!("was not able to send image {}!", e);
                        }
                    }
                    gamestate::UiRequest::SendAudio(audio) => {
                        let r = send_audio_via_curl(game_chat, &config.token, &audio.to_string_lossy());
                        if let Err(e) = r {
                            eprintln!("was not able to send audio {}!", e);
                        }
                    }
                    gamestate::UiRequest::Timeout(msg, delay) => {
                        let duration = match delay {
                            gamestate::Delay::Short => Duration::new(3, 0),
                            gamestate::Delay::Medium => Duration::new(5, 0),
                            gamestate::Delay::Long => Duration::new(10, 0),
                            gamestate::Delay::ExtraLong => Duration::new(15, 0),
                        };

                        let when = Instant::now() + duration;
                        let timer = tokio_01::timer::Delay::new(when);
                        let timer = timer.map_err(|_err| err_msg("timer error happened"));
                        let timer_and_msg = match msg {
                            Some(msg) => {
                                let msg = SendMessage::new(game_chat, msg);
                                let sendfut = api
                                    .send(msg)
                                    .boxed()
                                    .compat()
                                    .map_err(|err| {
                                        let msg =
                                            format!("send msg after timeout failed {:?}", err);
                                        err_msg(msg)
                                    })
                                    .map(|_| ());
                                let res: Box<dyn Future<Item = (), Error = Error> + Send> =
                                    Box::new(timer.and_then(|_| sendfut));
                                res
                            }
                            None => {
                                let res: Box<dyn Future<Item = (), Error = Error> + Send> =
                                    Box::new(timer);
                                res
                            }
                        };

                        // TODO(stash): handle?
                        let _ = sender.clone().send(Some(timer_and_msg)).compat().map_err(|_|()).await;
                    }
                    gamestate::UiRequest::ChooseTopic(current_player_name, topics) => {
                        let mut msg = SendMessage::new(
                            game_chat,
                            format!("{}, выберите тему", current_player_name),
                        );
                        let inline_keyboard = topics_inline_keyboard(topics);
                        msg.reply_markup(inline_keyboard);
                        api.send(msg).await?;
                    }
                    gamestate::UiRequest::ChooseQuestion(topic_idx, topic, costs) => {
                        let mut msg =
                            SendMessage::new(game_chat, format!("Выбрана тема '{}', выберите цену", topic));
                        let inline_keyboard = questioncosts_inline_keyboard(topic_idx, costs);
                        msg.reply_markup(inline_keyboard);
                        api.send(msg).await?;
                    }
                    gamestate::UiRequest::AskAdminYesNo(question) => {
                        let inline_keyboard = reply_markup!(inline_keyboard,
                            ["Yes" callback ANSWER_YES, "No" callback ANSWER_NO]
                        );
                        let mut msg = SendMessage::new(config.admin_chat, question);
                        msg.reply_markup(inline_keyboard);
                        api.send(msg).await?;
                    }
                    gamestate::UiRequest::SendToAdmin(msg) => {
                        let msg = SendMessage::new(config.admin_chat, msg);
                        api.send(msg).await?;
                    }
                    gamestate::UiRequest::StopTimer => {
                        // TODO(stash): handle?
                        let _ = sender.clone().send(None).compat().map_err(|_| ()).await;
                    },
                    gamestate::UiRequest::SendScoreTable(score_table) => {
                        let score_table_str = score_table.to_string();
                        let res = match send_score_table(score_table, game_chat, config.token.clone())
                        {
                            Ok(_) => (),
                            Err(errmsg) => {
                                eprintln!("Couldn't send score table image: '{:?}'", errmsg);

                                let mut msg = SendMessage::new(
                                    game_chat,
                                    String::from("```\n") + &score_table_str + "```",
                                );
                                msg.parse_mode(telegram_bot::ParseMode::Markdown);
                                api.send(msg).await?;
                            }
                        };

                        res
                    }
                    gamestate::UiRequest::CatInBagChoosePlayer(players) => {
                        let inline_keyboard = cat_in_bag_player_inline_keyboard(players);
                        let mut msg = SendMessage::new(game_chat, "Кто играет?".to_string());
                        msg.reply_markup(inline_keyboard);
                        api.send(msg).await?;
                    }
                    gamestate::UiRequest::CatInBagChooseCost(costs) => {
                        let inline_keyboard = cat_in_bag_cost_inline_keyboard(costs);
                        let mut msg = SendMessage::new(game_chat, "Выберите ставку".to_string());
                        msg.reply_markup(inline_keyboard);
                        api.send(msg).await?;
                    }
                }
            }
        }
        Result::<_, Error>::Ok(())
    };

    runtime.block_on_std(fut)?;

    Ok(())
}
