use std::{borrow::Cow, collections::HashMap, fmt::Debug, sync::Arc, time::Duration};

use async_session::{MemoryStore, Session, SessionStore};
use async_trait::async_trait;
use axum::{
    body::{Bytes, HttpBody},
    error_handling::HandleErrorLayer,
    extract::{FromRequest, Query, RequestParts},
    response::{IntoResponse, Response},
    routing::{get, post},
    BoxError, Extension, Form, Router,
};
use axum_extra::extract::CookieJar;
use dream_tutor::{crypto, GameRes};
use encoding_rs::GBK;
use hyper::{HeaderMap, StatusCode};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;

use tracing::metadata::LevelFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(LevelFilter::TRACE))
        .init();

    let dev_routes = Router::new()
        .route("/index.php", post(dev_index).get(dev_index))
        .route("/api/upload.php", post(upload));

    let bbs_routes = Router::new().route("/uc_server/avatar.php", get(avatar));

    let app = Router::new()
        .route("/filelist/lock_filelist1.txt", get(filelist))
        .nest("/dmdev", dev_routes)
        .nest("/dmbbs", bbs_routes)
        .layer(
            ServiceBuilder::new()
                // Handle errors from middleware
                .layer(HandleErrorLayer::new(handle_error))
                .load_shed()
                .concurrency_limit(1024)
                .timeout(Duration::from_secs(10))
                .layer(TraceLayer::new_for_http())
                .layer(Extension(Arc::new(SharedState::default())))
                .into_inner(),
        );

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[derive(Debug)]
struct SharedState {
    store: MemoryStore,
    files: RwLock<HashMap<String, Box<[u8]>>>,
    results: RwLock<Vec<Result<Box<[u8]>, String>>>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            store: MemoryStore::new(),
            files: Default::default(),
            results: Default::default(),
        }
    }
}

mod num_bool {
    use serde::{
        de::{Error, Unexpected},
        Deserialize, Deserializer,
    };

    pub fn serialize<S>(b: &bool, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(if *b { 1 } else { 0 })
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<bool, D::Error>
    where
        D: Deserializer<'de>,
    {
        match u8::deserialize(deserializer)? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(Error::invalid_value(
                Unexpected::Unsigned(other.into()),
                &"1 or 0",
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct User {
    c: String,
    a: String,
    username: String,
    password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TryFromPrimitive, IntoPrimitive)]
#[repr(u32)]
#[serde(try_from = "u32", into = "u32")]
enum CompileStatus {
    Processing = 0,
    Failed = 1,
    Done = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize, TryFromPrimitive, IntoPrimitive)]
#[repr(u32)]
#[serde(try_from = "u32", into = "u32")]
enum GameType {
    Login = 1,
    AutoUpdate = 2,
    Offline = 3,
}

#[derive(Debug, Serialize, Deserialize)]
struct CompileTask {
    id: u32,
    filename: String,
    addtime: time::PrimitiveDateTime,
    status: CompileStatus,
    op_login: GameType,
    #[serde(with = "num_bool")]
    op_qudong: bool,
    ver: u32,
}

#[derive(Debug, Deserialize)]
struct CompileOption {
    #[allow(unused)]
    name: String,
    filename: String,
    #[serde(with = "num_bool")]
    op_safedata: bool,
    /// Unknow, so just ignored
    #[serde(with = "num_bool")]
    #[allow(unused)]
    op_delad: bool,
    #[serde(with = "num_bool")]
    op_statistics: bool,
    #[serde(with = "num_bool")]
    op_jiasu: bool,
    op_keywords: String,
    #[serde(with = "num_bool")]
    op_qudong: bool,
    op_login: GameType,
    ver: u32,
}

#[derive(Debug)]
enum IndexAction {
    Login(User),
    Submit(CompileOption),
    GetList,
    GetReason(u32),
    Download(u32),
}
#[derive(Debug, Deserialize)]
struct IndexActionType {
    c: String,
    a: String,
    id: Option<u32>,
}

#[async_trait]
impl<B> FromRequest<B> for IndexAction
where
    B: Send + HttpBody,
    B::Data: Send,
    B::Error: Into<BoxError>,
{
    type Rejection = StatusCode;

    #[tracing::instrument(name = "IndexAction", skip(req))]
    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let query = Query::<IndexActionType>::from_request(req)
            .await
            .map_err(|err| tracing::info!("serialize query error: {:?}", err))
            .ok();
        tracing::trace!("query: {:?}", query);

        match query {
            Some(Query(q)) if q.c != "compile" => Err(StatusCode::BAD_REQUEST),
            Some(Query(q)) => match q.a.as_str() {
                "Submit" => {
                    // Not use `Form::from_request` while need of converting from GBK to utf-8
                    let bytes = Bytes::from_request(req).await.map_err(|err| {
                        tracing::error!("into bytes error: {:?}", err);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?;

                    let (s, _, _) = GBK.decode(&bytes);
                    tracing::debug!("compile request: {}", s);

                    let option = serde_urlencoded::from_str(&s).map_err(|err| {
                        tracing::error!("urlencoded: {:?}", err);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?;
                    Ok(IndexAction::Submit(option))
                }
                "GetList" => Ok(IndexAction::GetList),
                "getreason" => Ok(IndexAction::GetReason(q.id.ok_or(StatusCode::BAD_REQUEST)?)),
                "exedown" => Ok(IndexAction::Download(q.id.ok_or(StatusCode::BAD_REQUEST)?)),
                _ => Err(StatusCode::BAD_REQUEST),
            },
            None => Ok(IndexAction::Login(
                Form::<User>::from_request(req)
                    .await
                    .map_err(|err| {
                        tracing::trace!("err: {:?}", err);
                        StatusCode::BAD_REQUEST
                    })?
                    .0,
            )),
        }
    }
}

#[tracing::instrument]
async fn dev_index(
    Extension(state): Extension<Arc<SharedState>>,
    func: IndexAction,
    jar: CookieJar,
) -> Response {
    tracing::trace!("dev_index");
    match func {
        IndexAction::Login(user) => dev_login(state, user).await.into_response(),
        IndexAction::Submit(opt) => submit_compile(state, opt, jar).await.into_response(),
        IndexAction::GetList => get_compile_list(state, jar).await.into_response(),
        IndexAction::GetReason(id) => get_fail_reason(state, jar, id).await.into_response(),
        IndexAction::Download(id) => download(state, jar, id).await.into_response(),
    }
}

#[tracing::instrument]
async fn dev_login(
    state: Arc<SharedState>,
    user: User,
) -> Result<(HeaderMap, &'static str), (StatusCode, &'static str)> {
    // assertion: client should only request login as a member
    if user.a != "new_sw_login" || user.c != "member" {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "assertion failed"));
    }
    if user.username != "xyxx" || user.password != "xyxx" {
        return Err((StatusCode::FORBIDDEN, "incorrect username or password"));
    }

    // create a new session for the login for this time
    let session_id = {
        let session = Session::new();
        let session_cookie = state
            .store
            .store_session(session)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "failed to store session"))?
            .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "no valid session"))?;

        base64::encode_config(session_cookie, base64::CRYPT)
    };

    // build header
    let mut header = HeaderMap::new();
    header.insert(
        "Set-Cookie",
        format!("PHPSESSID={session_id}; path=/").parse().unwrap(),
    );
    header.insert("Expires", "Thu, 19 Nov 1981 08:52:00 GMT".parse().unwrap());
    header.insert(
        "Cache-Control",
        "no-store, no-cache, must-revalidate, post-check=0, pre-check=0"
            .parse()
            .unwrap(),
    );

    // 1 for uid, 76 for user group
    const USER_INFO: &str = "ok|1|2|76";
    Ok((header, USER_INFO))
}

#[tracing::instrument]
async fn check_session(store: &MemoryStore, jar: CookieJar) -> Result<Session, StatusCode> {
    let session_cookie = jar
        .get("PHPSESSID")
        .map(|cookie| base64::decode_config(cookie.value(), base64::CRYPT))
        .ok_or(StatusCode::UNAUTHORIZED)?
        .map(String::from_utf8)
        .map_err(|err| {
            tracing::info!("base64 decode: {:#?}", err);
            StatusCode::BAD_REQUEST
        })?
        .map_err(|err| {
            tracing::info!("from_utf8: {:#?}", err);
            StatusCode::BAD_REQUEST
        })?;

    store
        .load_session(session_cookie.to_owned())
        .await
        .map_err(|err| {
            tracing::error!("err: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::UNAUTHORIZED)
}

fn check_id_in_session(id: u32, session: Session) -> Result<(), StatusCode> {
    session
        .get::<HashMap<u32, CompileTask>>("tasks")
        .and_then(|t| t.contains_key(&id).then_some(()))
        .ok_or(StatusCode::FORBIDDEN)
}

fn compile(
    file: &[u8],
    option: &CompileOption,
    build_time: time::PrimitiveDateTime,
) -> Result<Box<[u8]>, String> {
    // only offline mode supported for now
    if !matches!(option.op_login, GameType::Offline) {
        return Err("unsupported game type".to_owned());
    }

    if !option.op_delad {
        return Err("unknown option delad".to_owned());
    }

    if !option.op_jiasu && !option.op_qudong && !option.op_safedata {
        return Err("unsupported option".to_owned());
    }

    // build game resources
    GameRes::new()
        .illegal_keywords(&option.op_keywords)
        .anti_memory_cheat(option.op_safedata)
        .anti_speed_hack(option.op_jiasu)
        .statistics(option.op_statistics)
        .build_time(build_time)
        .filename(&option.filename)
        .game_lua(file)
        .build()
        .map(|v| v.into_boxed_slice())
        .map_err(|err| err.to_string())
}

#[tracing::instrument]
async fn submit_compile(
    state: Arc<SharedState>,
    option: CompileOption,
    jar: CookieJar,
) -> Result<&'static str, StatusCode> {
    let mut session = check_session(&state.store, jar).await?;

    // get pre-upload game data file
    let files = state.files.read().await;
    let file = files
        .get(&option.filename)
        .ok_or(StatusCode::PRECONDITION_REQUIRED)?;

    // maybe use local time zone in future?
    let build_time = {
        let offseted = time::OffsetDateTime::now_utc();
        time::PrimitiveDateTime::new(offseted.date(), offseted.time())
    };

    // compile start get compile status
    let result = compile(file, &option, build_time).and_then(|bytes| {
        let mut buf = Vec::new();
        crypto::compress(&bytes, &mut buf)
            .map(|_| buf.into_boxed_slice())
            .map_err(|err| err.to_string())
    });
    let status = match result {
        Ok(_) => CompileStatus::Done,
        Err(_) => CompileStatus::Failed,
    };

    // push compilation result into results
    // get a id for future use
    let id = {
        let mut results = state.results.write().await;
        let id = results.len() as u32;
        results.push(result);
        id
    };

    // push the compilation detial into session for client querying
    let mut tasks: HashMap<u32, CompileTask> = session.get("tasks").unwrap_or_default();

    tasks.insert(
        id,
        CompileTask {
            id,
            filename: option.filename,
            addtime: build_time,
            status,
            op_login: option.op_login,
            op_qudong: option.op_qudong,
            ver: option.ver,
        },
    );

    session.insert("tasks", tasks).map_err(|err| {
        tracing::error!("insert task error: {:?}", err);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok("ok")
}

#[tracing::instrument]
async fn get_compile_list(state: Arc<SharedState>, jar: CookieJar) -> Result<String, StatusCode> {
    let session = check_session(&state.store, jar).await?;

    // get compilation tasks in this session, empty list by default
    let tasks = session.get_raw("tasks").unwrap_or_default();

    let mut s = String::new();
    s.push_str("ok");
    s.push_str(&tasks);
    Ok(s)
}

#[tracing::instrument]
async fn get_fail_reason(
    state: Arc<SharedState>,
    jar: CookieJar,
    id: u32,
) -> Result<String, StatusCode> {
    tracing::trace!("id = {:?}", id);

    let session = check_session(&state.store, jar).await?;
    check_id_in_session(id, session)?;

    let results = state.results.read().await;
    results
        .get(id as usize)
        .ok_or(StatusCode::NOT_FOUND)?
        .as_ref()
        .err()
        .cloned()
        .ok_or(StatusCode::PRECONDITION_FAILED)
}

#[tracing::instrument]
async fn download(
    state: Arc<SharedState>,
    jar: CookieJar,
    id: u32,
) -> Result<Vec<u8>, (StatusCode, &'static str)> {
    let session = check_session(&state.store, jar)
        .await
        .map_err(|code| (code, "invalid session"))?;
    check_id_in_session(id, session).map_err(|code| (code, "invalid id"))?;

    // get compilation result with request id
    let results = state.results.read().await;
    results
        .get(id as usize)
        .map(|r| r.as_ref().map(|data| data.clone().into_vec()))
        .ok_or((StatusCode::NOT_FOUND, "no such data for that id"))?
        .map_err(|_| (StatusCode::PRECONDITION_FAILED, "compile failed"))
}

#[derive(Debug)]
struct UploadedFile {
    filename: String,
    data: Box<[u8]>,
}

#[async_trait]
impl<B> FromRequest<B> for UploadedFile
where
    B: Send + HttpBody,
    B::Data: Send,
    B::Error: Into<BoxError>,
{
    type Rejection = (StatusCode, &'static str);

    #[tracing::instrument(name = "UploadedFile", skip_all)]
    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req).await.unwrap();

        // split bytes by number(0xc1) of space characters(0x20) as separator
        let (mut p0, mut p1) = (None, None);
        for (i, &b) in bytes.iter().enumerate() {
            match (p0, b) {
                (None, 0x20) => p0 = Some(i),
                (None, _) => {}
                (Some(_), 0x20) => {}
                (Some(s), _) if i - s > 0xc0 => {
                    p1 = Some(i - s);
                    break;
                }
                (Some(_), _) => p0 = None,
            }
        }
        let (p0, p1) = p0
            .zip(p1)
            .ok_or((StatusCode::BAD_REQUEST, "bad data format"))?;

        let (filename, rest) = bytes.split_at(p0);
        let (_pad, data) = rest.split_at(p1);

        // get real name (`123` in `..\compileplatform\upload\123.res`)
        let filename = filename
            .rsplit(|b| *b == b'\\')
            .next()
            .and_then(|s| s.split(|b| *b == b'.').next())
            .ok_or((StatusCode::BAD_REQUEST, "filename not found"))?
            .to_owned();

        tracing::debug!("filename = {:X?}", filename);

        let mut buf = Vec::new();
        crypto::decompress(data, &mut buf).map_err(|err| {
            tracing::error!("decompress error: {:?}", err);
            (StatusCode::INTERNAL_SERVER_ERROR, "decompress error")
        })?;

        let filename = String::from_utf8(filename)
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unexpected encoding"))?;

        Ok(UploadedFile {
            filename,
            data: buf.into_boxed_slice(),
        })
    }
}

#[tracing::instrument]
async fn upload(Extension(state): Extension<Arc<SharedState>>, file: UploadedFile) -> &'static str {
    let mut files = state.files.write().await;
    files.insert(file.filename, file.data);

    "ok"
}

async fn avatar() -> &'static [u8] {
    include_bytes!("../static/avatar.jpg")
}

#[derive(Debug, Deserialize)]
struct FileList {
    c: usize,
}

#[tracing::instrument]
async fn filelist(Query(FileList { c: id }): Query<FileList>) -> &'static str {
    tracing::trace!("enter");
    "1DDE3CA781B0431700B6591BB8FE403D"
}

async fn handle_error(error: BoxError) -> impl IntoResponse {
    if error.is::<tower::timeout::error::Elapsed>() {
        return (StatusCode::REQUEST_TIMEOUT, Cow::from("request timed out"));
    }

    if error.is::<tower::load_shed::error::Overloaded>() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Cow::from("service is overloaded, try again later"),
        );
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Cow::from(format!("Unhandled internal error: {}", error)),
    )
}
