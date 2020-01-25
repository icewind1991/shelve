use err_derive::Error;
use priority_queue::PriorityQueue;
use std::cmp::Ordering;
use std::convert::TryInto;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use uuid::{Builder, Uuid, Variant, Version};

#[derive(Debug, Clone, Hash, PartialEq, Eq, Copy)]
pub struct UploadId(Uuid);

#[derive(Debug, Error)]
pub enum InvalidUploadIdError {
    #[error(display = "Invalid string")]
    InvalidString(#[error(source)] std::str::Utf8Error),
    #[error(display = "Invalid upload id, uuid")]
    InvalidUUID(#[error(source)] uuid::Error),
}

impl UploadId {
    pub fn new(id: &str) -> Result<Self, InvalidUploadIdError> {
        let uuid = Uuid::parse_str(id)?;
        Ok(UploadId(uuid))
    }

    pub fn generate(expire: u64) -> Self {
        use rand::RngCore;

        let mut rng = rand::thread_rng();
        let mut bytes = [0; 16];

        rng.fill_bytes(&mut bytes);

        let mut uuid_bytes = *Builder::from_bytes(bytes)
            .set_variant(Variant::RFC4122)
            .set_version(Version::Random)
            .build()
            .as_bytes();

        // store the expire time in the top 7 bytes of the uuid
        // since the uuid stores metadata in bytes 6 and 8, we only
        // have 7 consecutive bytes of "free" space, so we discard 1 bytes
        // from the expire time
        // we xor the expire with the rng, to make the id look "nicer"
        let expire_masked = expire ^ u64::from_le_bytes(uuid_bytes[0..8].try_into().unwrap());
        uuid_bytes[9..].copy_from_slice(&expire_masked.to_le_bytes()[0..7]);

        UploadId(Uuid::from_bytes(uuid_bytes))
    }

    pub fn get_expire(&self) -> u64 {
        let uuid_bytes = *self.0.as_bytes();
        let mut mask_bytes: [u8; 8] = uuid_bytes[0..8].try_into().unwrap();
        mask_bytes[7] = 0;

        let mut bytes = [0; 8];
        bytes[0..7].copy_from_slice(&uuid_bytes[9..]);
        let expire_masked = u64::from_ne_bytes(bytes);

        expire_masked ^ u64::from_le_bytes(mask_bytes)
    }

    pub fn is_expired(&self, time: u64) -> bool {
        return time >= self.get_expire();
    }

    pub fn as_string(&self) -> String {
        format!("{}", self.0.to_simple())
    }
}

#[test]
fn test_generate_upload_id() {
    let id = UploadId::generate(12345);
    assert_eq!(12345, id.get_expire());
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Copy)]
pub struct Expiration(u64);

impl PartialOrd for Expiration {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // reverse the order so sort nearest expiration time first
        other.0.partial_cmp(&self.0)
    }
}

impl Ord for Expiration {
    fn cmp(&self, other: &Self) -> Ordering {
        // reverse the order so sort nearest expiration time first
        other.0.cmp(&self.0)
    }
}

impl From<u64> for Expiration {
    fn from(from: u64) -> Self {
        Expiration(from)
    }
}

#[derive(Debug, Clone)]
pub struct ExpireQueue {
    queue: Arc<Mutex<PriorityQueue<UploadId, Expiration>>>,
}

impl ExpireQueue {
    pub fn new() -> Self {
        ExpireQueue {
            queue: Arc::default(),
        }
    }

    pub fn push(&self, key: UploadId) {
        self.queue
            .lock()
            .unwrap()
            .push(key, key.get_expire().into());
    }

    pub fn get_expired(&self, since: u64) -> Vec<UploadId> {
        let expire = Expiration::from(since);
        let mut queue = self.queue.lock().unwrap();

        let mut expired = Vec::new();

        while queue.peek().map(|(_, exp)| *exp >= expire).unwrap_or(false) {
            expired.push(queue.pop().map(|(id, _)| id).unwrap())
        }

        expired
    }

    pub fn len(&self) -> usize {
        self.queue.lock().unwrap().len()
    }
}

#[test]
fn test_queue() {
    let queue = ExpireQueue::new();

    let id1 = UploadId::generate(10);
    let id2 = UploadId::generate(15);

    queue.push(id1);
    queue.push(id2);

    assert_eq!(vec![id1], queue.get_expired(12));

    assert_eq!(vec![id2], queue.get_expired(20));

    let id3 = UploadId::generate(10);
    let id4 = UploadId::generate(15);
    let id5 = UploadId::generate(20);

    queue.push(id3);
    queue.push(id4);
    queue.push(id5);

    assert_eq!(vec![id3, id4, id5], queue.get_expired(20));
}
