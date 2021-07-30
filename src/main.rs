use crate::expire_queue::{ExpireQueue, InvalidUploadIdError, UploadId};
use crate::token::{UploadToken, ValidTokens};
use dotenv::dotenv;
use rocket::data::ToByteUnit;
use rocket::fs::NamedFile;
use rocket::request::FromParam;
use rocket::response::Redirect;
use rocket::{get, launch, post, put, routes, Data, Responder, State};
use rust_embed::RustEmbed;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::env::{self, current_dir};
use std::fs::{create_dir, read_dir, remove_dir_all, File};
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::thread::{sleep, spawn, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use upload::MultipartDatas;

mod expire_queue;
mod token;
mod upload;

impl<'r> FromParam<'r> for UploadId {
    type Error = InvalidUploadIdError;

    fn from_param(param: &'r str) -> Result<Self, Self::Error> {
        UploadId::new(param)
    }
}

#[derive(RustEmbed)]
#[folder = "templates/"]
struct Templates;

#[derive(Responder)]
#[response(content_type = "html")]
struct HtmlResponse(Cow<'static, [u8]>);

#[get("/")]
fn home() -> HtmlResponse {
    HtmlResponse(Templates::get("index.html").unwrap_or(Cow::Borrowed(b"Template not found")))
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
async fn put_upload(
    data: Data<'_>,
    expire: Option<u64>,
    name: String,
    _token: UploadToken,
    basedir: &State<PathBuf>,
    expire_queue: &State<ExpireQueue>,
) -> io::Result<String> {
    let id = UploadId::generate(now() + expire.unwrap_or(THOUSAND_YEARS));
    expire_queue.push(id);

    let mut path = basedir.join(id.as_string());
    create_dir(&path)?;
    path.push(name);

    data.open(2.gibibytes()).into_file(path).await?;
    Ok(id.as_string())
}

#[derive(Debug, Responder)]
enum UploadResponse {
    Data(String),
    Redirect(Redirect),
}

#[derive(Debug, Serialize)]
struct UploadData {
    success: bool,
    error: Option<&'static str>,
    urls: Option<Vec<String>>,
}

#[post("/upload", data = "<data>")]
fn post_upload(
    data: MultipartDatas,
    accepted_tokens: &State<ValidTokens>,
    basedir: &State<PathBuf>,
    expire_queue: &State<ExpireQueue>,
) -> UploadResponse {
    let mut fields: HashMap<String, String> = data
        .texts
        .into_iter()
        .map(|text| (text.key, text.value))
        .collect();
    let expire = fields
        .get("expire")
        .and_then(|expire| u64::from_str(expire).ok());
    let ajax = fields.get("ajax").is_some();
    let token = fields.remove("token").unwrap_or_default();

    if !accepted_tokens.contains(&token) {
        if ajax {
            UploadResponse::Data(
                serde_json::to_string(&UploadData {
                    success: false,
                    error: Some("invalid token"),
                    urls: None,
                })
                .unwrap_or_default(),
            )
        } else {
            UploadResponse::Redirect(Redirect::to("/?error=invalid%20token"))
        }
    } else {
        match data
            .files
            .into_iter()
            .map(|file| {
                let id = UploadId::generate(now() + expire.unwrap_or(THOUSAND_YEARS));
                expire_queue.push(id);
                let name = &file.filename;

                let mut path: PathBuf = basedir.join(id.as_string());
                create_dir(&path)?;
                path.push(name);

                let mut file = File::open(&file.path)?;
                io::copy(&mut file, &mut File::create(&path)?)?;
                Ok(format!("{}/{}", id.as_string(), &name))
            })
            .collect::<io::Result<Vec<String>>>()
        {
            Ok(urls) => {
                if ajax {
                    UploadResponse::Data(
                        serde_json::to_string(&UploadData {
                            success: true,
                            error: None,
                            urls: Some(urls),
                        })
                        .unwrap_or_default(),
                    )
                } else {
                    UploadResponse::Redirect(Redirect::to(""))
                }
            }
            Err(_) => UploadResponse::Data(
                serde_json::to_string(&UploadData {
                    success: false,
                    error: Some("error while moving file"),
                    urls: None,
                })
                .unwrap_or_default(),
            ),
        }
    }
}

#[get("/<id>/<name..>")]
async fn download(id: UploadId, name: PathBuf, basedir: &State<PathBuf>) -> Option<NamedFile> {
    if id.is_expired(now()) {
        None
    } else {
        let path = basedir.join(id.as_string()).join(name);
        NamedFile::open(path).await.ok()
    }
}

#[launch]
fn rockert() -> _ {
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

    rocket::build()
        .manage(tokens)
        .manage(basedir.clone())
        .manage(expire_queue)
        .mount("/", routes![home, put_upload, post_upload, download])
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
