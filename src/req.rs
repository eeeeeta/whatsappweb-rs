//! Requests to be made over a WhatsApp Web connection.

use crate::message::{MessageId, ChatMessage, Peer};
use crate::conn::{WebConnection, CallbackType};
use crate::{Jid, PresenceStatus, GroupParticipantsChange, ChatAction, MediaType};
use crate::websocket_protocol::WebsocketMessageMetric;
use crate::node_protocol::{AppEvent, AppMessage, MessageEventType, GroupCommand, Query};
use crate::json_protocol;
use crate::errors::*;

use std::pin::Pin;

pub use uuid::Uuid;

pub enum WaRequest {
    MessagePlayed {
        mid: MessageId,
        peer: Peer
    },
    MessageRead {
        mid: MessageId,
        peer: Peer
    },
    SetPresence {
        presence: PresenceStatus,
        jid: Option<Jid>
    },
    SubscribePresence(Jid),
    SetStatus(String),
    SetNotifyName(String),
    SetProfileBlocked {
        jid: Jid,
        blocked: bool
    },
    ChatAction {
        jid: Jid,
        action: ChatAction
    },
    SendMessage(ChatMessage),
    CreateGroup {
        subject: String,
        participants: Vec<Jid>
    },
    ChangeGroupParticipants {
        jid: Jid,
        change: GroupParticipantsChange,
        participants: Vec<Jid>
    },
    /// Get message history for a given chat.
    ///
    /// This request returns history before the given message ID,
    /// up to a total of `count` messages.
    ///
    /// If it's successful, the returned history will result in
    /// a `WebEvent::MessageHistory` event, with the `uuid` supplied
    /// here.
    GetMessageHistoryBefore {
        /// The JID of the chat to get history for.
        jid: Jid,
        /// The message ID to receive history before.
        mid: MessageId,
        /// The maximum amount of messages to receive.
        count: u16,
        /// An identifier for this history request.
        uuid: Uuid,
    },
    RequestFileUpload {
        hash: Vec<u8>,
        media_type: MediaType,
        uuid: Uuid,
    },
    RequestMediaConn {
        uuid: Uuid,
    },
    GetProfilePicture(Jid),
    GetProfileStatus(Jid),
    GetGroupMetadata(Jid),
}
impl WaRequest {
    pub(crate) fn apply(self, mut conn: Pin<&mut WebConnection>) -> Result<()> {
        use self::WaRequest::*;

        match self {
            MessagePlayed { mid, peer } => {
                conn.increment_epoch();
                conn.send_set_app_event(WebsocketMessageMetric::Received, AppEvent::MessagePlayed { id: mid, peer })?;
            },
            MessageRead { mid, peer } => {
                conn.send_set_app_event(WebsocketMessageMetric::Read, AppEvent::MessageRead { id: mid, peer })?;
            },
            SetPresence { presence, jid } => {
                conn.send_set_app_event(WebsocketMessageMetric::Presence, AppEvent::PresenceChange(presence, jid))?;
            },
            SetStatus(st) => {
                conn.send_set_app_event(WebsocketMessageMetric::Status, AppEvent::StatusChange(st))?;
            },
            SetNotifyName(st) => {
                conn.send_set_app_event(WebsocketMessageMetric::Profile, AppEvent::NotifyChange(st))?;
            },
            SetProfileBlocked { jid, blocked } => {
                let unblock = !blocked;
                conn.send_set_app_event(WebsocketMessageMetric::Block, AppEvent::BlockProfile { unblock, jid })?;
            },
            ChatAction { jid, action } => {
                conn.send_set_app_event(WebsocketMessageMetric::Chat, AppEvent::ChatAction(jid, action))?;
            },
            SendMessage(msg) => {
                if !msg.direction.is_sending() {
                    Err(WaError::InvalidDirection)?
                }
                let mid = msg.id.clone();
                let amsg = AppMessage::MessagesEvents(Some(MessageEventType::Relay), vec![AppEvent::Message(msg)]);
                conn.send_app_message(Some(mid.0.clone()), WebsocketMessageMetric::Message, amsg, CallbackType::ProcessAck { mid })?;
            },
            CreateGroup { subject, participants } => {
                conn.send_group_command(GroupCommand::Create(subject), participants)?;
            },
            ChangeGroupParticipants { jid, change, participants } => {
                conn.send_group_command(GroupCommand::ParticipantsChange(jid, change), participants)?;
            },
            RequestFileUpload { hash, media_type, uuid } => {
                let req = json_protocol::build_file_upload_request(&hash, media_type);
                conn.send_json_message(req, CallbackType::FileUpload { uuid });
            },
            RequestMediaConn { uuid } => {
                let req = json_protocol::build_media_conn_request();
                conn.send_json_message(req, CallbackType::MediaConn { uuid });
            }
            GetMessageHistoryBefore { jid, mid, count, uuid } => {
                let msg = AppMessage::Query(Query::MessagesBefore { jid, id: mid.0, count });
                conn.send_app_message(None, WebsocketMessageMetric::QueryMessages, msg, CallbackType::MessagesBefore { uuid })?;
            },
            GetProfilePicture(jid) => {
                let req = json_protocol::build_profile_picture_request(&jid);
                conn.send_json_message(req, CallbackType::ProfilePicture { jid });
            },
            GetProfileStatus(jid) => {
                let req = json_protocol::build_profile_status_request(&jid);
                conn.send_json_message(req, CallbackType::ProfileStatus { jid });
            },
            GetGroupMetadata(jid) => {
                let req = json_protocol::build_group_metadata_request(&jid);
                conn.send_json_message(req, CallbackType::GroupMetadata);
            },
            SubscribePresence(jid) => {
                let req = json_protocol::build_presence_subscribe(&jid);
                conn.send_json_message(req, CallbackType::Noop);
            },
        }
        Ok(())
    }
}
