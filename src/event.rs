//! Events that can occur when connected to WhatsApp Web.

use qrcode::QrCode;
use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::session::PersistentSession;
use crate::message::{MessageId, ChatMessage};
use crate::{Contact, Jid, Chat, ChatAction, GroupParticipantsChange, PresenceStatus, GroupMetadata};
use crate::json_protocol::ServerMessage;
use crate::node_protocol::AppMessage;
use crate::errors::Result;

/// An event arising from a WhatsApp Web connection.
pub enum WaEvent {
    /// The underlying websocket has connected.
    ///
    /// Note that this does not mean you can send messages yet;
    /// a session must still be negotiated!
    WebsocketConnected,
    /// A QR code is ready for the user to scan.
    ///
    /// This usually needs to be scanned within a few seconds of this
    /// message being received in order to work.
    ScanCode(QrCode),
    /// A session has been successfully established, and the connection
    /// is now ready to use.
    ///
    /// You should store the value of `persistent` somewhere,
    /// and use it to avoid scanning the QR code in future.
    SessionEstablished {
        /// Persistent session data for future sessions.
        persistent: PersistentSession,
        /// The JID of the logged in user.
        jid: Jid
    },
    /// A message was received.
    Message {
        /// Was this message just received, or was it part of the backlog?
        is_new: bool,
        /// The message object.
        msg: ChatMessage
    },
    /// Initial burst of contacts from the user's address book.
    InitialContacts(Vec<Contact>),
    /// A contact was added or modified.
    AddContact(Contact),
    /// The contact with the given JID was deleted.
    DeleteContact(Jid),
    /// Initial burst of chats open in WhatsApp.
    InitialChats(Vec<Chat>),
    /// Something happened to one of the chats in the chat list.
    ChatEvent {
        /// The JID of the relevant chat.
        jid: Jid,
        /// What happened.
        event: ChatAction
    },
    /// The presence of some known WhatsApp user changed.
    PresenceChange {
        /// The JID of the user.
        jid: Jid,
        /// Their new presence.
        presence: PresenceStatus,
        /// The timestamp associated with this change.
        ///
        /// For example, if they're offline, this timestamp
        /// is used as the 'last seen' value.
        ts: Option<NaiveDateTime>
    },
    /// A message was acknowledged (as being sent, delivered, read, ...)
    MessageAck(crate::message::MessageAck),
    /// A user changed their status text.
    ///
    /// This event is also fired when a user's status is manually requested.
    ProfileStatus {
        /// The JID of the user.
        jid: Jid,
        /// Their new status text.
        status: String,
        /// Whether or not the user actually just *changed* their status,
        /// as opposed to us requesting it.
        was_request: bool
    },
    /// The user was invited to, and joined, a new group chat.
    GroupIntroduce {
        /// Whether the group chat was newly created (`false` implies
        /// it existed before, and the user has just been invited to it)
        newly_created: bool,
        /// JID of the user who invited us.
        inducer: Jid,
        /// Group metadata.
        meta: GroupMetadata
    },
    /// Group metadata was returned from a metadata query.
    GroupMetadata {
        /// The relevant group metadata.
        meta: Result<GroupMetadata>
    },
    /// The participants in a group chat changed.
    GroupParticipantsChange {
        /// The JID of the group chat.
        jid: Jid,
        /// What change occurred.
        change: GroupParticipantsChange,
        /// Who made this change.
        ///
        /// FIXME: under what circumstances is this `None`?
        inducer: Option<Jid>,
        /// Which participants the change applies to.
        participants: Vec<Jid>,
    },
    /// The subject of a group chat changed.
    GroupSubjectChange {
        /// The JID of the group chat.
        jid: Jid,
        /// The new subject.
        subject: String,
        /// The timestamp of this change (i.e. when the new subject
        /// was applied).
        ts: NaiveDateTime,
        /// The person who made the subject change.
        inducer: Jid
    },
    /// Someone changed or removed their profile picture.
    PictureChange {
        /// The JID of the relevant user.
        jid: Jid,
        /// Whether the picture was removed or not.
        removed: bool,
    },
    /// A profile picture was returned from a query.
    ProfilePicture {
        /// The JID of the relevant user.
        jid: Jid,
        /// The URL of their profile picture, if they have one.
        url: Option<String>
    },
    /// A message might have failed to send.
    ///
    /// This is a 'might' because we haven't seen this failure case in the wild,
    /// but we think it might exist.
    MessageSendFail {
        /// The message ID that didn't send properly.
        mid: MessageId,
        /// The returned status code from WhatsApp.
        status: u16
    },
    /// Message history was successfully retrieved.
    MessageHistory {
        /// The UUID associated with the history request.
        uuid: Uuid,
        /// The returned history messages.
        history: Result<Vec<ChatMessage>>
    },
    /// A file upload URL was successfully retrieved.
    FileUpload {
        /// The UUID associated with the file upload request.
        uuid: Uuid,
        /// The URL to upload the file to.
        url: String
    },
    MediaConn {
        /// The UUID associated with the media conn request
        uuid: Uuid,
        /// The auth string to be used on the media upload
        auth: String,
        /// The point in time when the auth stops being valid
        ttl: NaiveDateTime,
        /// List of hosts available for the upload
        hosts: Vec<String>
    },
    /// The phone's battery level changed to a number of percentage points.
    BatteryLevel(u8)
}
impl WaEvent {
    pub(crate) fn from_app_message(a: AppMessage) -> Vec<Self> {
        use self::AppMessage::*;
        match a {
            Contacts(contacts) => {
                vec![WaEvent::InitialContacts(contacts)]
            }
            Chats(chats) => {
                vec![WaEvent::InitialChats(chats)]
            }
            MessagesEvents(event_type, events) => {
                events.into_iter()
                    .filter_map(|event| {
                        use crate::node_protocol::{AppEvent, MessageEventType};
                        match event {
                            AppEvent::Message(message) => Some(WaEvent::Message {
                                is_new: event_type == Some(MessageEventType::Relay),
                                msg: message
                            }),
                            AppEvent::MessageAck(message_ack) => Some(WaEvent::MessageAck(message_ack)),
                            AppEvent::ContactDelete(jid) => Some(WaEvent::DeleteContact(jid)),
                            AppEvent::ContactAddChange(contact) => Some(WaEvent::AddContact(contact)),
                            AppEvent::ChatAction(jid, action) => Some(WaEvent::ChatEvent {
                                jid,
                                event: action
                            }),
                            AppEvent::Battery(level) => Some(WaEvent::BatteryLevel(level)),
                            ae => {
                                warn!("Received supposedly unreachable AppEvent: {:?}", ae);
                                None
                            },
                        }
                    })
                    .collect()
            },
            Query(c) => {
                warn!("Received a query AppMessage: {:?}", c);
                vec![]
            }
        }
    }
    pub(crate) fn from_server_message(r: ServerMessage, own_jid: Option<&Jid>) -> Vec<Self> {
        use self::ServerMessage::*;
        match r {
            PresenceChange { jid, status, time } => {
                vec![WaEvent::PresenceChange {
                    jid,
                    presence: status,
                    ts: time.and_then(|timestamp| if timestamp != 0 {
                        Some(NaiveDateTime::from_timestamp(timestamp, 0))
                    } else {
                        None
                    })
                }]
            },
            MessageAck { message_id, level, sender, receiver, participant, time } => {
                if let Some(own_jid) = own_jid {
                    vec![WaEvent::MessageAck(crate::message::MessageAck::from_server_message(
                            message_id,
                            level,
                            sender,
                            receiver,
                            participant,
                            time,
                            own_jid))]
                }
                else {
                    warn!("Discarding message acks due to unknown own jid!");
                    vec![]
                }
            },
            MessageAcks { message_ids, level, sender, receiver, participant, time } => {
                if let Some(own_jid) = own_jid {
                    message_ids
                        .into_iter()
                        .map(|message_id| {
                            WaEvent::MessageAck(crate::message::MessageAck::from_server_message(
                                message_id,
                                level,
                                sender.clone(),
                                receiver.clone(),
                                participant.clone(),
                                time,
                                own_jid))
                        })
                        .collect()
                }
                else {
                    warn!("Discarding message acks due to unknown own jid!");
                    vec![]
                }
            },
            GroupIntroduce { newly_created, inducer, meta } => {
                vec![WaEvent::GroupIntroduce {
                    newly_created, inducer, meta
                }]
            }
            GroupParticipantsChange { group, change, inducer, participants } => {
                vec![WaEvent::GroupParticipantsChange {
                    jid: group,
                    change, inducer, participants
                }]
            }
            StatusChange(jid, status) => {
                vec![WaEvent::ProfileStatus { jid, status, was_request: false }]
            }
            PictureChange { jid, removed } => {
                vec![WaEvent::PictureChange { jid, removed }]
            }
            GroupSubjectChange { group, subject, subject_time, subject_owner } => {
                let subject_time = NaiveDateTime::from_timestamp(subject_time, 0);
                vec![WaEvent::GroupSubjectChange {
                    jid: group,
                    subject,
                    ts: subject_time,
                    inducer: subject_owner
                }]
            },
            _ => panic!("WaEvent cannot convert from this ServerMessage")
        }
    }
}
