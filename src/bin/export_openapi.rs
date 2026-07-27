use fiestaaa_back::docs::ApiDoc;
use utoipa::OpenApi;

fn main() {
    let document =
        serde_json::to_string_pretty(&ApiDoc::openapi()).expect("OpenAPI document must serialize");
    print!("{document}");
}
