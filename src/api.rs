use std::{collections::{HashMap, HashSet}, io::{Read, Cursor}, sync::Mutex};

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::{Serialize, Deserialize};
use yara::Compiler;
use zip::ZipArchive;

use crate::error::DragonflyError;

const BASE_URL: &str = "http://127.0.0.1:8000";
const MAX_SIZE: usize = 250000000;

#[derive(Debug, Serialize)]
pub struct SubmitJobResultsBody<'a> {
    pub name: &'a String,
    pub version: &'a String,
    pub score: Option<i64>,
    pub inspector_url: Option<&'a String>,
    pub rules_matched: &'a HashSet<&'a String>,
}

#[derive(Debug, Deserialize)]
pub struct Job {
    pub hash: String,
    pub name: String,
    pub version: String,
    pub distributions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GetJobResponse {
    Job(Job),
    Error { detail: String },
}

#[derive(Debug, Deserialize)]
pub struct GetRulesResponse {
    hash: String,
    rules: HashMap<String, String>,
}

pub struct State {
    pub rules: yara::Rules,
    pub hash: String,
}

pub struct DragonflyClient {
    pub client: Client,
    pub state: State,
}

fn fetch_rules(client: &Client) -> Result<GetRulesResponse, reqwest::Error> {
    client.get(format!("{BASE_URL}/rules"))
        .send()?
        .json()
}

impl State {
    pub fn new(rules: yara::Rules, hash: String) -> Self {
        Self { rules, hash }
    }

    pub fn set_hash(&mut self, hash: String) {
        self.hash = hash;
    }

    pub fn set_rules(&mut self, rules: yara::Rules) {
        self.rules = rules;
    }
}

impl DragonflyClient {
    pub fn new() -> Result<Self, DragonflyError> {
        let client = Client::builder().gzip(true).build()?;

        let response = fetch_rules(&client)?;
        let hash = response.hash;
        let rules_str = response.rules
            .into_iter()
            .map(|(_, v)| v)
            .collect::<Vec<String>>()
            .join("\n");

        let compiler = Compiler::new()?
            .add_rules_str(&rules_str)?;
        let rules = compiler.compile_rules()?;
        
        let state = State::new(rules, hash);

        Ok(Self { 
            client, 
            state,
        })
    }

    pub fn fetch_tarball(&self, download_url: &String) -> Result<tar::Archive<Cursor<Vec<u8>>>, DragonflyError> {
        let response = self.client.get(download_url)
            .send()?;

        let mut decompressed = GzDecoder::new(response);
        let mut cursor: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let read = decompressed.read_to_end(cursor.get_mut())?;

        if read > MAX_SIZE {
            Err(DragonflyError::DownloadTooLarge(download_url.to_owned()))
        } else {
            Ok(tar::Archive::new(cursor))
        }
    }

    pub fn fetch_zipfile(&self, download_url: &String) -> Result<ZipArchive<Cursor<Vec<u8>>>, DragonflyError> {
        let mut response = self.client.get(download_url)
            .send()?;

        let mut cursor = Cursor::new(Vec::new());
        let read = response.read_to_end(cursor.get_mut())?;

        if read > MAX_SIZE {
            Err(DragonflyError::DownloadTooLarge(download_url.to_owned()))
        } else {
            let zip = ZipArchive::new(cursor)?;
            Ok(zip)
        }
    }
    
    pub fn sync_rules(&mut self) -> Result<(), DragonflyError> {
        let response = fetch_rules(&self.client)?;
        
        let rules_str = response.rules
            .iter()
            .map(|(_, v)| v.to_owned())
            .collect::<Vec<String>>()
            .join("\n");
        
        let compiler = Compiler::new()?
            .add_rules_str(&rules_str)?;
        let compiled_rules = compiler.compile_rules()?;

        self.state.set_hash(response.hash);
        self.state.set_rules(compiled_rules);

        Ok(())
    }


    pub fn get_job(&self) -> reqwest::Result<Option<Job>> {
        let res: GetJobResponse = self.client.post(format!("{BASE_URL}/job"))
            .send()?
            .json()?;
        
        let job = match res {
            GetJobResponse::Job(job) => Some(job),
            GetJobResponse::Error {..} => None,
        };

        Ok(job)
    }

    pub fn submit_job_results(&self, body: SubmitJobResultsBody) -> reqwest::Result<()> {
        self.client.put(format!("{BASE_URL}/package"))
            .json(&body)
            .send()?;

        Ok(())
    }
}
