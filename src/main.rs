use crate::expire_queue::{ExpireQueue, InvalidUploadIdError, UploadId};
use crate::token::{UploadToken, ValidTokens};
use dotenv::dotenv;
use futures_util::future::try_join_all;
use rocket::data::{Limits, ToByteUnit};
use rocket::form::Form;
use rocket::fs::{FileName, NamedFile, TempFile};
use rocket::request::FromParam;
use rocket::response::Redirect;
use rocket::{get, launch, post, put, routes, Config, Data, FromForm, Responder, State};
use rust_embed::RustEmbed;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::env::{self, current_dir};
use std::fs::{create_dir, create_dir_all, read_dir, remove_dir_all};
use std::io;
use std::path::PathBuf;
use std::thread::{sleep, spawn, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod expire_queue;
mod token;

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
struct UploadResponseData {
    success: bool,
    error: Option<Cow<'static, str>>,
    urls: Option<Vec<String>>,
}

#[derive(FromForm, Debug)]
struct UploadData<'r> {
    expire: Option<u64>,
    ajax: bool,
    token: &'r str,
    files: Vec<TempFile<'r>>,
}

#[post("/upload", data = "<data>")]
async fn post_upload(
    data: Form<UploadData<'_>>,
    accepted_tokens: &State<ValidTokens>,
    basedir: &State<PathBuf>,
    expire_queue: &State<ExpireQueue>,
) -> UploadResponse {
    let data = data.into_inner();
    let expire = data.expire;
    let ajax = data.ajax;
    let token = data.token;

    if !accepted_tokens.contains(token) {
        if ajax {
            UploadResponse::Data(
                serde_json::to_string(&UploadResponseData {
                    success: false,
                    error: Some("invalid token".into()),
                    urls: None,
                })
                .unwrap_or_default(),
            )
        } else {
            UploadResponse::Redirect(Redirect::to("/?error=invalid%20token"))
        }
    } else {
        match try_join_all(data.files.into_iter().filter(|file| file.len() > 0).map(
            |mut file| async move {
                let id = UploadId::generate(now() + expire.unwrap_or(THOUSAND_YEARS));
                expire_queue.push(id);
                let name = file.name().unwrap_or("upload");
                let ext = file.raw_name().and_then(filename_ext).unwrap_or_default();
                let name = format!("{}.{}", name, ext);
                let url = format!("{}/{}", id.as_string(), &name);

                let mut path: PathBuf = basedir.join(id.as_string());
                create_dir(&path)?;
                path.push(name);

                file.persist_to(path).await.map(|_| url)
            },
        ))
        .await
        {
            Ok(urls) => {
                if ajax {
                    UploadResponse::Data(
                        serde_json::to_string(&UploadResponseData {
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
            Err(e) => UploadResponse::Data(
                serde_json::to_string(&UploadResponseData {
                    success: false,
                    error: Some(format!("error while moving file: {}", e).into()),
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
fn rocket() -> _ {
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

    let tmpdir = basedir.join("tmp");
    create_dir_all(&tmpdir).expect("failed to create tmp directory");

    expire_job(basedir.clone(), expire_queue.clone());

    let figment = Config::figment().merge(("temp_dir", tmpdir)).merge((
        "limits",
        Limits::new()
            .limit("file", 2.gibibytes())
            .limit("data-form", 2.gibibytes()),
    ));

    rocket::custom(figment)
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

fn filename_ext(name: &FileName) -> Option<&str> {
    let raw = name.dangerous_unsafe_unsanitized_raw().as_str();
    let (name, ext) = raw.split_once('.')?;
    if name.len() > 0 && ext.len() < 8 && ext.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
    {
        Some(ext)
    } else {
        None
    }
}

#[test]
fn test_ext() {
    assert_eq!(Some("jpg"), filename_ext("foo.jpg".into()));
    assert_eq!(Some("tar.gz"), filename_ext("foo.tar.gz".into()));
    assert_eq!(None, filename_ext(".png".into()));
    assert_eq!(None, filename_ext("../foo.png".into()));
    assert_eq!(None, filename_ext("tmp/../foo.png".into()));
}
