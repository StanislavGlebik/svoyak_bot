#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Question {
    question: String,
    answer: String,
    comments: Option<String>,
}

impl Question {
    pub fn new<T: ToString>(question: T, answer: T, comments: Option<T>) -> Self {
        Self {
            question: question.to_string(),
            answer: answer.to_string(),
            comments: comments.map(|s| s.to_string()),
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
}
