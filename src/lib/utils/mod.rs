pub mod bitstream;
pub mod futures;

#[cfg(not(feature = "wasm"))]
pub mod time;

#[cfg(feature = "wasm")]
pub mod time_wasm;
#[cfg(feature = "wasm")]
pub use time_wasm as time;
