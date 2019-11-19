#![feature(proc_macro_hygiene, decl_macro)]

use crate::expire_queue::{ExpireQueue, InvalidUploadIdError, UploadId};
use crate::token::{UploadToken, ValidTokens};
use dotenv::dotenv;
use rocket::http::RawStr;
use rocket::request::FromParam;
use rocket::response::NamedFile;
use rocket::*;
use std::collections::HashMap;
use std::env;
use std::env::current_dir;
use std::fs::{create_dir, read_dir, remove_dir_all};
use std::io;
use std::path::PathBuf;
use std::thread::{sleep, spawn, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod expire_queue;
mod token;

impl<'r> FromParam<'r> for UploadId {
    type Error = InvalidUploadIdError;

    fn from_param(param: &'r RawStr) -> Result<Self, Self::Error> {
        let param = param.url_decode()?;

        UploadId::new(&param)
    }
}

#[get("/")]
fn home() -> &'static str {
    "Home"
}

fn now() -> u64 {
    let start = SystemTime::now();
    start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

const THOUSAND_YEARS: u64 = 1000 * 356 * 24 * 60 * 60;

#[put("/upload?<expire>&<name>", data = "<data>")]
fn upload(
    data: Data,
    expire: Option<u64>,
    name: String,
    _token: UploadToken,
    basedir: State<PathBuf>,
    expire_queue: State<ExpireQueue>,
) -> io::Result<String> {
    let id = UploadId::generate(now() + expire.unwrap_or(THOUSAND_YEARS));
    expire_queue.push(id);

    let mut path = basedir.join(id.as_string());
    create_dir(&path)?;
    path.push(name);

    data.stream_to_file(path)?;
    Ok(id.as_string())
}

#[get("/<id>/<name..>")]
fn download(id: UploadId, name: PathBuf, basedir: State<PathBuf>) -> Option<NamedFile> {
    if id.is_expired(now()) {
        None
    } else {
        let path = basedir.join(id.as_string()).join(name);
        NamedFile::open(path).ok()
    }
}

fn main() {
    dotenv().ok();

    let mut env: HashMap<_, _> = env::vars().collect();
    let expire_queue = ExpireQueue::new();

    let tokens: ValidTokens = env
        .remove("TOKENS")
        .unwrap_or_default()
        .split(',')
        .collect();

    let basedir = env
        .remove("BASEDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| current_dir().unwrap_or_default().join("data"));

    expire_job(basedir.clone(), expire_queue.clone());

    rocket::ignite()
        .manage(tokens)
        .manage(basedir.clone())
        .manage(expire_queue)
        .mount("/", routes![home, upload, download])
        .launch();
}

fn expire_job(expire_basedir: PathBuf, expire_queue: ExpireQueue) -> JoinHandle<()> {
    spawn(move || {
        for dir_entry in read_dir(&expire_basedir).expect("Failed to list base directory") {
            if let Ok(entry) = dir_entry {
                if let Some(upload_id) = entry
                    .file_name()
                    .to_str()
                    .and_then(|s| UploadId::new(s).ok())
                {
                    expire_queue.push(upload_id);
                }
            }
        }

        loop {
            for expired in expire_queue.get_expired(now()) {
                let _ = remove_dir_all(expire_basedir.join(expired.as_string()));
            }

            sleep(Duration::from_secs(5 * 60));
        }
    })
}
