//! Connecting to WhatsApp Web via the websocket protocol.
//!
//! This is a reimplementation of the original `connection` module,
//! using `futures` and somewhat sane coding practices.

use tokio_tungstenite as ws;
use ws::MaybeTlsStream;
use ws::tungstenite::Message;
use tokio::net::TcpStream;
use std::collections::HashMap;
use json::JsonValue;
use qrcode::QrCode;
use uuid::Uuid;
use std::collections::VecDeque;
use core::task::{Context, Poll};
use futures::{Sink, Future, FutureExt, Stream};
use tokio::time::{Interval, Delay};
use std::time::{Duration, Instant};
use std::pin::Pin;

use crate::req::WaRequest;
use crate::session::{SessionState, PersistentSession};
use crate::websocket_protocol::{WebsocketMessage, WebsocketMessagePayload, WebsocketMessageMetric};
use crate::json_protocol::{self, ServerMessage};
use crate::node_protocol::{self, AppEvent, AppMessage, MessageEventType, GroupCommand};
use crate::message::{MessageId, Peer};
use crate::event::WaEvent;
use crate::node_wire::Node;
use crate::errors::*;
use crate::{crypto, Jid};

/// WhatsApp Web WebSocket endpoint URL.
const ENDPOINT_URL: &str = "wss://web.whatsapp.com/ws";
/// WhatsApp Web WebSocker origin header value.
const ORIGIN_URL: &str = "https://web.whatsapp.com";

type WsClient = ws::WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Debug)]
pub(crate) enum CallbackType {
    /// Handle a login response for a new login.
    LoginNew,
    /// Handle a login response for a persistent login.
    LoginPersistent,
    /// Check that the returned `status` response has
    /// code 200, and throw an error if not.
    CheckStatus,
    /// Handle a received message ack after sending a message.
    ProcessAck { mid: MessageId },
    /// Handle returned message history after a message history query.
    MessagesBefore { uuid: Uuid },
    /// Handle a file upload response.
    FileUpload { uuid: Uuid },
    /// Handle a media conn response.
    MediaConn { uuid: Uuid },
    /// Handle a profile picture response.
    ProfilePicture { jid: Jid },
    /// Handle a profile status response.
    ProfileStatus { jid: Jid },
    /// Handle a group metadata response.
    GroupMetadata,
    /// Don't do anything.
    Noop
}

/// A connection to WhatsApp Web.
///
/// ## Connecting
///
/// When first connecting, use the `WebConnection::connect_new()` method,
/// and scan the QR code that appears through the `ScanCode` event.
/// This establishes a persistent session (`SessionEstablished` event),
/// which you can reuse for future connections via the
/// `WebConnection::connect_persistent()` method to avoid scanning the
/// code again.
///
/// ## Usage
///
/// This `struct` implements `Stream` and `Sink` from the `futures` crate.
/// In order to use it, you read `WaEvent`s from the stream, and send
/// `WaRequest`s into the sink to get stuff done. Read the documentation
/// on those two `enum`s to get a better idea of how it works.
///
/// You **must** actively call `poll()` and `poll_complete()` on the stream
/// and sink; it won't work at all if you don't.
///
/// ## Callbacks
///
/// Some requests you can make, like getting a profile picture, will result
/// in a corresponding event being generated. Often, you'll want to generate
/// a `Uuid` to tie the event to the request you made.
pub struct WebConnection {
    inner: WsClient,
    session_state: SessionState,
    callbacks: HashMap<String, CallbackType>,
    tag_counter: u32,
    epoch: u32,
    ping_timer: Interval,
    response_timer: Option<Delay>,
    ws_outbox: VecDeque<ws::tungstenite::Message>,
    outbox: VecDeque<WaEvent>,
    user_jid: Option<Jid>
}
impl std::marker::Unpin for WebConnection {}

impl Stream for WebConnection {
    type Item = WaResult<WaEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<WaResult<WaEvent>>> {
        while let Poll::Ready(m) = Pin::new(&mut self.inner).poll_next(cx)? {
            match m {
                Some(m) => {
                    self.response_timer = None;
                    self.on_message(m)?;
                },
                None => {
                    Err(WaError::WebsocketDisconnected)?
                }
            }
        }
        if let Poll::Ready(_) = Pin::new(&mut self.ping_timer).poll_tick(cx) {
            self.on_ping_timer();
        }
        match self.response_timer.as_mut().map(|mut x| Pin::new(&mut x).poll(cx)) {
            Some(Poll::Ready(_)) => Err(WaError::Timeout)?,
            _ => {}
        }
        match self.outbox.pop_front() {
            Some(evt) => Poll::Ready(Some(Ok(evt))),
            None => Poll::Pending,
        }
    }
}

impl Sink<WaRequest> for WebConnection {
    type Error = WaError;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<WaResult<()>> {
        match Pin::new(&mut self.inner).poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(r) => Poll::Ready(r.map_err(|e| WaError::from(e))),
        }
    }

    fn start_send(self: Pin<&mut Self>, item: WaRequest) -> WaResult<()> {
        item.apply(self)?;
        Ok(())
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<WaResult<()>> {
        loop {
            while let Some(msg) = self.ws_outbox.pop_front() {
                match Pin::new(&mut self.inner).poll_ready(cx)? {
                    Poll::Pending => {
                        self.ws_outbox.push_front(msg);
                        return Poll::Pending;
                    },
                    Poll::Ready(_) => {},
                }

                Pin::new(&mut self.inner).start_send(msg)?;
            }
            match Pin::new(&mut self.inner).poll_flush(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(WaError::from(e))),
                _ => {},
            }
            if self.ws_outbox.len() > 0 {
                continue;
            }
            else {
                break Poll::Ready(Ok(()));
            }
        }
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<WaResult<()>> {
        match Pin::new(&mut self.inner).poll_close(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(r) => Poll::Ready(r.map_err(|e| WaError::from(e))),
        }
    }
}

// *** NOTE **********************************************
// * The following `impl` blocks are actually organized
// * by function. If you're changing or adding a function,
// * check it's in vaguely the right one!
// *******************************************************

impl WebConnection {
    // This `impl` block: connecting and instantiating
    fn setup(sess: SessionState, ws: WsClient) -> Self {
        let mut ret = Self {
            inner: ws,
            session_state: sess,
            callbacks: HashMap::new(),
            tag_counter: 0,
            epoch: 0,
            ws_outbox: VecDeque::new(),
            outbox: VecDeque::new(),
            ping_timer: tokio::time::interval(Duration::new(13, 0)),
            response_timer: None,
            user_jid: None
        };
        ret.on_connected();
        ret
    }
    fn ws_connect(sess: SessionState) -> impl Future<Output=WaResult<Self>> {
        let req = http::Request::builder()
            .uri(ENDPOINT_URL)
            .header("Origin", ORIGIN_URL)
            .body(()).expect("invalid ENDPOINT_URL or ORIGIN_URL");

        let fut = tokio_tungstenite::connect_async(req)
            .map(|r| r
                .map(|ws| WebConnection::setup(sess, ws.0))
                .map_err(|e| WaError::from(e))
            );
        fut
    }
    /// Connect to WhatsApp Web, starting a new session.
    pub fn connect_new() -> impl Future<Output=WaResult<Self>> {
        Self::ws_connect(SessionState::pending_new())
    }
    /// Connect to WhatsApp Web, reusing an old persistent session.
    pub fn connect_persistent(sess: PersistentSession) -> impl Future<Output=WaResult<Self>> {
        Self::ws_connect(SessionState::pending_persistent(sess))
    }
}
impl WebConnection {
    // This `impl` block: low-level protocol functions, like sending
    // and receiving different message types
    fn alloc_message_tag(&mut self) -> String {
        let tag = self.tag_counter;
        self.tag_counter += 1;
        tag.to_string()
    }
    fn send_ws_message(&mut self, msg: WebsocketMessage, ct: CallbackType) {
        self.callbacks.insert(msg.tag.clone().into(), ct);
        self.ws_outbox.push_back(msg.serialize());
    }
    pub(crate) fn increment_epoch(&mut self) {
        self.epoch += 1;
    }
    pub(crate) fn send_json_message(&mut self, message: JsonValue, ct: CallbackType) {
        let tag = self.alloc_message_tag();
        debug!("--> JSON (tag {}): {:?}", tag, message);
        self.send_ws_message(WebsocketMessage {
            tag: tag.into(),
            payload: WebsocketMessagePayload::Json(message)
        }, ct);
    }
    pub(crate) fn send_node_message(&mut self, tag: Option<String>, metric: WebsocketMessageMetric, node: Node, ct: CallbackType) -> Result<()> {
        debug!("--> node (tag {:?}): {:?}", tag, node);
        self.send_binary_message(tag, metric, &node.serialize(), ct)?;
        Ok(())
    }
    pub(crate) fn send_binary_message(&mut self, tag: Option<String>, metric: WebsocketMessageMetric, message: &[u8], ct: CallbackType) -> Result<()> {
        let encrypted_message = if let SessionState::Established { ref persistent_session } = self.session_state {
            crypto::sign_and_encrypt_message(&persistent_session.enc, &persistent_session.mac, &message)
        } else {
            Err(WaError::InvalidSessionState)?
        };

        let tag = tag.unwrap_or_else(|| self.alloc_message_tag());
        debug!("--> binary (tag {}): {:?}", tag, message);
        self.send_ws_message(WebsocketMessage {
            tag: tag.into(),
            payload: WebsocketMessagePayload::BinaryEphemeral(metric, &encrypted_message)
        }, ct);
        Ok(())
    }
    pub(crate) fn send_set_app_event(&mut self, metric: WebsocketMessageMetric, evt: AppEvent) -> Result<()> {
        let msg = AppMessage::MessagesEvents(Some(MessageEventType::Set), vec![evt]);
        self.send_app_message(None, metric, msg, CallbackType::Noop)?;
        Ok(())
    }
    pub(crate) fn send_app_message(&mut self, tag: Option<String>, metric: WebsocketMessageMetric, app_message: AppMessage, ct: CallbackType) -> Result<()> {
        self.epoch += 1;
        let epoch = self.epoch;
        self.send_node_message(tag, metric, app_message.serialize(epoch), ct)?;
        Ok(())
    }
    pub(crate) fn send_group_command(&mut self, command: GroupCommand, participants: Vec<Jid>) -> Result<()> {
        let tag = self.alloc_message_tag();

        let app_event = AppEvent::GroupCommand { inducer: self.user_jid.clone().unwrap(), participants, id: tag.clone(), command };

        self.send_app_message(
            Some(tag),
            WebsocketMessageMetric::Group,
            AppMessage::MessagesEvents(Some(MessageEventType::Set), vec![app_event]),
            CallbackType::Noop
        )?;
        Ok(())
    }
    fn decrypt_binary_message(&mut self, encrypted_message: &[u8]) -> Result<Vec<u8>> {
        trace!("Decrypting binary message: {:?}", encrypted_message);
        if let SessionState::Established { ref persistent_session } = self.session_state {
            crypto::verify_and_decrypt_message(&persistent_session.enc[..], &persistent_session.mac[..], &encrypted_message)
        } else {
            Err(WaError::InvalidSessionState)?
        }
    }
}
impl WebConnection {
    // This `impl` block: CallbackType impls
    fn ct_login_new(&mut self, p: JsonValue) -> Result<()> {
        let resp = json_protocol::parse_init_response(&p)?;
        if let SessionState::PendingNew {
            ref public_key,
            ref client_id,
            ..
        } = self.session_state {
            let qrc = QrCode::new(
                format!("{},{},{}", resp, base64::encode(&public_key), base64::encode(&client_id))
                )?;
            self.outbox.push_back(WaEvent::ScanCode(qrc));
        }
        else {
            return Err(WaError::InvalidSessionState);
        }
        Ok(())
    }
    fn ct_login_persistent(&mut self, p: JsonValue) -> Result<()> {
        json_protocol::parse_response_status(&p)?;
        if let SessionState::PendingPersistent { ref persistent_session } = self.session_state {
            let login_command = json_protocol::build_takeover_request(
                persistent_session.client_token.as_str(),
                persistent_session.server_token.as_str(),
                &base64::encode(&persistent_session.client_id)
                );
            self.send_json_message(login_command, CallbackType::CheckStatus);
        }
        else {
            return Err(WaError::InvalidSessionState);
        }
        Ok(())
    }
    fn ct_check_status(&mut self, p: JsonValue) -> Result<()> {
        json_protocol::parse_response_status(&p)?;
        Ok(())
    }
    fn ct_process_ack(&mut self, p: JsonValue, mid: MessageId) -> Result<()> {
        use crate::message::{MessageAckLevel, MessageAckSide, MessageAck};
        use crate::json_protocol::LowLevelAck;

        debug!("Processing message ack for {}", mid.0);

        let LowLevelAck { status_code, timestamp } = LowLevelAck::deserialize(&p)?;
        if status_code != 200 {
            self.outbox.push_back(WaEvent::MessageSendFail {
                mid,
                status: status_code
            });
        }
        else {
            let mack = MessageAck {
                level: MessageAckLevel::Sent,
                time: Some(timestamp),
                id: mid.clone(),
                side: MessageAckSide::Here(Peer::Individual(
                        self.user_jid.clone().ok_or(WaError::NoJidYet)?
                ))
            };
            self.outbox.push_back(WaEvent::MessageAck(mack));
        }
        Ok(())
    }
    fn ct_messages_before(&mut self, uu: Uuid, n: Node) -> Result<()> {
        let resp = node_protocol::parse_message_response(n);
        self.outbox.push_back(WaEvent::MessageHistory {
            uuid: uu,
            history: resp
        });
        Ok(())
    }
    fn ct_file_upload(&mut self, p: JsonValue, uuid: Uuid) -> Result<()> {
        let resp = json_protocol::parse_file_upload_response(&p)?;
        self.outbox.push_back(WaEvent::FileUpload {
            uuid,
            url: resp.into()
        });
        Ok(())
    }
    fn ct_media_conn(&mut self, p: JsonValue, uuid: Uuid) -> Result<()> {
        let resp = json_protocol::parse_media_conn_response(&p)?;
        self.outbox.push_back(WaEvent::MediaConn {
            uuid,
            auth: resp.0.into(),
            ttl: chrono::Utc::now().naive_utc() + chrono::Duration::milliseconds(resp.1),
            hosts:  resp.2.into_iter().map(|x| x.into()).collect(),
        });
        Ok(())
    }
    fn ct_profile_picture(&mut self, p: JsonValue, jid: Jid) -> Result<()> {
        let pict = json_protocol::parse_profile_picture_response(&p);
        self.outbox.push_back(WaEvent::ProfilePicture {
            jid,
            url: pict.map(|x| x.to_owned())
        });
        Ok(())
    }
    fn ct_profile_status(&mut self, p: JsonValue, jid: Jid) -> Result<()> {
        let st = json_protocol::parse_profile_status_response(&p);
        if let Some(st) = st {
            self.outbox.push_back(WaEvent::ProfileStatus {
                jid,
                status: st.into(),
                was_request: true
            });
        }
        else {
            warn!("Got empty profile status response for {}", jid);
        }
        Ok(())
    }
    fn ct_group_metadata(&mut self, j: JsonValue) -> Result<()> {
        let resp = json_protocol::parse_group_metadata_response(&j);
        self.outbox.push_back(WaEvent::GroupMetadata {
            meta: resp
        });
        Ok(())
    }
}
impl WebConnection {
    // This `impl` block: functions that get called to deal
    // with different messages coming down the wire
    fn handle_callback_json(&mut self, j: JsonValue, c: CallbackType) -> Result<()> {
        use self::CallbackType::*;
        let ret = match c.clone() {
            LoginNew => self.ct_login_new(j),
            LoginPersistent => self.ct_login_persistent(j),
            CheckStatus => self.ct_check_status(j),
            ProcessAck { mid }  => self.ct_process_ack(j, mid),
            FileUpload { uuid } => self.ct_file_upload(j, uuid),
            MediaConn { uuid } => self.ct_media_conn(j, uuid),
            ProfilePicture { jid } => self.ct_profile_picture(j, jid),
            ProfileStatus { jid } => self.ct_profile_status(j, jid),
            GroupMetadata => self.ct_group_metadata(j),
            Noop => Ok(()),
            x => Err(WaError::InvalidPayload(format!("{:?}", x), "json"))?
        };
        if let Err(e) = ret {
            error!("Handler for {:?} failed: {}", c, e);
            Err(e)?
        }
        Ok(())
    }
    fn handle_callback_node(&mut self, n: Node, c: CallbackType) -> Result<()> {
        use self::CallbackType::*;
        let ret: Result<()> = match c.clone() {
            MessagesBefore { uuid } => self.ct_messages_before(uuid, n),
            Noop => Ok(()),
            x => Err(WaError::InvalidPayload(format!("{:?}", x), "node"))?
        };
        if let Err(e) = ret {
            error!("Handler for {:?} failed: {}", c, e);
            Err(e)?
        }
        Ok(())
    }
    fn handle_connection_ack(&mut self, user_jid: Jid, client_token: &str, server_token: &str, secret: Option<&str>) -> Result<(PersistentSession, Jid)> {
        debug!("Handling connection ack");
        let (new_session_state, persistent_session, user_jid) = match self.session_state {
            SessionState::PendingNew { ref mut private_key, ref client_id, .. } => {
                let secret = base64::decode(secret.ok_or(WaError::JsonFieldMissing("secret"))?)?;
                let (enc, mac) = crypto::calculate_secret_keys(&secret, private_key.take().unwrap())?;

                self.user_jid = Some(user_jid);

                let persistent_session = PersistentSession {
                    client_token: client_token.to_string(),
                    server_token: server_token.to_string(),
                    client_id: *client_id,
                    enc,
                    mac
                };

                (SessionState::Established { persistent_session: persistent_session.clone() }, persistent_session, self.user_jid.clone())
            }
            SessionState::PendingPersistent { ref persistent_session } => {
                self.user_jid = Some(user_jid);

                let new_persistent_session = PersistentSession {
                    client_id: persistent_session.client_id,
                    enc: persistent_session.enc,
                    mac: persistent_session.mac,
                    client_token: client_token.to_string(),
                    server_token: server_token.to_string()
                };

                (SessionState::Established { persistent_session: new_persistent_session.clone() }, new_persistent_session, self.user_jid.clone())
            }
            _ => Err(WaError::InvalidSessionState)?
        };
        self.session_state = new_session_state;
        Ok((persistent_session, user_jid.unwrap()))
    }
    fn handle_server_challenge(&mut self, challenge: &[u8]) -> Result<()> {
        trace!("Got server challenge: {:?}", challenge);
        debug!("Handling server challenge");
        let persist = match self.session_state {
            SessionState::Established { ref persistent_session, .. } => persistent_session,
            SessionState::PendingPersistent { ref persistent_session, .. } => persistent_session,
            _ => Err(WaError::InvalidSessionState)?
        };

        let signature = crypto::sign_challenge(&persist.mac, challenge);
        let resp = json_protocol::build_challenge_response(
            persist.server_token.as_str(),
            &base64::encode(&persist.client_id),
            signature.as_ref());

        self.send_json_message(resp, CallbackType::CheckStatus);
        Ok(())
    }
    fn generate_empty_ack(&mut self, mid: String) -> Result<()> {
        use crate::message::{MessageAckLevel, MessageAckSide, MessageAck};

        let mack = MessageAck {
            level: MessageAckLevel::PendingSend,
            time: None,
            id: MessageId(mid),
            side: MessageAckSide::Here(Peer::Individual(
                    self.user_jid.clone().ok_or(WaError::NoJidYet)?
                    ))
        };
        self.outbox.push_back(WaEvent::MessageAck(mack));
        Ok(())
    }
    fn on_server_message(&mut self, r: ServerMessage) -> Result<()> {
        use self::ServerMessage::*;

        match r {
            ConnectionAck {
                user_jid,
                client_token,
                server_token,
                secret
            } => {
                let (persistent, jid) = self.handle_connection_ack(user_jid, client_token, server_token, secret)?;
                self.outbox.push_back(WaEvent::SessionEstablished { persistent, jid })
            },
            ChallengeRequest(challenge) => {
                self.handle_server_challenge(&challenge)?;
            },
            Disconnect(kind) => {
                let reason = if kind.is_some() {
                    DisconnectReason::Replaced
                }
                else {
                    DisconnectReason::Removed
                };
                warn!("Received disconnection message from server");
                Err(WaError::Disconnected(reason))?
            },
            oth => {
                let self_jid = self.user_jid.as_ref();
                let events = WaEvent::from_server_message(oth, self_jid);
                self.outbox.extend(events);
            }
        }
        Ok(())
    }
}
impl WebConnection {
    // This `impl` block: websocket callback handlers
    fn on_connected(&mut self) {
        self.outbox.push_back(WaEvent::WebsocketConnected);
        let (client_id, callback_type) = match self.session_state {
            SessionState::PendingNew { client_id, .. } => (client_id, CallbackType::LoginNew),
            SessionState::PendingPersistent { ref persistent_session, .. } => (persistent_session.client_id, CallbackType::LoginPersistent),
            _ => panic!("on_connected called with invalid session state")
        };
        let init_command = json_protocol::build_init_request(base64::encode(&client_id).as_str());
        self.send_json_message(init_command, callback_type);
    }
    fn on_ping_timer(&mut self) {
        self.ws_outbox.push_front(Message::Text("?,,".into()));
        let deadline = tokio::time::Instant::from_std(Instant::now() + Duration::new(3, 0));
        self.response_timer = Some(tokio::time::delay_until(deadline));
    }
    fn on_message(&mut self, m: Message) -> Result<()> {
        trace!("<-- {:?}", m);
        let message = match WebsocketMessage::deserialize(&m) {
            Some(m) => m,
            None => {
                error!("Failed to deserialize websocket message!");
                warn!("Message contents: {:?}", m);
                return Ok(());
            }
        };
        match message.payload {
            WebsocketMessagePayload::Json(p) => {
                if let Some(ct) = self.callbacks.remove(&message.tag as &str) {
                    debug!("<-- JSON (tag {} -> {:?}): {}", message.tag, ct, &p);
                    self.handle_callback_json(p, ct)?;
                }
                else {
                    debug!("<-- JSON (tag {}): {}", message.tag, &p);
                    match ServerMessage::deserialize(&p) {
                        Ok(r) => {
                            self.on_server_message(r)?;
                        },
                        Err(e) => {
                            debug!("Failed to deserialize JSON: {}", e);
                        }
                    }
                }
            },
            WebsocketMessagePayload::BinarySimple(p) => {
                let dec = match self.decrypt_binary_message(p) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to decrypt binary message payload: {}", e);
                        debug!("Payload: {:?}", p);
                        return Ok(());
                    }
                };
                let payload = match Node::deserialize(&dec) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to deserialize node: {}", e);
                        warn!("Payload: {:?}", dec);
                        return Ok(());
                    },
                };
                if let Some(ct) = self.callbacks.remove(&message.tag as &str) {
                    debug!("<-- node (tag {} -> {:?}): {:?}", message.tag, ct, &payload);
                    self.handle_callback_node(payload, ct)?;
                }
                else {
                    debug!("<-- node (tag {}): {:?}", message.tag, &payload);
                    match AppMessage::deserialize(payload) {
                        Ok(p) => {
                            let events = WaEvent::from_app_message(p);
                            self.outbox.extend(events);
                        },
                        Err(e) => {
                            error!("Failed to deserialize appmessage: {}", e);
                        }
                    }
                }
            },
            WebsocketMessagePayload::Empty => {
                debug!("<-- empty (tag {})", message.tag);
                if message.tag.len() > 10 {
                    debug!("Interpreting empty payload as an ack for {}", message.tag);
                    self.generate_empty_ack(message.tag.to_string())?;
                }
            },
            WebsocketMessagePayload::Pong => {
                debug!("<-- pong (tag {})", message.tag);
            },
            WebsocketMessagePayload::BinaryEphemeral(a, b) => {
                // FIXME: I don't know what this is, but why are we ignoring it?
                debug!("<-- binary ephemeral (tag {}): metric {:?}, {:?}", message.tag, a, b);
            },
        }
        Ok(())
    }
}
