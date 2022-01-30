use rand::{seq::SliceRandom, thread_rng};

pub fn get_rand_sticker() -> Option<String> {
   let stickers = vec![
       "CAACAgIAAxkBAAJC8mHu7iSGjSCrqcX_6idsLAHqm181AAIVAAPANk8TzVamO2GeZOcjBA".to_string(),
       "CAACAgIAAxkBAAJC82Hu7nhptVATZC7GLnGz00Q6nqCMAAJxFAAC6Cy5SjtLqwG1uMNJIwQ".to_string(),
       "CAACAgIAAxkBAAJC9GHu7oWfAsm3m31zx06tvFjUK6DHAAJJFgACJl6gSN8LumhksQqgIwQ".to_string(),
       "CAACAgIAAxkBAAJLWWH2fgX2KK1dnrruyvIKTGGFYv7yAALSEgACCzsRShf2atm48POfIwQ".to_string(),
       "CAACAgIAAxkBAAJLWmH2fiNXRWY4cXNQEHECeNepDXyBAAJTFQACl6NASUkdCbRrtLunIwQ".to_string(),
       "CAACAgIAAxkBAAJLW2H2fkSnL9rzDECwodrfKTgxvTgEAALUFAACb7nISPsOb82nfnIQIwQ".to_string(),
       "CAACAgEAAxkBAAJLXGH2fmQMzV62jolwSQ3YgpfhulsaAAJKAQACoAQpR4ZbZ4pD98oxIwQ".to_string(),
       "CAACAgIAAxkBAAJLXWH2fo1TB4qUewwEBZhLBbjf-K5JAALdDwACzkP4SjmdKcNmQDlrIwQ".to_string(),
   ];
    let mut rng = thread_rng();
    stickers.choose(&mut rng).cloned()
}
