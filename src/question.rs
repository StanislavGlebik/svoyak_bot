use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Question {
    question: String,
    answer: String,
    comments: Option<String>,
    image: Option<PathBuf>,
}

impl Question {
    pub fn new<T: ToString>(question: T, answer: T, comments: Option<T>) -> Self {
        Self {
            question: question.to_string(),
            answer: answer.to_string(),
            comments: comments.map(|s| s.to_string()),
            image: None,
        }
    }

    pub fn question(&self) -> String {
        self.question.clone()
    }

    pub fn answer(&self) -> String {
        self.answer.clone()
    }

    pub fn comments(&self) -> &Option<String> {
        &self.comments
    }

    pub fn image(&self) -> &Option<PathBuf> {
        &self.image
    }

    pub fn set_image(&mut self, path: PathBuf) {
        self.image = Some(path);
    }
}
