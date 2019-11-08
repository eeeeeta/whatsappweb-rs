use std::time::Duration;
use std::str::FromStr;

use protobuf;
use protobuf::Message;
use ring::rand::{SystemRandom, SecureRandom};
use chrono::NaiveDateTime;

use super::message_wire;
use super::Jid;
use crate::errors::*;

macro_rules! get_fileinfo {
    ($msg:ident) => {
        FileInfo {
            url: $msg.take_url(),
            mime: $msg.take_mimetype(),
            sha256: $msg.take_fileSha256(),
            enc_sha256: $msg.take_fileEncSha256(),
            size: $msg.get_fileLength() as usize,
            key: $msg.take_mediaKey(),
        }
    }
}
macro_rules! get_caption {
    ($msg:ident) => {
        if $msg.has_caption() {
            Some($msg.take_caption())
        }
        else {
            None
        }
    }
}
macro_rules! get_context_info {
    ($msg:expr) => {
        if $msg.has_contextInfo() {
            QuotedChatMessage::from_context_info($msg.take_contextInfo())?
        }
        else {
            None
        }
    }
}
#[derive(Debug, Clone, PartialOrd, PartialEq)]
pub struct MessageId(pub String);

impl MessageId {
    pub fn generate() -> MessageId {
        let mut message_id_binary = vec![0u8; 12];
        message_id_binary[0] = 0x3E;
        message_id_binary[1] = 0xB0;
        SystemRandom::new().fill(&mut message_id_binary[2..]).unwrap();
        MessageId(message_id_binary.iter().map(|b| format!("{:X}", b)).collect::<Vec<_>>().concat())
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    Individual(Jid),
    Group { group: Jid, participant: Jid },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerAck {
    Individual(Jid),
    GroupIndividual { group: Jid, participant: Jid },
    GroupAll(Jid),
}

/// Which direction a message is going - i.e. sending or receiving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Direction {
    /// We're sending this message.
    Sending(Jid),
    /// We've received this message.
    Receiving(Peer),
}

impl Direction {
    pub fn is_sending(&self) -> bool {
        if let Direction::Sending(_) = self {
            true
        }
        else {
            false
        }
    }
    fn parse(msg: &mut message_wire::WebMessageInfo) -> Result<Direction> {
        let mut key = msg.take_key();
        let remote_jid = Jid::from_str(&key.take_remoteJid())?;
        Ok(if key.get_fromMe() {
            Direction::Sending(remote_jid)
        } else {
            Direction::Receiving(if msg.has_participant() {
                Peer::Group { group: remote_jid, participant: Jid::from_str(&msg.take_participant())? }
            } else {
                Peer::Individual(remote_jid)
            })
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub enum MessageAckLevel {
    PendingSend = 0,
    Sent = 1,
    Received = 2,
    Read = 3,
    Played = 4,
    Error
}

#[derive(Debug)]
pub enum MessageAckSide {
    Here(Peer),
    There(PeerAck),
}

#[derive(Debug)]
pub struct MessageAck {
    pub level: MessageAckLevel,
    pub time: Option<NaiveDateTime>,
    pub id: MessageId,
    pub side: MessageAckSide,
}

impl MessageAck {
    pub fn from_server_message(message_id: &str, level: MessageAckLevel, sender: Jid, receiver: Jid, participant: Option<Jid>, time: i64, own_jid: &Jid) -> MessageAck {
        MessageAck {
            level,
            time: Some(NaiveDateTime::from_timestamp(time, 0)),
            id: MessageId(message_id.to_string()),
            side: if own_jid == &sender {
                MessageAckSide::There(if let Some(participant) = participant {
                    PeerAck::GroupIndividual { group: receiver, participant }
                } else {
                    PeerAck::Individual(receiver)
                })
            } else {
                MessageAckSide::Here(if let Some(participant) = participant {
                    Peer::Group { group: sender, participant }
                } else {
                    Peer::Individual(sender)
                })
            },
        }
    }

    pub fn from_app_message(message_id: MessageId, level: MessageAckLevel, jid: Jid, participant: Option<Jid>, owner: bool) -> MessageAck {
        MessageAck {
            level,
            time: None,
            id: message_id,
            side: if owner {
                MessageAckSide::There(if jid.is_group {
                    PeerAck::GroupAll(jid)
                } else {
                    PeerAck::Individual(jid)
                })
            } else {
                MessageAckSide::Here(if let Some(participant) = participant {
                    Peer::Group { group: jid, participant }
                } else {
                    Peer::Individual(jid)
                })
            },
        }
    }
}

/// Information about a file.
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// The URL where this file is hosted.
    pub url: String,
    /// The file's MIME type.
    pub mime: String,
    /// The SHA256 of this file when encrypted (as you'll download it).
    pub sha256: Vec<u8>,
    /// The SHA256 of this file when decrypted.
    ///
    /// **Note:** This seems to be wrong for some unknown reason.
    pub enc_sha256: Vec<u8>,
    /// The file size, in bytes.
    pub size: usize,
    /// The key used to decrypt the message.
    pub key: Vec<u8>,
}

/// The content of a WhatsApp message.
#[derive(Debug, Clone)]
pub enum ChatMessageContent {
    /// A simple, plain text message.
    Text(String),
    /// An image.
    Image {
        /// Information about the image file itself.
        info: FileInfo,
        /// Height (presumably in pixels?)
        height: u32,
        /// Width (presumably in pixels?)
        width: u32,
        /// JPEG thumbnail of the image.
        thumbnail: Vec<u8>,
        /// Image caption, if there is one.
        caption: Option<String>
    },
    /// Some recorded audio.
    Audio {
        /// Information about the audio file itself.
        info: FileInfo,
        /// How long the file lasts.
        dur: Duration
    },
    /// Some recorded video.
    Video {
        /// Information about the video file itself.
        info: FileInfo,
        /// How long the file lasts.
        dur: Duration,
        /// Video caption, if there is one.
        caption: Option<String>
    },
    /// A generic uploaded file.
    Document {
        /// Information about the file itself.
        info: FileInfo,
        /// The supplied filename.
        filename: String
    },
    /// An uploaded contact card (i.e. vCard).
    Contact {
        /// The name of the contact.
        display_name: String,
        /// The contact card, in vCard format.
        vcard: String
    },
    /// A location somewhere on earth.
    Location {
        /// Degrees latitude.
        lat: f64,
        /// Degrees longitude.
        long: f64,
        /// A friendly name for the location, if provided.
        name: Option<String>,
        /// An address, if provided.
        address: Option<String>
    },
    /// A live broadcast of someone's location.
    LiveLocation {
        /// Degrees latitude.
        lat: f64,
        /// Degrees longitude.
        long: f64,
        /// Accuracy in metres.
        accuracy: Option<u32>,
        /// Speed, in m/s.
        speed: Option<f32>,
        /// Heading (in degrees from north).
        heading: Option<u32>,
        /// Sequence number of this broadcast.
        seq: Option<i64>
    },
    /// A redaction (i.e. someone 'deleting' a message they've sent).
    Redaction {
        /// The message ID being deleted.
        mid: MessageId
    },
    /// An unimplemented message type.
    /// 
    /// The text contains a debug version of the raw protobuf message.
    Unimplemented(String)
}

impl ChatMessageContent {
    pub fn quoted_description(&self) -> String {
        use self::ChatMessageContent::*;

        match *self {
            Text(ref st) => {
                st.to_owned()
            },
            Image { ref caption, .. } => {
                if let Some(c) = caption {
                    format!("Image: {}", c)
                }
                else {
                    "Image".into()
                }
            },
            Video { ref caption, .. } => {
                if let Some(c) = caption {
                    format!("Video: {}", c)
                }
                else {
                    "Video".into()
                }
            },
            Audio { .. } => "Audio".into(),
            Document { ref filename, .. } => format!("Document: {}", filename),
            Contact { ref display_name, .. } => format!("Contact: {}", display_name),
            Location { lat, long, .. } => format!("Location: ({}, {})", lat, long),
            LiveLocation { lat, long, .. } => format!("Live location: ({}, {})", lat, long),
            Redaction { ref mid } => format!("Redaction of {}", mid.0),
            Unimplemented(_) => format!("[unimplemented]"),
        }
    }
    pub fn take_caption(&mut self) -> Option<String> {
        use self::ChatMessageContent::*;

        match *self {
            Image { ref mut caption, .. } => caption.take(),
            Video { ref mut caption, .. } => caption.take(),
            _ => None
        }
    }
    fn from_proto(mut message: message_wire::Message) -> Result<ChatMessageContent> {
        use self::ChatMessageContent::*;

        if message.has_conversation() {
            return Ok(Text(message.take_conversation()));
        }
        if message.has_extendedTextMessage() {
            let mut etm = message.take_extendedTextMessage();
            return Ok(Text(etm.take_text()));
        }
        if message.has_imageMessage() {
            let mut imsg = message.take_imageMessage();
            let caption = if imsg.has_caption() {
                Some(imsg.take_caption())
            }
            else {
                None
            };
            return Ok(Image {
                info: get_fileinfo!(imsg),
                height: imsg.get_height(),
                width: imsg.get_width(),
                thumbnail: imsg.take_jpegThumbnail(),
                caption
            });
        }
        if message.has_audioMessage() {
            let mut amsg = message.take_audioMessage();
            return Ok(Audio {
                info: get_fileinfo!(amsg),
                dur: Duration::new(u64::from(amsg.get_seconds()), 0)
            });
        }
        if message.has_videoMessage() {
            let mut vmsg = message.take_videoMessage();
            return Ok(Video {
                info: get_fileinfo!(vmsg),
                dur: Duration::new(u64::from(vmsg.get_seconds()), 0),
                caption: get_caption!(vmsg)
            });
        }
        if message.has_audioMessage() {
            let mut amsg = message.take_audioMessage();
            return Ok(Audio {
                info: get_fileinfo!(amsg),
                dur: Duration::new(u64::from(amsg.get_seconds()), 0)
            });
        }
        if message.has_documentMessage() {
            let mut dmsg = message.take_documentMessage();
            return Ok(Document {
                info: get_fileinfo!(dmsg),
                filename: dmsg.take_fileName()
            });
        }
        if message.has_contactMessage() {
            let mut cmsg = message.take_contactMessage();
            return Ok(Contact {
                display_name: cmsg.take_displayName(),
                vcard: cmsg.take_vcard(),
            });
        }
        if message.has_protocolMessage() {
            let mut pmsg = message.take_protocolMessage();
            if pmsg.has_key() && pmsg.has_field_type() {
                return Ok(Redaction {
                    mid: MessageId(pmsg.take_key().take_id())
                });
            }
        }
        if message.has_locationMessage() {
            let mut lmsg = message.take_locationMessage();
            let name = if lmsg.has_name() { Some(lmsg.take_name()) } else { None };
            let address = if lmsg.has_address() { Some(lmsg.take_address()) } else { None };
            return Ok(Location {
                lat: lmsg.get_degreesLatitude(),
                long: lmsg.get_degreesLongitude(),
                name,
                address
            });
        }
        if message.has_liveLocationMessage() {
            let lmsg = message.take_liveLocationMessage();
            let accuracy = if lmsg.has_accuracyInMeters() { Some(lmsg.get_accuracyInMeters()) } else { None };
            let speed = if lmsg.has_speedInMps() { Some(lmsg.get_speedInMps()) } else { None };
            let heading = if lmsg.has_degreesClockwiseFromMagneticNorth() { Some(lmsg.get_degreesClockwiseFromMagneticNorth()) } else { None };
            let seq = if lmsg.has_sequenceNumber() { Some(lmsg.get_sequenceNumber()) } else { None };
            return Ok(LiveLocation {
                lat: lmsg.get_degreesLatitude(),
                long: lmsg.get_degreesLongitude(),
                accuracy, speed, heading, seq
            });
        }
        Ok(ChatMessageContent::Unimplemented(format!("{:?}", message)))
    }
    pub fn into_proto(self) -> message_wire::Message {
        let mut message = message_wire::Message::new();
        match self {
            ChatMessageContent::Text(text) => message.set_conversation(text),
            ChatMessageContent::Image { info, height, width, thumbnail, caption } => {
                let mut image_message = message_wire::ImageMessage::new();
                image_message.set_url(info.url);
                image_message.set_mimetype(info.mime);
                image_message.set_fileEncSha256(info.enc_sha256);
                image_message.set_fileSha256(info.sha256);
                image_message.set_fileLength(info.size as u64);
                image_message.set_mediaKey(info.key);
                image_message.set_height(height);
                image_message.set_width(width);
                image_message.set_jpegThumbnail(thumbnail);
                if let Some(caption) = caption {
                    image_message.set_caption(caption);
                }
                message.set_imageMessage(image_message);
            }
            ChatMessageContent::Document{ info, filename } => {
                let mut document_message = message_wire::DocumentMessage::new();
                document_message.set_url(info.url);
                document_message.set_mimetype(info.mime);
                document_message.set_fileEncSha256(info.enc_sha256);
                document_message.set_fileSha256(info.sha256);
                document_message.set_fileLength(info.size as u64);
                document_message.set_mediaKey(info.key);
                document_message.set_fileName(filename);
                message.set_documentMessage(document_message);
            }
            _ => unimplemented!()
        }

        message
    }
}
/// A message embedded in another.
#[derive(Debug)]
pub struct QuotedChatMessage {
    /// The person who originally sent the quoted message.
    pub participant: Jid,
    /// The message contents.
    pub content: ChatMessageContent
}
impl QuotedChatMessage {
    pub fn from_message(m: &mut message_wire::Message) -> Result<Option<Self>> {
        if m.has_extendedTextMessage() {
            return Ok(get_context_info!(m.mut_extendedTextMessage()));
        }
        if m.has_imageMessage() {
            return Ok(get_context_info!(m.mut_imageMessage()));
        }
        if m.has_audioMessage() {
            return Ok(get_context_info!(m.mut_audioMessage()));
        }
        if m.has_videoMessage() {
            return Ok(get_context_info!(m.mut_videoMessage()));
        }
        if m.has_documentMessage() {
            return Ok(get_context_info!(m.mut_documentMessage()));
        }
        if m.has_contactMessage() {
            return Ok(get_context_info!(m.mut_contactMessage()));
        }
        if m.has_locationMessage() {
            return Ok(get_context_info!(m.mut_locationMessage()));
        }
        Ok(None)
    }
    pub fn from_context_info(mut ctx: message_wire::ContextInfo) -> Result<Option<Self>> {
        if !ctx.has_participant() || !ctx.has_quotedMessage() {
            return Ok(None);
        }
        let participant: Jid = ctx.take_participant().parse()?;
        let content = ChatMessageContent::from_proto(ctx.take_quotedMessage())?;
        Ok(Some(Self { participant, content }))
    }
}

pub use crate::message_wire::WebMessageInfo_WEB_MESSAGE_INFO_STUBTYPE as MessageStubType;

/// A WhatsApp message.
#[derive(Debug)]
pub struct ChatMessage {
    /// The direction of this message - i.e. who we're sending it to, or receiving it from.
    pub direction: Direction,
    /// Timestamp, in UTC.
    pub time: NaiveDateTime,
    /// Message ID - probably unique.
    pub id: MessageId,
    /// The message contents.
    pub content: ChatMessageContent,
    /// The message this message is in reply to (or quoting), if any.
    pub quoted: Option<QuotedChatMessage>,
    /// If this message has a stub type, that stub type.
    pub stub_type: Option<MessageStubType>
}

impl ChatMessage {
    /// Create a new message, addressed to a specific JID.
    pub fn new(to: Jid, content: ChatMessageContent) -> Self {
        let message_id = MessageId::generate();
        Self {
            content,
            time: chrono::Utc::now().naive_utc(),
            direction: Direction::Sending(to),
            id: message_id,
            quoted: None,
            stub_type: None
        }
    }
    pub(crate) fn from_proto_binary(content: &[u8]) -> Result<ChatMessage> {
        let webmessage = protobuf::parse_from_bytes::<message_wire::WebMessageInfo>(content).map_err(|_| "Invalid Protobuf chatmessage")?;
        ChatMessage::from_proto(webmessage)
    }


    pub(crate) fn from_proto(mut webmessage: message_wire::WebMessageInfo) -> Result<ChatMessage> {
        debug!("Processing WebMessageInfo: {:?}", &webmessage);
        let mut msg = webmessage.take_message();
        let quoted = QuotedChatMessage::from_message(&mut msg)?;
        let stub_type = if webmessage.has_messageStubType() {
            Some(webmessage.get_messageStubType())
        }
        else {
            None
        };
        Ok(ChatMessage {
            id: MessageId(webmessage.mut_key().take_id()),
            direction: Direction::parse(&mut webmessage)?,
            time: NaiveDateTime::from_timestamp(webmessage.get_messageTimestamp() as i64, 0),
            content: ChatMessageContent::from_proto(msg)?,
            quoted, stub_type
        })
    }

    pub(crate) fn into_proto_binary(self) -> Vec<u8> {
        let webmessage = self.into_proto();
        webmessage.write_to_bytes().unwrap()
    }

    pub(crate) fn into_proto(self) -> message_wire::WebMessageInfo {
        let mut webmessage = message_wire::WebMessageInfo::new();
        let mut key = message_wire::MessageKey::new();

        key.set_id(self.id.0);
        match self.direction {
            Direction::Sending(jid) => {
                key.set_remoteJid(jid.to_message_jid());
                key.set_fromMe(true);
            }
            Direction::Receiving(_) => unimplemented!()
        }
        webmessage.set_key(key);

        webmessage.set_messageTimestamp(self.time.timestamp() as u64);

        webmessage.set_message(self.content.into_proto());

        webmessage.set_status(message_wire::WebMessageInfo_WEB_MESSAGE_INFO_STATUS::PENDING);
        debug!("Building WebMessageInfo: {:?}", &webmessage);

        webmessage
    }
}

impl Jid {
    pub fn to_message_jid(&self) -> String {
        self.id.to_string() + if self.is_group { "@g.us" } else { "@s.whatsapp.net" }
    }
}
