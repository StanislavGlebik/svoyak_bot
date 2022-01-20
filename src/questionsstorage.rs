use csv;
use failure::{err_msg, Error};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use crate::question::Question;

pub trait QuestionsStorage {
    fn get(&self, topic_name: String, difficulty: usize) -> Option<Question>;

    fn get_tours(&self) -> Vec<TourDescription>;
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

// Questions for the same topic have to go one after another
// Row: question,answer,optional comment,topic
pub struct CsvQuestionsStorage {
    questions: HashMap<(String, usize), Question>,
    tours: Vec<TourDescription>,
}

impl CsvQuestionsStorage {
    // TODO(stash): skip header
    pub fn new<P: AsRef<Path>>(p: P) -> Result<Self, Error> {
        let dir = p.as_ref();
        eprintln!("{:?}", dir);
        let mut questions_storage = HashMap::new();

        let mut tours = vec![];
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
                        let question = Question::new(question, answer, comment);
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

        Ok(Self {
            questions: questions_storage,
            tours,
        })
    }
}

impl QuestionsStorage for CsvQuestionsStorage {
    fn get(&self, topic_name: String, difficulty: usize) -> Option<Question> {
        self.questions.get(&(topic_name, difficulty)).cloned()
    }

    fn get_tours(&self) -> Vec<TourDescription> {
        self.tours.clone()
    }
}
