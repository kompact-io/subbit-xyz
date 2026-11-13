//! Cross-platform async sleep. `tx::submit`'s retry/backoff loop is
//! the one place in this crate's core logic that needs to wait between
//! attempts, and that logic has to work compiled either way (native
//! `ui::cli`/`subbit-canary`, or wasm `ui::wasm`) without either skin
//! owning the implementation -- same "same logic engine everywhere"
//! rule as everything else in this crate (`lib.rs`'s own doc). No new
//! third-party dependency: native defers to `tokio::time::sleep`
//! (every native caller here already runs under a tokio runtime);
//! wasm uses the browser's own `setTimeout`, via `web-sys`/
//! `wasm-bindgen-futures`, both already crate dependencies for
//! `ui::wasm`'s other needs.

use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn sleep(duration: Duration) {
    let millis = duration.as_millis().min(i32::MAX as u128) as i32;
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let window = web_sys::window().expect("sleep: no global `window`");
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis)
            .expect("set_timeout_with_callback_and_timeout_and_arguments_0 failed");
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
