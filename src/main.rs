use std::{
    borrow::Cow,
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_session::{MemoryStore, Session, SessionStore};
use async_trait::async_trait;
use axum::{
    body::{Bytes, HttpBody},
    error_handling::HandleErrorLayer,
    extract::{FromRequest, Query, RequestParts},
    response::{IntoResponse, Response},
    routing::{get, post},
    BoxError, Extension, Form, Json, Router,
};
use axum_extra::extract::{cookie::Cookie, CookieJar};
use hyper::StatusCode;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .init();

    let dev_routes = Router::new()
        .route("/index.php", post(dev_index))
        .route("/api/upload.php", post(upload));

    let bbs_routes = Router::new().route("/uc_server/avatar.php", get(avatar));

    let app = Router::new()
        .route("/filelist", get(filelist))
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
    vfs: RwLock<HashMap<String, Box<[u8]>>>,
    compiled: RwLock<Vec<Box<[u8]>>>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            store: MemoryStore::new(),
            vfs: Default::default(),
            compiled: Default::default(),
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

#[derive(Deserialize)]
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
    name: String,
    addtime: time::OffsetDateTime,
    status: CompileStatus,
    op_login: GameType,
    #[serde(with = "num_bool")]
    op_qudong: bool,
    ver: u32,
}

#[derive(Deserialize)]
struct CompileOption {
    name: String,
    filename: String,
    #[serde(with = "num_bool")]
    op_safedata: bool,
    #[serde(with = "num_bool")]
    op_delad: bool,
    #[serde(with = "num_bool")]
    op_statistics: bool,
    #[serde(with = "num_bool")]
    op_jiasu: bool,
    op_keywords: String,
    op_qudong: bool,
    op_login: GameType,
    ver: u32,
}

enum IndexAction {
    Login(User),
    Submit(CompileOption),
    GetList,
    Download(u32),
}

#[async_trait]
impl<B> FromRequest<B> for IndexAction
where
    B: Send + HttpBody,
    B::Data: Send,
    B::Error: Into<BoxError>,
{
    type Rejection = StatusCode;

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        #[derive(Deserialize)]
        struct IndexFuncDesc {
            c: String,
            a: String,
            id: Option<u32>,
        }

        let Query(query) = Query::<Option<IndexFuncDesc>>::from_request(req)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        match query {
            Some(q) => {
                if q.c != "compile" {
                    Err(StatusCode::BAD_REQUEST)
                } else if q.a == "Submit" {
                    Ok(IndexAction::Submit(
                        Form::<CompileOption>::from_request(req)
                            .await
                            .map_err(|_| StatusCode::BAD_REQUEST)?
                            .0,
                    ))
                } else if q.a == "GetList" {
                    Ok(IndexAction::GetList)
                } else if q.a == "exedown" {
                    Ok(IndexAction::Download(q.id.ok_or(StatusCode::BAD_REQUEST)?))
                } else {
                    Err(StatusCode::BAD_REQUEST)
                }
            }
            None => Ok(IndexAction::Login(
                Json::<User>::from_request(req)
                    .await
                    .map_err(|_| StatusCode::BAD_REQUEST)?
                    .0,
            )),
        }
    }
}

async fn dev_index(
    Extension(state): Extension<Arc<SharedState>>,
    func: IndexAction,
    jar: CookieJar,
) -> Response {
    let store = &state.store;

    match func {
        IndexAction::Login(user) => dev_login(store, user, jar).await.into_response(),
        IndexAction::Submit(opt) => {
            let vfs = state.vfs.read().await;
            let mut compiled = state.compiled.write().await;
            submit_compile(store, &*vfs, &mut *compiled, opt, jar)
                .await
                .into_response()
        }
        IndexAction::GetList => get_compile_list(store, jar).await.into_response(),
        IndexAction::Download(id) => {
            let compiled = state.compiled.read().await;
            download(store, &*compiled, id, jar).await.into_response()
        }
    }
}

async fn dev_login(
    store: &MemoryStore,
    user: User,
    jar: CookieJar,
) -> Result<(CookieJar, &'static str), StatusCode> {
    if user.a != "new_sw_login" || user.c != "member" {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    if user.username != "xyxx" || user.password != "xyxx" {
        return Err(StatusCode::FORBIDDEN);
    }

    let session = Session::new();
    let session_cookie = store.store_session(session).await.unwrap().unwrap();
    Ok((
        jar.add(Cookie::new("PHPSESSID", session_cookie)),
        "ok|1|2|76",
    ))
}

async fn submit_compile(
    store: &MemoryStore,
    vfs: &HashMap<String, Box<[u8]>>,
    compiled: &mut Vec<Box<[u8]>>,
    opt: CompileOption,
    jar: CookieJar,
) -> Result<&'static str, StatusCode> {
    let session_cookie = jar
        .get("PHPSESSID")
        .map(|cookie| cookie.value())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let mut session = store
        .load_session(session_cookie.to_owned())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let data = vfs.get(&opt.filename).ok_or(StatusCode::BAD_REQUEST)?;

    // TODO
    let id = compiled.len() as u32;
    compiled.push(data.clone());

    let status = CompileStatus::Done;

    let mut tasks: HashMap<u32, CompileTask> = session.get("tasks").unwrap_or_default();

    tasks.insert(
        id,
        CompileTask {
            id,
            name: opt.name,
            addtime: time::OffsetDateTime::now_utc(),
            status,
            op_login: opt.op_login,
            op_qudong: opt.op_qudong,
            ver: opt.ver,
        },
    );

    session
        .insert("tasks", tasks)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok("ok")
}

async fn get_compile_list(store: &MemoryStore, jar: CookieJar) -> Result<String, StatusCode> {
    let session_cookie = jar
        .get("PHPSESSID")
        .map(|cookie| cookie.value())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let session = store
        .load_session(session_cookie.to_owned())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let tasks = session.get_raw("tasks").unwrap_or_default();

    let mut s = String::new();
    s.push_str("ok");
    s.push_str(&tasks);
    Ok(s)
}

async fn download(
    store: &MemoryStore,
    compiled: &[Box<[u8]>],
    id: u32,
    jar: CookieJar,
) -> Result<Vec<u8>, StatusCode> {
    let session_cookie = jar
        .get("PHPSESSID")
        .map(|cookie| cookie.value())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let _session = store
        .load_session(session_cookie.to_owned())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    compiled
        .get(id as usize)
        .map(|res| res.clone().into_vec())
        .ok_or(StatusCode::BAD_REQUEST)
}

struct Source {
    filename: String,
    data: Box<[u8]>,
}

#[async_trait]
impl<B> FromRequest<B> for Source
where
    B: Send + HttpBody,
    B::Data: Send,
    B::Error: Into<BoxError>,
{
    type Rejection = StatusCode;

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req).await.unwrap();
        
        
        todo!()
    }
}

async fn upload(source: Source) -> Result<&'static str, StatusCode> {
    Ok("ok")
}

async fn avatar() {}

#[derive(Debug, Deserialize)]
struct FileList {
    c: usize,
}

#[tracing::instrument]
async fn filelist(Query(FileList { c: id }): Query<FileList>) -> &'static str {
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
