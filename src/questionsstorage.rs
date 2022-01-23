use csv;
use failure::{err_msg, Error};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use crate::question::Question;

pub trait QuestionsStorage {
    fn get(&self, topic_name: String, difficulty: usize) -> Option<Question>;

    fn get_tours(&self) -> Vec<TourDescription>;

    fn get_cats_in_bags(&self) -> Vec<CatInBag>;

    fn get_manual_questions(&self) -> Vec<(String, usize)>;

    fn get_auctions(&self) -> Vec<(String, usize)>;
}

#[derive(Clone)]
pub struct Topic {
    pub name: String,
}

#[derive(Clone)]
pub struct TourDescription {
    pub multiplier: usize,
    pub topics: Vec<Topic>,
}

#[derive(Clone)]
pub struct CatInBag {
    pub old_topic: String,
    pub cost: usize,
    pub new_topic: String,
    pub question: String,
    pub answer: String,
}

// Questions for the same topic have to go one after another
// Row: question,answer,optional comment,topic
pub struct CsvQuestionsStorage {
    questions: HashMap<(String, usize), Question>,
    tours: Vec<TourDescription>,
    cats_in_bags: Vec<CatInBag>,
    manual_questions: Vec<(String, usize)>,
    auctions: Vec<(String, usize)>,
}

impl CsvQuestionsStorage {
    // TODO(stash): skip header
    pub fn new<P: AsRef<Path>>(p: P) -> Result<Self, Error> {
        let dir = p.as_ref();
        eprintln!("{:?}", dir);
        let mut questions_storage = HashMap::new();

        let mut tours = vec![];
        let mut cats_in_bags = vec![];
        let mut manual_questions = vec![];
        let mut auctions = vec![];
        let mut i = 1;
        loop {
            let multiplier = 100 * i;
            let file = dir.join(format!("tour{}.csv", i));
            if !file.exists() {
                break;
            }
            eprintln!("opening {:?}", file);

            let mut topics = vec![];

            let file = File::open(file)?;
            let mut reader = csv::ReaderBuilder::new()
                    .has_headers(false)
                    .from_reader(file);
            let mut current_topic: Option<String> = None;
            let mut current_difficulty = 0;

            for r in reader.records() {
                let record = r?;
                if record.len() < 4 {
                    let msg = format!("incorrect number of field: {} < 4", record.len());
                    return Err(err_msg(msg));
                }
                let topic = record.get(0).unwrap().to_string();
                // second field is cost, we ignore it here
                let question = record.get(2).unwrap();
                let answer = record.get(3).unwrap();
                let comment = record.get(4);
                let comment = if comment == Some(&"".to_string()) {
                    None
                } else {
                    comment
                };
                if topic == "" {
                    current_difficulty += 1;
                } else {
                    eprintln!("Topic {}", topic);
                    topics.push(Topic {
                        name: topic.clone()
                    });
                    current_topic = Some(topic.clone());
                    current_difficulty = 1;
                }
                match current_topic {
                    Some(ref current_topic) => {

                        let question = if let Some((cat_in_bag_topic, question)) = check_if_cat_in_bag(question.to_string())? {
                            let cat_in_bag = CatInBag {
                                old_topic: current_topic.clone(),
                                cost: current_difficulty * multiplier,
                                new_topic: cat_in_bag_topic,
                                question: question.clone(),
                                answer: answer.to_string(),
                            };
                            cats_in_bags.push(cat_in_bag);
                            Question::new(question, answer.to_string(), comment.map(|c| c.to_string()))
                        } else if let Some(question) = check_if_manual(question.to_string())? {
                            manual_questions.push((current_topic.clone(), current_difficulty * multiplier));
                            Question::new(question, answer.to_string(), comment.map(|c| c.to_string()))
                        } else if let Some(question) = check_if_auction(question.to_string())? {
                            auctions.push((current_topic.clone(), current_difficulty * multiplier));
                            Question::new(question, answer.to_string(), comment.map(|c| c.to_string()))
                        } else {
                            Question::new(question, &answer, comment)
                        };
                        questions_storage.insert((current_topic.clone(), current_difficulty), question);
                    }
                    None => {
                        return Err(err_msg("current topic is empty"));
                    }
                }
            }

            tours.push(TourDescription {
                multiplier,
                topics,
            });
            i += 1;
        }

        eprintln!("Found {} cats in bags", cats_in_bags.len());
        eprintln!("Found {} manual questions", manual_questions.len());
        eprintln!("Found {} auctions", auctions.len());

        Ok(Self {
            questions: questions_storage,
            tours,
            cats_in_bags,
            manual_questions,
            auctions,
        })
    }
}

fn check_if_cat_in_bag(question: String) -> Result<Option<(String, String)>, Error> {
    let question = question.trim();
    let cat_in_bag_prefix = "КОТ В МЕШКЕ";
    if question.starts_with(cat_in_bag_prefix) {
        let question = question.trim_start_matches(cat_in_bag_prefix).trim();
        let topic_prefix = "Тема: ";
        if !question.starts_with(topic_prefix) {
            return Err(err_msg("malformatted cat in bag"));
        }
        let question = question.trim_start_matches(topic_prefix);
        let split: Vec<_> = question.splitn(2, ".").collect();
        if split.len() != 2 {
            return Err(err_msg("malformatted cat in bag"));
        }

        let topic = split[0];
        let question = split[1];
        return Ok(Some((topic.to_string(), question.to_string())));
    }

    Ok(None)
}

fn check_if_manual(question: String) -> Result<Option<String>, Error> {
    let question = question.trim();
    let manual = "РУЧНОЙ";

    if question.starts_with(manual) {
        let question = question.trim_start_matches(manual).trim();
        return Ok(Some(question.to_string()));
    }

    return Ok(None);
}

fn check_if_auction(question: String) -> Result<Option<String>, Error> {
    let question = question.trim();
    let auction = "АУКЦИОН";

    if question.starts_with(auction) {
        let question = question.trim_start_matches(auction).trim();
        return Ok(Some(question.to_string()))
    }

    return Ok(None);
}

impl QuestionsStorage for CsvQuestionsStorage {
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
