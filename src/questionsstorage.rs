use csv;
use failure::{err_msg, Error};
use hyper::Client;
use hyper_tls::HttpsConnector;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use regex::Regex;

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
    pub async fn new(p: String, google_api_key: Option<String>, use_cached_questions: bool) -> Result<Self, Error> {
        let dir = if p.starts_with("http") {
            eprintln!("downloading questions from google drive");
            downloading_questions_from_gdrive(p, use_cached_questions).await?
        } else {
            PathBuf::from(&p)
        };

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
                if record.len() < 5 {
                    let msg = format!("incorrect number of field: {} < 4", record.len());
                    return Err(err_msg(msg));
                }
                let topic = record.get(0).unwrap().to_string();
                // second field is cost, we ignore it here
                let attachment = record.get(2).unwrap();
                let (image, audio) = if !attachment.is_empty() {
                    parse_attachment(attachment, google_api_key.clone()).await?
                } else {
                    (None, None)
                };
                let question = record.get(3).unwrap();
                let answer = record.get(4).unwrap();
                let comment = record.get(5);
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

                        let mut question = if let Some((cat_in_bag_topic, question)) = check_if_cat_in_bag(question.to_string())? {
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
                        if let Some(image) = image {
                            question.set_image(image);
                        }
                        if let Some(audio) = audio {
                            question.set_audio(audio);
                        }
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

async fn downloading_questions_from_gdrive(url: String, use_cached_questions: bool) -> Result<PathBuf, Error> {
    
    let p = PathBuf::from("downloaded_questions");
    if use_cached_questions {
        eprintln!("using cached questions");
        for i in 1..4 {
            let tour = p.join(format!("tour{}.csv", i));
            if !tour.exists() {
                return Err(err_msg(format!("cannot use cached questions because {:?} does not exist", p)));
            }
        }

        return Ok(p);
    }

    let regex = "^https://docs.google.com/spreadsheets/d/([^/]+)/edit";
    let re = Regex::new(regex)?;
    let matches = re.captures(&url).ok_or_else(|| err_msg("invalid questions url"))?;
    let m = matches.get(1).unwrap().as_str();

    
    if !p.exists() {
        std::fs::create_dir(p.clone())?;
    }
    for i in 1..4 {
        let s = serde_urlencoded::to_string(&[("sheet", format!("Тур {}", i))])?;
        let url = format!("https://docs.google.com/spreadsheets/d/{}/gviz/tq?tqx=out:csv&{}", m, s);
        eprintln!("downloading {}", url);
        let bytes = download_url(&url).await?;
        eprintln!("downloaded {}", bytes.len());
        let tour = p.join(format!("tour{}.csv", i));
        std::fs::write(tour.clone(), bytes)?;
        eprintln!("written to {:?}", tour);
    }
    
    Ok(p)
}

async fn parse_attachment(attachment: &str, google_api_key: Option<String>) -> Result<(Option<PathBuf>, Option<PathBuf>), Error> {
    let split = attachment.splitn(2, " ").collect::<Vec<_>>();
    let uri = if split.len() == 2 {
        split[1]
    } else {
        split[0]
    };

    let uri = convert_url(uri.to_string(), google_api_key);
    eprintln!("converted url to {}", uri);
    let mut s = DefaultHasher::new();
    uri.hash(&mut s);
    let filename = format!("{}", s.finish());
    
    if !Path::new(&filename).exists() {
        let bytes = download_url(&uri).await?;
        eprintln!("downloaded {}", bytes.len());
        std::fs::write(filename.clone(), bytes)?;
        eprintln!("written to {}", filename);
    } else {
        eprintln!("skiping download because already downloaded");
    }

    let maybe_type = infer::get_from_path(filename.clone())?;
    let ty = maybe_type.ok_or_else(|| err_msg(format!("cannot get type of {}", filename)))?;

    if  ty.matcher_type() == infer::MatcherType::Image {
        Ok((Some(filename.into()), None))
    } else if ty.matcher_type() == infer::MatcherType::Audio {
        Ok((None, Some(filename.into())))
    } else {
        Err(err_msg(format!("invalid attachment type {}", ty)))
    }
}

async fn download_url(uri: &str) -> Result<hyper::body::Bytes, Error> {
    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);
    let uri = uri.parse()?;

    let mut resp = client.get(uri).await?;
    let mut status = resp.status();

    if status == hyper::StatusCode::FOUND || status == hyper::StatusCode::SEE_OTHER {
        let uri = resp.headers().get("Location")
            .ok_or_else(|| err_msg("no location after redirect"))?
            .to_str()?;
        let uri = uri.parse()?;
        resp = client.get(uri).await?;
        status = resp.status();
    }

    if status != hyper::StatusCode::OK {
        return Err(err_msg(format!("failed with error code {}", status)));
    }
    let bytes = hyper::body::to_bytes(resp.into_body()).await?;

    Ok(bytes)
}

fn convert_url(s: String, google_api_key: Option<String>) -> String {
    let regexes = &[
        "^https://drive.google.com/file/d/([^/])/view",
        "^https://docs.google.com/uc\\?export=download&id=([^/]+)",
    ];
    for regex in regexes {
        let re = Regex::new(regex).expect("wrong regex");
        if let Some(matches) = re.captures(&s) {
            let m = matches.get(1).unwrap().as_str();
            if let Some(ref google_api_key) = google_api_key {
                return format!("https://www.googleapis.com/drive/v3/files/{}?key={}&alt=media", m, google_api_key);
            } else {
                return format!("https://docs.google.com/uc?export=download&id={}", m);
            }
        }
    }
    s
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
