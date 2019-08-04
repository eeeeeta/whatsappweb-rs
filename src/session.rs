//! Session management types.

/// Stores persistent session data, used to login without scanning the QR code again.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct PersistentSession {
    pub client_token: String,
    pub server_token: String,
    pub client_id: [u8; 8],
    pub enc: [u8; 32],
    pub mac: [u8; 32]
}

pub(crate) enum SessionState {
    PendingNew {
        private_key: Option<ring::agreement::EphemeralPrivateKey>,
        public_key: Vec<u8>,
        client_id: [u8; 8]
    },
    PendingPersistent {
        persistent_session: PersistentSession
    },
    Established {
        persistent_session: PersistentSession
    }
}
impl SessionState {
    pub(crate) fn pending_new() -> Self {
        use ring::rand::{SecureRandom, SystemRandom};
        use crate::crypto;

        let mut client_id = [0u8; 8];
        SystemRandom::new().fill(&mut client_id).unwrap();

        let (private_key, public_key) = crypto::generate_keypair();
        SessionState::PendingNew {
            private_key: Some(private_key),
            public_key,
            client_id,
        }
    }
    pub(crate) fn pending_persistent(sess: PersistentSession) -> Self {
        SessionState::PendingPersistent {
            persistent_session: sess
        }
    }
}
