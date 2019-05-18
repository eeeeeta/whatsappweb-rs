use std::io;
use ws;
use ring;
#[cfg(feature = "media")]
use reqwest;
use json;
use base64;
use protobuf;

macro_rules! impl_from_for_error {
        ($error:ident, $($var:ident => $orig:ty),*) => {
                $(
                        impl From<$orig> for $error {
                                fn from(err: $orig) -> $error {
                                        $error::$var(err)
                                }
                        }
                 )*
        }
}

#[macro_export]
macro_rules! bail_untyped {
        ($msg:expr) => {
                return Err(WaError::Untyped($msg.into()));
        };
        ($($arg:tt)*) => {
                return Err(WaError::UntypedOwned(format!($($arg)*)));
        }
}

#[derive(Debug, Fail)]
pub enum WaError {
        #[fail(display = "I/O error: {}", _0)]
        Io(io::Error),
        #[fail(display = "WebSocket error: {}", _0)]
        Websocket(ws::Error),
        #[fail(display = "Crypto error: {}", _0)]
        Crypto(ring::error::Unspecified),
        #[cfg(feature = "media")]
        #[fail(display = "reqwest error: {}", _0)]
        Reqwest(reqwest::Error),
        #[fail(display = "JSON error: {}", _0)]
        Json(json::Error),
        #[fail(display = "base64 decode error: {}", _0)]
        Base64(base64::DecodeError),
        #[fail(display = "Protobuf error: {}", _0)]
        Protobuf(protobuf::ProtobufError),
        #[fail(display = "Missing node attribute \"{}\"", _0)]
        NodeAttributeMissing(&'static str),
        #[fail(display = "Missing JSON field \"{}\"", _0)]
        JsonFieldMissing(&'static str),
        #[fail(display = "{}", _0)]
        UntypedOwned(String),
        #[fail(display = "{}", _0)]
        Untyped(&'static str)
}

pub type WaResult<T> = ::std::result::Result<T, WaError>;
// FIXME: to avoid changing all the damn result types everywhere
pub(crate) type Result<T> = WaResult<T>;

impl_from_for_error!(WaError,
                     Io => io::Error,
                     Websocket => ws::Error,
                     Crypto => ring::error::Unspecified,
                     Json => json::Error,
                     Base64 => base64::DecodeError,
                     Protobuf => protobuf::ProtobufError,
                     UntypedOwned => String,
                     Untyped => &'static str);
#[cfg(feature = "media")]
impl_from_for_error!(WaError,
                     Reqwest => reqwest::Error);
