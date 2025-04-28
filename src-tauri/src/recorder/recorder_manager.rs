use actix_web::Response;

fn handle_hls_request(ts_path: Option<&str>) -> Response {
    if let Some(ts_path) = ts_path {
        if let Ok(content) = std::fs::read(ts_path) {
            return Response::builder()
                .status(200)
                .header("Content-Type", "video/mp2t")
                .header("Cache-Control", "no-cache")
                .header("Access-Control-Allow-Origin", "*")
                .body(content)
                .unwrap();
        }
    }
    Response::builder()
        .status(404)
        .header("Content-Type", "text/plain")
        .header("Cache-Control", "no-cache")
        .header("Access-Control-Allow-Origin", "*")
        .body(b"Not Found".to_vec())
        .unwrap()
}
