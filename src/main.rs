use crate::expire_queue::{ExpireQueue, InvalidUploadIdError, UploadId};
use crate::token::{UploadToken, ValidTokens};
use dotenvy::dotenv;
use futures_util::future::try_join_all;
use rocket::data::{Limits, ToByteUnit};
use rocket::form::Form;
use rocket::fs::{FileName, NamedFile, TempFile};
use rocket::request::FromParam;
use rocket::response::{Redirect, Responder};
use rocket::{get, launch, post, put, routes, Config, Data, FromForm, Request, State};
use rust_embed::{EmbeddedFile, RustEmbed};
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

struct FileResponse(EmbeddedFile);

impl<'r, 'o: 'r> Responder<'r, 'o> for FileResponse {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'o> {
        self.0.data.respond_to(request)
    }
}

#[derive(Responder)]
#[response(content_type = "html")]
struct HtmlResponse(FileResponse);

#[get("/")]
fn home() -> HtmlResponse {
    HtmlResponse(
        Templates::get("index.html")
            .map(FileResponse)
            .expect("Template not found"),
    )
}

#[derive(Responder)]
#[response(content_type = "image/svg+xml")]
struct SvgResponse(FileResponse);

#[get("/icon.svg")]
fn icon() -> SvgResponse {
    SvgResponse(
        Templates::get("icon.svg")
            .map(FileResponse)
            .expect("Template not found"),
    )
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
    name: &str,
    _token: UploadToken,
    basedir: &State<PathBuf>,
    expire_queue: &State<ExpireQueue>,
) -> io::Result<String> {
    let name = <&FileName>::from(name);
    let id = UploadId::generate(now() + expire.unwrap_or(THOUSAND_YEARS));
    expire_queue.push(id);

    let mut path = basedir.join(id.as_string());
    let name = format_filename(name);
    create_dir(&path)?;
    path.push(&name);

    data.open(2.gibibytes()).into_file(path).await?;
    Ok(format!("{}/{}", id, name))
}

#[derive(Debug, Responder)]
enum UploadResponse {
    Data(String),
    Redirect(Box<Redirect>),
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
            UploadResponse::Redirect(Box::new(Redirect::to("/?error=invalid%20token")))
        }
    } else {
        match try_join_all(data.files.into_iter().filter(|file| file.len() > 0).map(
            |mut file| async move {
                let id = UploadId::generate(now() + expire.unwrap_or(THOUSAND_YEARS));
                expire_queue.push(id);
                let name = file
                    .raw_name()
                    .map(format_filename)
                    .unwrap_or_else(|| "upload.bin".into());
                let url = format!("{}/{}", id, &name);

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
                    UploadResponse::Redirect(Box::new(Redirect::to("")))
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
        .mount("/", routes![home, put_upload, post_upload, download, icon])
}

fn expire_job(expire_basedir: PathBuf, expire_queue: ExpireQueue) -> JoinHandle<()> {
    spawn(move || {
        let entries = match read_dir(&expire_basedir) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("Failed to list base directory: {e:#}");
                return;
            }
        };

        for entry in entries.flatten() {
            if let Some(upload_id) = entry
                .file_name()
                .to_str()
                .and_then(|s| UploadId::new(s).ok())
            {
                expire_queue.push(upload_id);
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
    if !name.is_empty()
        && ext.len() < 8
        && ext.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
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

fn format_filename(name: &FileName) -> String {
    let name_no_ext = name.as_str().unwrap_or("upload");
    match filename_ext(name) {
        Some(ext) => format!("{}.{}", name_no_ext, ext),
        None => name_no_ext.into(),
    }
}
