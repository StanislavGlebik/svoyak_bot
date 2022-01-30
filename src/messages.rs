use rand::{seq::SliceRandom, thread_rng};
pub const BEGIN_CMD: &str = "Начинаем";

pub const INCORRECT_ANSWER: &str = "Нет";


pub fn get_rand_correct_answer() -> String {
    let answers = vec![
        "Правильно!".to_string(),
        "Верно!".to_string(),
        "В точку!".to_string(),
        "Несомненно это так".to_string(),
        "Блестящий ответ!".to_string(),
        "Отлично!".to_string(),
        "Замечательно, продолжаем".to_string(),
    ];

    let mut rng = thread_rng();
    answers.choose(&mut rng).cloned().unwrap()
}
