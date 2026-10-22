// JS: minimist/index.js (flag values), holesail-server/index.js:put (JSON), node:net validatePort.
// JS semantics for argv values and the DHT record; neighbouring crates take `JsValue` from here.
pub mod error;
pub mod json;
mod json_error;
mod json_parse;
pub mod number;
pub mod port;
pub mod record;
mod string_number;
pub mod value;

pub use error::JsError;
pub use port::to_port;
pub use record::{nullish, record_json};
pub use value::JsValue;
