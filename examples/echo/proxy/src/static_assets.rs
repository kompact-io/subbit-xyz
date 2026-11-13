use include_dir::{Dir, include_dir};
use pingora::prelude::*;

/// The browser demo's html/js/wasm bundle (`examples/echo/www`),
/// embedded at compile time. `pkg/` is wasm-pack's `--target web`
/// output, built by `examples/echo/www/build.sh` before `cargo build`.
static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../www");

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html"
    } else if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else {
        "application/octet-stream"
    }
}

/// Serves `path` from the embedded bundle if it matches one, bypassing
/// any metering: these bytes have no catalog entry, and the page has
/// to load (wasm included) before it has any means to spend at all.
/// `None` if `path` isn't a static asset, so the caller falls through
/// to the normal (metered) proxy path.
pub async fn serve(session: &mut Session, path: &str) -> Option<Result<bool>> {
    let lookup = if path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };
    let file = ASSETS.get_file(lookup)?;
    Some(
        async {
            let mut resp = ResponseHeader::build(200, None)?;
            resp.insert_header("content-type", content_type(lookup))?;
            session.set_keepalive(None);
            session.write_response_header(Box::new(resp), false).await?;
            session
                .write_response_body(Some(bytes::Bytes::from(file.contents())), true)
                .await?;
            Ok(true)
        }
        .await,
    )
}
