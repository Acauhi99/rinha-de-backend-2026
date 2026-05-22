mod response;
mod search;
mod vectorizer;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use axum::{Router, routing::{get, post}, response::IntoResponse, extract::State};
use std::sync::Arc;

struct AppState {
    index: search::Index,
}

async fn ready() -> impl IntoResponse {
    (axum::http::StatusCode::OK, "ready")
}

async fn fraud_score(
    State(state): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let vector = match vectorizer::parse_and_vectorize(&body) {
        Ok(v) => v,
        Err(_) => return response::fraud_response(0),
    };

    let query = vectorizer::quantize(&vector);
    let top5 = search::search(&state.index, &query);
    let fraud_count = top5.iter().filter(|c| c.label == 1).count() as u32;
    response::fraud_response(fraud_count)
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let index_path = if args.len() > 1 { &args[1] } else { "index.bin" };
    let bind = if args.len() > 2 { &args[2] } else { "0.0.0.0:3000" };

    let index = search::Index::load(index_path).expect("Failed to load index");
    let state = Arc::new(AppState { index });

    let app = Router::new()
        .route("/ready", get(ready))
        .route("/fraud-score", post(fraud_score))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
