#[cfg(feature = "native")]
pub fn spawn<F>(fut: F)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    use tokio::runtime::Handle;

    match Handle::try_current() {
        Ok(_) => {
            tokio::task::spawn_local(fut);
        }
        Err(_) => {
            std::thread::spawn(move || {
                tokio::runtime::LocalRuntime::new().unwrap().block_on(fut);
            });
        }
    };
}

#[cfg(feature = "wasm")]
pub fn spawn<F>(fut: F)
where
    F: Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(fut);
}
