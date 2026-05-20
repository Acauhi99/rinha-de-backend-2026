use axum::http::StatusCode;
use axum::response::IntoResponse;

pub fn fraud_response(fraud_count: u32) -> impl IntoResponse {
    let body = match fraud_count {
        0 => b"{\"approved\":true,\"fraud_score\":0.0}\n" as &[u8],
        1 => b"{\"approved\":true,\"fraud_score\":0.2}\n" as &[u8],
        2 => b"{\"approved\":true,\"fraud_score\":0.4}\n" as &[u8],
        3 => b"{\"approved\":false,\"fraud_score\":0.6}\n" as &[u8],
        4 => b"{\"approved\":false,\"fraud_score\":0.8}\n" as &[u8],
        5 => b"{\"approved\":false,\"fraud_score\":1.0}\n" as &[u8],
        _ => unreachable!(),
    };
    (StatusCode::OK, [("content-type", "application/json")], body)
}
