
use csv;
use failure::{Error, err_msg};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use question::Question;

pub trait QuestionsStorage {
    fn get(&self, topic_name: String, difficulty: usize) -> Option<Question>;
}


// Questions for the same topic have to go one after another
// Row: question,answer,optional comment,topic
pub struct CsvQuestionsStorage {
    questions: HashMap<(String, usize), Question>
}

impl CsvQuestionsStorage {
    pub fn new<P: AsRef<Path>>(p: P) -> Result<Self, Error> {
        println!("{:?}", p.as_ref());
        let file = File::open(p)?;
        let mut reader = csv::Reader::from_reader(file);
        let mut current_topic: Option<String> = None;
        let mut current_difficulty = 0;

        let mut questions_storage = HashMap::new();
        for r in reader.records() {
            let record = r?;
            if record.len() != 4 {
                let msg = format!("incorrect number of field: {} != 4", record.len());
                return Err(err_msg(msg));
            }
            let question = record.get(0).unwrap();
            let answer = record.get(1).unwrap();
            // TODO(stash): ignore comments for now
            let topic = record.get(3).unwrap().clone();
            if current_topic != Some(topic.clone()) {
                current_topic = Some(topic.clone());
                current_difficulty = 1;
            } else {
                current_difficulty += 1;
            }
            let question = Question::new(question, answer);
            questions_storage.insert((topic, current_difficulty), question);
        }

        Ok(Self {
            questions: questions_storage,
        })
    }
}

impl QuestionsStorage for CsvQuestionsStorage {
    fn get(&self, topic_name: String, difficulty: usize) -> Option<Question> {
        self.questions.get(&(topic_name, difficulty)).cloned()
    }
}