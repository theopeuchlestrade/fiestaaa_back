//! Loopback-only provider fixture: never forwards requests to real services.
use actix_web::{
    App, HttpRequest, HttpResponse, HttpServer, dev::ServerHandle, http::StatusCode, web,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};
type RecordedRequests = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

#[derive(Clone)]
struct State {
    responses: Arc<Mutex<HashMap<String, (u16, Value)>>>,
    requests: RecordedRequests,
}
pub struct MockServer {
    pub base: String,
    state: State,
    handle: ServerHandle,
    thread: JoinHandle<()>,
}
async fn respond(req: HttpRequest, body: web::Bytes, state: web::Data<State>) -> HttpResponse {
    state
        .requests
        .lock()
        .unwrap()
        .push((req.path().to_string(), body.to_vec()));
    let (status, value) = state
        .responses
        .lock()
        .unwrap()
        .get(req.path())
        .cloned()
        .unwrap_or((503, serde_json::json!({"error":"unconfigured_fixture"})));
    HttpResponse::build(StatusCode::from_u16(status).unwrap()).json(value)
}
impl MockServer {
    pub fn start() -> Self {
        let state = State {
            responses: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let server_state = state.clone();
        let (tx, rx) = std::sync::mpsc::channel::<(SocketAddr, ServerHandle)>();
        let thread = std::thread::spawn(move || {
            actix_web::rt::System::new().block_on(async move {
                let server = HttpServer::new(move || {
                    App::new()
                        .app_data(web::Data::new(server_state.clone()))
                        .default_service(web::to(respond))
                })
                .workers(1)
                .bind(("127.0.0.1", 0))
                .unwrap();
                let addr = server.addrs()[0];
                let future = server.run();
                tx.send((addr, future.handle())).unwrap();
                future.await.unwrap();
            })
        });
        let (addr, handle) = rx.recv().unwrap();
        Self {
            base: format!("http://{addr}"),
            state,
            handle,
            thread,
        }
    }
    pub fn response(&self, path: &str, status: u16, value: Value) {
        self.state
            .responses
            .lock()
            .unwrap()
            .insert(path.into(), (status, value));
    }
    pub fn requests(&self) -> Vec<(String, Vec<u8>)> {
        self.state.requests.lock().unwrap().clone()
    }
    pub async fn stop(self) {
        self.handle.stop(true).await;
        self.thread.join().unwrap();
    }
}
