use err_derive::Error;
use rocket::http::Status;
use rocket::outcome::Outcome;
use rocket::request::{self, FromRequest, Request};
use rocket::State;
use std::iter::FromIterator;

#[derive(PartialOrd, PartialEq, Debug)]
pub struct UploadToken(String);

pub struct ValidTokens(Vec<UploadToken>);

impl<'a> FromIterator<&'a str> for ValidTokens {
    fn from_iter<T: IntoIterator<Item = &'a str>>(iter: T) -> Self {
        let tokens: Vec<_> = iter
            .into_iter()
            .map(|token| token.to_string())
            .map(UploadToken)
            .collect();
        ValidTokens(tokens)
    }
}

impl ValidTokens {
    pub fn contains(&self, token: &str) -> bool {
        self.0.iter().any(|accepted| accepted.0 == token)
    }
}

#[derive(Debug, Error)]
pub enum UploadTokenError {
    #[error(display = "Wrong number of upload tokens provided")]
    BadCount,
    #[error(display = "No upload token provided")]
    Missing,
    #[error(display = "Invalid upload token")]
    Invalid,
}

#[async_trait::async_trait]
impl<'r> FromRequest<'r> for UploadToken {
    type Error = UploadTokenError;

    async fn from_request(request: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        let accepted_tokens = request
            .guard::<&State<ValidTokens>>()
            .await
            .expect("No tokens configured");
        let keys: Vec<_> = request.headers().get("x-upload-token").collect();

        match keys.len() {
            0 => Outcome::Failure((Status::Unauthorized, UploadTokenError::Missing)),
            1 if accepted_tokens.contains(keys[0]) => {
                Outcome::Success(UploadToken(keys[0].to_string()))
            }
            1 => Outcome::Failure((Status::Unauthorized, UploadTokenError::Invalid)),
            _ => Outcome::Failure((Status::BadRequest, UploadTokenError::BadCount)),
        }
    }
}
