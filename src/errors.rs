use std::io;
use ring;
#[cfg(feature = "media")]
use reqwest;
use json;
use base64;
use protobuf;
use qrcode;

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

pub trait WaErrorContext {
        fn with_context(self, ctx: &'static str) -> Self;
        fn with_owned_context<T: Into<String>>(self, ctx: T) -> Self;
}
impl<T> WaErrorContext for Result<T> {
        fn with_context(self, ctx: &'static str) -> Self {
                self.map_err(|e| {
                        WaError::Context(ctx, Box::new(e))
                })
        }
        fn with_owned_context<U: Into<String>>(self, ctx: U) -> Self {
                self.map_err(|e| {
                        WaError::OwnedContext(ctx.into(), Box::new(e))
                })
        }
}
#[derive(Debug, Copy, Clone)]
pub enum DisconnectReason {
        Replaced,
        Removed
}
#[derive(Debug, Fail)]
pub enum WaError {
        #[fail(display = "I/O error: {}", _0)]
        Io(io::Error),
        #[fail(display = "WebSocket error: {}", _0)]
        Websocket(tokio_tungstenite::tungstenite::Error),
        #[fail(display = "Crypto error: {}", _0)]
        Crypto(ring::error::Unspecified),
        #[cfg(feature = "media")]
        #[fail(display = "reqwest error: {}", _0)]
        Reqwest(reqwest::Error),
        #[cfg(feature = "media")]
        #[fail(display = "http error code {}, message: {}", _0, _1)]
        HttpError(reqwest::StatusCode, String),
        #[fail(display = "JSON error: {}", _0)]
        Json(json::Error),
        #[fail(display = "base64 decode error: {}", _0)]
        Base64(base64::DecodeError),
        #[fail(display = "Protobuf error: {}", _0)]
        Protobuf(protobuf::ProtobufError),
        #[fail(display = "QR code error: {}", _0)]
        Qr(qrcode::types::QrError),
        #[fail(display = "Missing node attribute \"{}\"", _0)]
        NodeAttributeMissing(&'static str),
        #[fail(display = "Missing JSON field \"{}\"", _0)]
        JsonFieldMissing(&'static str),
        #[fail(display = "while {}: {}", _0, _1)]
        Context(&'static str, Box<WaError>),
        #[fail(display = "while {}: {}", _0, _1)]
        OwnedContext(String, Box<WaError>),
        #[fail(display = "unknown tag {}", _0)]
        InvalidTag(u8),
        #[fail(display = "invalid payload for {}: got {}", _0, _1)]
        InvalidPayload(String, &'static str),
        #[fail(display = "invalid session state for message")]
        InvalidSessionState,
        #[fail(display = "no jid yet to make an ack")]
        NoJidYet,
        #[fail(display = "invalid direction for outgoing message")]
        InvalidDirection,
        #[fail(display = "connection timed out")]
        Timeout,
        #[fail(display = "websocket disconnected")]
        WebsocketDisconnected,
        #[fail(display = "timer failed")]
        TimerFailed,
        #[fail(display = "received status code {}", _0)]
        StatusCode(u16),
        #[fail(display = "disconnected from server")]
        Disconnected(DisconnectReason),
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
                     Websocket => tokio_tungstenite::tungstenite::Error,
                     Crypto => ring::error::Unspecified,
                     Json => json::Error,
                     Base64 => base64::DecodeError,
                     Protobuf => protobuf::ProtobufError,
                     Qr => qrcode::types::QrError,
                     UntypedOwned => String,
                     Untyped => &'static str);
#[cfg(feature = "media")]
impl_from_for_error!(WaError,
                     Reqwest => reqwest::Error);
